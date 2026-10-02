import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Mic, MicOff, Minimize2, Pause, Play, Square } from "lucide-react";
import {
  acceptCallOffer,
  dismissCallOffer,
  fitMeetingPill,
  getMeetingSpeakers,
  setMeetingPaused,
  startMeeting,
  stopMeeting,
  type MeetingLevels,
  type MeetingSpeaker,
} from "@/components/settings/meetings/api";
import {
  formatClock,
  groupIntoTurns,
  otherSpeakerOrder,
  speakerNameResolver,
  speakerTone,
  type SpeakerTurn,
} from "@/components/settings/meetings/speakers";
import AudioWaveform from "@/components/shared/AudioWaveform";
import { TONE_HEX } from "./tones";
import {
  useMeetingClock,
  useMeetingLevel,
  useMeetingPill,
  type ClockAnchor,
  type LevelStore,
} from "./useMeetingPill";
import { MeetingAsk } from "./MeetingAsk";
import { useSafeWindowDrag } from "@/lib/useSafeWindowDrag";
import { preventBrowserContextMenu } from "@/lib/contextMenu";
import "./MeetingPill.css";

/** Below this a stream counts as silent. */
const SILENT_LEVEL = 0.02;

/** Pointer travel, in screen pixels, past which a press was a drag rather than
 *  a click. Matches `useSafeWindowDrag`'s threshold. */
const CLICK_SLOP_PX = 4;

/** Which corner the card grows out of, so the zoom starts at the pill. */
type Origin = { right: boolean; bottom: boolean };

/**
 * Where the pill sits on its display, read at the moment it is clicked.
 *
 * Rust keeps the window's corner fixed when it grows (`pill::corner_for`), and
 * this answers the same question from inside the webview so the zoom animates
 * out of the pill rather than out of the card's centre. `availLeft`/`availTop`
 * are Chromium extensions that place a secondary display correctly; without
 * them the primary display's origin is assumed.
 */
const originNow = (): Origin => {
  const display = screen as Screen & { availLeft?: number; availTop?: number };
  const left = display.availLeft ?? 0;
  const top = display.availTop ?? 0;
  return {
    right:
      window.screenX + window.innerWidth / 2 >= left + screen.availWidth / 2,
    bottom:
      window.screenY + window.innerHeight / 2 >= top + screen.availHeight / 2,
  };
};

/**
 * The floating meeting indicator.
 *
 * Collapsed it is the dictation overlay's compact pill in a meeting's colours: a
 * recording dot, the clock, and a small waveform — nothing to read, nothing to
 * decide. It replaced a 276px strip with a two-row level meter and three
 * permanent buttons, which put more on screen during a call than the call app
 * itself did.
 *
 * - **Clicking the pill opens it.** The card zooms out of the pill's corner, so
 *   there is no separate expand button to aim for; the pill *is* the way in.
 * - **Pause and stop only appear under the pointer**, in place of the waveform
 *   at the pill's trailing edge. The width does not change, so the window never
 *   resizes under a hovering cursor and nothing flickers.
 * - **The pill is still the drag handle.** A press that travels is a move, one
 *   that does not is a click — the same threshold `useSafeWindowDrag` uses.
 *
 * Two things about this component are constraints rather than choices:
 *
 * - **The measured node must not scroll.** The window's height is whatever the
 *   outer element reports, so a scroll container above `.pill-transcript` would
 *   feed the measurement back into itself and grow the window until it filled
 *   the display. The transcript's `max-height` in CSS is what bounds it.
 * - **The ask input is blurred before collapsing.** On Windows the collapsed
 *   pill is unfocusable so it can never take the caret from the app the user is
 *   working in, and the platform will not remove focusability from a window
 *   that currently holds focus.
 */
const MeetingPill: React.FC = () => {
  const { t } = useTranslation();
  useSafeWindowDrag();
  const {
    state,
    recording,
    paused,
    clockAnchor,
    levels,
    items,
    expanded,
    setExpanded,
    offer,
    clearOffer,
  } = useMeetingPill();

  const rootRef = useRef<HTMLDivElement>(null);
  const askRef = useRef<HTMLInputElement>(null);
  const pressRef = useRef<{ x: number; y: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const [speakers, setSpeakers] = useState<MeetingSpeaker[]>([]);
  const [origin, setOrigin] = useState<Origin>({ right: true, bottom: true });

  const meetingId = state.meeting_id;

  /* ── window sizing ── */

  // The window takes the height this content measures. Reported on every
  // observer callback; Rust ignores a height it already has, which is what stops
  // a resize from triggering the measurement that caused it.
  useEffect(() => {
    const node = rootRef.current;
    if (!node) return;
    const report = () => {
      const height = Math.ceil(node.getBoundingClientRect().height);
      if (height > 0) void fitMeetingPill(height).catch(() => {});
    };
    report();
    const observer = new ResizeObserver(report);
    observer.observe(node);
    return () => observer.disconnect();
  }, [expanded, recording, offer]);

  /* ── speakers ── */

  // Only needed by the expanded card, and only worth fetching once per meeting:
  // during a recording the only keys that exist are the two channel-derived ones,
  // and diarization runs after the call.
  useEffect(() => {
    if (!expanded || meetingId === null) return;
    void getMeetingSpeakers(meetingId)
      .then(setSpeakers)
      .catch(() => {});
  }, [expanded, meetingId]);

  /* ── transcript ── */

  const turns = useMemo(() => groupIntoTurns(items), [items]);
  const others = useMemo(
    () => otherSpeakerOrder(turns, speakers),
    [turns, speakers],
  );
  const nameFor = useMemo(
    () =>
      speakerNameResolver(speakers, others, {
        me: t("meetings.speakers.me"),
        others: t("meetings.speakers.others"),
        numbered: (number) => t("meetings.speakers.numbered", { number }),
      }),
    [speakers, others, t],
  );

  const scrollRef = useRef<HTMLDivElement>(null);
  const turnCount = turns.length;
  useEffect(() => {
    const node = scrollRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [turnCount, expanded]);

  /* ── actions ── */

  const togglePause = () => {
    void setMeetingPaused(!paused).catch(() => {});
  };

  const stop = () => {
    setBusy(true);
    // No `finally`: a successful stop hides this window, so clearing the flag
    // afterwards would only matter on the failure path, and on that path the
    // buttons must come back.
    void stopMeeting().catch(() => setBusy(false));
  };

  const open = () => {
    setOrigin(originNow());
    setExpanded(true);
  };

  const collapse = () => {
    // Before the mode change, not after: `set_focusable(false)` is not honoured
    // for a window that holds focus, and a pill left focusable steals the caret.
    askRef.current?.blur();
    setExpanded(false);
  };

  // Escape folds the card back into the pill. The ask field consumes its own
  // Escape first (clearing a half-typed question), so this never eats one.
  useEffect(() => {
    if (!expanded) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) collapse();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // `collapse` only touches a ref and a stable setter, so it is not a dep.
  }, [expanded]);

  if (!recording) {
    // An offer to record a call the detector noticed. Shown in the same window the
    // recording indicator uses, so accepting is visually continuous.
    if (offer) {
      return (
        <div
          ref={rootRef}
          className="pill-shell"
          onContextMenu={preventBrowserContextMenu}
        >
          <OfferCard app={offer.app} onDone={clearOffer} />
        </div>
      );
    }
    // Otherwise this is only the gap between the window appearing and the first
    // state arriving. Render nothing rather than an idle pill that would flash on
    // every meeting.
    return <div ref={rootRef} className="pill-shell" />;
  }

  // Amber, not red, when the far side is not being captured: the recording is
  // running but it is only hearing one person, and that is worth noticing
  // before the call ends rather than after.
  const dotState = paused ? "paused" : state.system_audio ? "live" : "mic-only";
  const statusLabel = paused
    ? t("meetings.recorder.paused")
    : t("meetings.recorder.recording");

  const pauseButton = (
    <button
      type="button"
      className="pill-action"
      onClick={(event) => {
        event.stopPropagation();
        togglePause();
      }}
      disabled={busy}
      title={
        paused ? t("meetings.recorder.resume") : t("meetings.recorder.pause")
      }
      aria-label={
        paused ? t("meetings.recorder.resume") : t("meetings.recorder.pause")
      }
    >
      {paused ? <Play size={12} /> : <Pause size={12} />}
    </button>
  );

  if (!expanded) {
    return (
      <div
        ref={rootRef}
        className="pill-shell"
        onContextMenu={preventBrowserContextMenu}
      >
        {/* The clock owns its 500 ms tick, so the collapsed pill (whose
            label carries the time) re-renders on it and nothing else does. */}
        <MeetingClock anchor={clockAnchor} paused={paused}>
          {(clock) => (
            <div
              className="mpill"
              data-paused={String(paused)}
              data-tauri-drag-region
              // A group, not a button: `role="button"` is on the never-draggable
              // list in `useSafeWindowDrag`, and the pill has to stay the handle.
              role="group"
              title={t("meetings.pill.expand")}
              aria-label={`${statusLabel} ${clock}. ${t("meetings.pill.expand")}`}
              onPointerDown={(event) => {
                pressRef.current = { x: event.screenX, y: event.screenY };
              }}
              onClick={(event) => {
                const press = pressRef.current;
                pressRef.current = null;
                // A press that travelled moved the window; it was not a click.
                if (
                  press &&
                  (Math.abs(event.screenX - press.x) > CLICK_SLOP_PX ||
                    Math.abs(event.screenY - press.y) > CLICK_SLOP_PX)
                )
                  return;
                open();
              }}
            >
              <span
                className="pill-dot"
                data-state={dotState}
                aria-hidden="true"
              />
              <span className="mpill-clock">{clock}</span>
              <span className="mpill-end">
                <span className="mpill-wave" aria-hidden="true">
                  <PillWave levels={levels} paused={paused} />
                </span>
                <span className="mpill-controls">
                  {pauseButton}
                  <button
                    type="button"
                    className="pill-action"
                    data-variant="stop"
                    onClick={(event) => {
                      event.stopPropagation();
                      stop();
                    }}
                    disabled={busy}
                    title={t("meetings.pill.stop")}
                    aria-label={t("meetings.pill.stop")}
                  >
                    <Square size={10} fill="currentColor" />
                  </button>
                </span>
              </span>
            </div>
          )}
        </MeetingClock>
      </div>
    );
  }

  return (
    <div
      ref={rootRef}
      className="pill-shell"
      onContextMenu={preventBrowserContextMenu}
    >
      <div
        className="pill-card"
        data-origin-x={origin.right ? "right" : "left"}
        data-origin-y={origin.bottom ? "bottom" : "top"}
      >
        <div className="pill-head" data-tauri-drag-region>
          <span className="pill-head-status" role="status">
            <span
              className="pill-dot"
              data-state={dotState}
              aria-hidden="true"
            />
            <span className="pill-clock" data-paused={String(paused)}>
              <MeetingClock anchor={clockAnchor} paused={paused}>
                {(clock) => clock}
              </MeetingClock>
            </span>
            {/* The breathing red dot already says "recording"; a paused
                recording is the state that needs saying in words. */}
            {paused ? (
              <span className="pill-head-label">{statusLabel}</span>
            ) : (
              <span className="sr-only">{statusLabel}</span>
            )}
          </span>
          <span className="pill-head-sides" aria-hidden="true">
            <SideChip
              label={t("meetings.speakers.me")}
              levels={levels}
              side="mic"
              paused={paused}
              captured
            />
            <SideChip
              label={t("meetings.speakers.others")}
              levels={levels}
              side="system"
              paused={paused}
              captured={state.system_audio}
            />
          </span>
          <span className="pill-head-spacer" />
          <span className="pill-head-actions">
            {pauseButton}
            <button
              type="button"
              className="pill-btn"
              data-variant="stop"
              onClick={stop}
              disabled={busy}
              title={t("meetings.pill.stop")}
            >
              <Square size={10} aria-hidden="true" />
              {t("meetings.recorder.stop")}
            </button>
            <button
              type="button"
              className="pill-action"
              data-variant="ghost"
              onClick={collapse}
              title={t("meetings.pill.collapse")}
              aria-label={t("meetings.pill.collapse")}
            >
              <Minimize2 size={14} />
            </button>
          </span>
        </div>

        {!state.system_audio && (
          <p className="pill-notice">
            <MicOff size={13} aria-hidden="true" />
            <span className="pill-notice-text">
              {t("meetings.pill.micOnly")}
            </span>
          </p>
        )}

        <div ref={scrollRef} className="pill-transcript">
          {turns.length === 0 ? (
            <p className="pill-empty">{t("meetings.pill.listening")}</p>
          ) : (
            turns.map((turn) => (
              <TurnRow
                key={turn.key}
                turn={turn}
                name={nameFor(turn.speakerKey)}
                color={TONE_HEX[speakerTone(turn.speakerKey, others)]}
              />
            ))
          )}
        </div>

        <MeetingAsk inputRef={askRef} meetingId={meetingId} />
      </div>
    </div>
  );
};

/**
 * The recording clock, as text, for whatever `children` draws it into.
 *
 * It ticks on its own (`useMeetingClock`), so the pill root, and with it the
 * expanded card's transcript list, does not re-render twice a second.
 */
const MeetingClock: React.FC<{
  anchor: ClockAnchor;
  paused: boolean;
  children: (clock: string) => React.ReactNode;
}> = ({ anchor, paused, children }) => (
  <>{children(formatClock(useMeetingClock(anchor, paused)))}</>
);

/**
 * The collapsed pill's waveform.
 *
 * One combined activity signal, because the pill only answers "is it hearing
 * anyone". Which side is audible is the card's job, and a far side that is not
 * being captured at all turns the dot amber here as well. Memoised on the
 * value so a clock tick does not look like fresh audio to the waveform. Reads
 * the level store itself, so a level event renders only this.
 */
const PillWave: React.FC<{ levels: LevelStore; paused: boolean }> = ({
  levels,
  paused,
}) => {
  const loudest = useMeetingLevel(levels, (current) =>
    paused ? 0 : Math.max(current.mic, current.system),
  );
  const waveLevels = useMemo(() => (loudest > 0 ? [loudest] : []), [loudest]);
  return (
    <AudioWaveform
      barCount={9}
      levels={waveLevels}
      size="sm"
      active={!paused}
      mode="reactive"
    />
  );
};

/**
 * "You" / "Others" with a light that is on while that side is audible.
 *
 * This is what the old two-row meter was for — a far side that is not being
 * captured is the feature's most common failure — reduced to the one bit it
 * actually carried. A side that is not captured at all is struck through
 * rather than merely dark, because "quiet" and "not recorded" must not look
 * alike. It reads its own side's level, so a level event renders only this.
 */
const SideChip: React.FC<{
  label: string;
  levels: LevelStore;
  side: keyof MeetingLevels;
  paused: boolean;
  captured: boolean;
}> = ({ label, levels, side, paused, captured }) => {
  const level = useMeetingLevel(levels, (current) =>
    paused ? 0 : current[side],
  );
  return (
    <span
      className="pill-side"
      data-captured={String(captured)}
      data-active={String(captured && level >= SILENT_LEVEL)}
    >
      <span className="pill-side-light" />
      {label}
    </span>
  );
};

interface OfferCardProps {
  /** The capturing app's name, when it could be read. */
  app: string | null;
  onDone: () => void;
}

/**
 * "Looks like you're on a call. Record it?"
 *
 * The detector that raises this never records anything — it observes that some other
 * process is holding the microphone open, which is what a call is mechanically, and
 * asks. Accepting is what starts a recording, and that is a hard rule: software that
 * began capturing a private conversation because it inferred one was happening would
 * be unacceptable however accurate the inference was.
 *
 * ## Why it must not look like the recorder
 *
 * It did, and that was the single most alarming thing in this feature: the card
 * reused the live recorder's breathing red dot — the universal "capturing now"
 * signal — on a card that arrives unprompted six to nine seconds into a call. A
 * user who glanced at it read "SpeakoFlow started recording my call by itself".
 * The indicator is a hollow, static ring in the accent colour, which is not a
 * state the live recorder can ever be in, and the card says "Not recording" in as
 * many words. Redundant next to "Record it?", and kept anyway: this is the one
 * card in the app where a misreading costs the user's trust rather than a click.
 *
 * The title is composed here rather than in Rust so its default is localised, which
 * is the same reason `MeetingsSection` composes it.
 */
const OfferCard: React.FC<OfferCardProps> = ({ app, onDone }) => {
  const { t, i18n } = useTranslation();
  const [busy, setBusy] = useState(false);

  const accept = () => {
    setBusy(true);
    const when = new Intl.DateTimeFormat(i18n.language, {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date());
    // Told first, so a slow device open cannot let the watcher's next tick raise a
    // second offer for the call already being accepted.
    void acceptCallOffer()
      .then(() => startMeeting(t("meetings.newTitle", { when })))
      // The state event that follows clears the offer, so nothing to do on success.
      .catch(() => setBusy(false));
  };

  const dismiss = () => {
    onDone();
    void dismissCallOffer().catch(() => {});
  };

  return (
    <div className="pill-offer" data-tauri-drag-region>
      <div className="pill-offer-top">
        <span className="pill-offer-ring" aria-hidden="true" />
        <span className="pill-offer-body">
          <span className="pill-offer-text">
            {app
              ? t("meetings.offer.detectedApp", { app: friendlyAppName(app) })
              : t("meetings.offer.detected")}
          </span>
          <span className="pill-offer-state">{t("meetings.offer.idle")}</span>
        </span>
      </div>
      {/* Both answers in words. Two bare icons — a microphone and an X — made
          the one decision this card asks for a guess. */}
      <div className="pill-offer-actions">
        <button
          type="button"
          className="pill-btn"
          data-variant="primary"
          onClick={accept}
          disabled={busy}
        >
          <Mic size={13} aria-hidden="true" />
          {t("meetings.offer.record")}
        </button>
        <button
          type="button"
          className="pill-btn"
          data-variant="ghost"
          onClick={dismiss}
          disabled={busy}
        >
          {t("meetings.offer.dismiss")}
        </button>
      </div>
    </div>
  );
};

/**
 * `Zoom.exe` reads as a file path in a sentence; `Zoom` reads as an app.
 *
 * Only cosmetic — nothing branches on the name, and the whole offer works when it is
 * absent — so this stays a trim rather than a lookup table of known executables,
 * which would be wrong the week after it was written.
 */
const friendlyAppName = (name: string): string =>
  name.replace(/\.(exe|app)$/i, "");

interface TurnRowProps {
  turn: SpeakerTurn;
  name: string;
  color: string;
}

const TurnRow: React.FC<TurnRowProps> = ({ turn, name, color }) => (
  <div className="pill-turn" data-uncertain={String(turn.lowConfidence)}>
    <span className="pill-turn-who">
      <span
        className="pill-turn-swatch"
        style={{ background: color }}
        aria-hidden="true"
      />
      {name}
      <span className="pill-turn-time">{formatClock(turn.startMs)}</span>
    </span>
    <p className="pill-turn-text">{turn.text}</p>
  </div>
);

export default MeetingPill;
