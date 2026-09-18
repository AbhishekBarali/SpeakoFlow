import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  ChevronUp,
  Mic,
  Pause,
  Play,
  Square,
  X,
} from "lucide-react";
import {
  acceptCallOffer,
  dismissCallOffer,
  fitMeetingPill,
  getMeetingSpeakers,
  setMeetingPaused,
  startMeeting,
  stopMeeting,
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
import { TONE_HEX } from "./tones";
import { useMeetingPill } from "./useMeetingPill";
import { MeetingAsk } from "./MeetingAsk";
import "./MeetingPill.css";

/** Bars in each row of the level meter. */
const METER_BARS = 22;

/** Below this a stream counts as silent and its row greys out. */
const SILENT_LEVEL = 0.02;

/**
 * The floating meeting pill.
 *
 * Collapsed it answers one question continuously: *is this still capturing, and
 * is it hearing both sides?* Expanded it shows the live transcript and takes a
 * question about it.
 *
 * Two things about this component are constraints rather than choices:
 *
 * - **The measured node must not scroll.** The window's height is whatever the
 *   outer element reports, so a scroll container above `.pill-transcript` would
 *   feed the measurement back into itself and grow the window until it filled the
 *   display. The transcript's `max-height` in CSS is what bounds it.
 * - **The ask input is blurred before collapsing.** On Windows the collapsed pill
 *   is unfocusable so it can never take the caret from the app the user is
 *   working in, and the platform will not remove focusability from a window that
 *   currently holds focus.
 */
const MeetingPill: React.FC = () => {
  const { t } = useTranslation();
  const {
    state,
    recording,
    paused,
    elapsedMs,
    levels,
    items,
    expanded,
    setExpanded,
    offer,
    clearOffer,
  } = useMeetingPill();

  const rootRef = useRef<HTMLDivElement>(null);
  const askRef = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [speakers, setSpeakers] = useState<MeetingSpeaker[]>([]);

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
  }, [expanded]);

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
    const next = !paused;
    void setMeetingPaused(next).catch(() => {});
  };

  const stop = () => {
    setBusy(true);
    // No `finally`: a successful stop hides this window, so clearing the flag
    // afterwards would only matter on the failure path, and on that path the
    // buttons must come back.
    void stopMeeting().catch(() => setBusy(false));
  };

  const collapse = () => {
    // Before the mode change, not after: `set_focusable(false)` is not honoured
    // for a window that holds focus, and a pill left focusable steals the caret.
    askRef.current?.blur();
    setExpanded(false);
  };

  if (!recording) {
    // An offer to record a call the detector noticed. Shown in the same window the
    // recording indicator uses, so accepting is visually continuous.
    if (offer) {
      return (
        <div ref={rootRef} className="pill-shell">
          <OfferCard app={offer.app} onDone={clearOffer} />
        </div>
      );
    }
    // Otherwise this is only the gap between the window appearing and the first
    // state arriving. Render nothing rather than an idle pill that would flash on
    // every meeting.
    return <div ref={rootRef} className="pill-shell" />;
  }

  const clock = formatClock(elapsedMs);
  const meter = (
    <LevelMeter mic={levels.mic} system={levels.system} paused={paused} />
  );

  const pauseButton = (
    <button
      type="button"
      className="pill-action"
      onClick={togglePause}
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

  const stopButton = (
    <button
      type="button"
      className="pill-action"
      data-variant="stop"
      onClick={stop}
      disabled={busy}
      title={t("meetings.recorder.stop")}
      aria-label={t("meetings.recorder.stop")}
    >
      <Square size={11} />
    </button>
  );

  if (!expanded) {
    return (
      <div ref={rootRef} className="pill-shell">
        <div
          className="pill-bar"
          role="button"
          tabIndex={0}
          onClick={() => setExpanded(true)}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              setExpanded(true);
            }
          }}
          aria-label={t("meetings.pill.expand")}
        >
          <span
            className="pill-dot"
            // Amber, not red, when the far side is not being captured: the
            // recording is running but it is only hearing one person, and that is
            // worth noticing before the call ends rather than after.
            data-live={paused ? "false" : String(state.system_audio)}
            aria-hidden="true"
          />
          <span className="pill-clock">{clock}</span>
          {meter}
          {/* Stop propagation so pressing a control does not also expand. */}
          <span
            className="pill-controls"
            onClick={(event) => event.stopPropagation()}
            style={{ display: "flex", gap: 6, flex: "none" }}
          >
            {pauseButton}
            {stopButton}
          </span>
          <ChevronUp size={13} style={{ flex: "none", opacity: 0.5 }} />
        </div>
      </div>
    );
  }

  return (
    <div ref={rootRef} className="pill-shell">
      <div className="pill-card">
        <div className="pill-head">
          <span
            className="pill-dot"
            data-live={paused ? "false" : String(state.system_audio)}
            aria-hidden="true"
          />
          <span className="pill-clock">{clock}</span>
          <span className="pill-head-meta">
            <span className="pill-head-label">
              {paused
                ? t("meetings.recorder.paused")
                : t("meetings.recorder.recording")}
            </span>
          </span>
          {pauseButton}
          {stopButton}
          <button
            type="button"
            className="pill-action"
            onClick={collapse}
            title={t("meetings.pill.collapse")}
            aria-label={t("meetings.pill.collapse")}
          >
            <ChevronDown size={13} />
          </button>
        </div>

        {!state.system_audio && (
          <p className="pill-notice">{t("meetings.pill.micOnly")}</p>
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
    <div className="pill-bar" data-offer="true">
      <span className="pill-dot" data-live="true" aria-hidden="true" />
      <span className="pill-offer-text">
        {app
          ? t("meetings.offer.detectedApp", { app: friendlyAppName(app) })
          : t("meetings.offer.detected")}
      </span>
      <button
        type="button"
        className="pill-action"
        data-variant="accept"
        onClick={accept}
        disabled={busy}
        title={t("meetings.offer.record")}
        aria-label={t("meetings.offer.record")}
      >
        <Mic size={12} />
      </button>
      <button
        type="button"
        className="pill-action"
        onClick={dismiss}
        disabled={busy}
        title={t("meetings.offer.dismiss")}
        aria-label={t("meetings.offer.dismiss")}
      >
        <X size={13} />
      </button>
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
    <span className="pill-turn-who" style={{ color }}>
      <span className="pill-turn-swatch" aria-hidden="true" />
      {name}
    </span>
    <p className="pill-turn-text">{turn.text}</p>
  </div>
);

interface LevelMeterProps {
  mic: number;
  system: number;
  paused: boolean;
}

/**
 * Two rows of bars: the user above, everyone else below.
 *
 * Split rather than combined because *which* row is moving is the diagnostic. A
 * flat bottom row means the other participants are not being captured, which is
 * this feature's most common failure and is otherwise completely silent until the
 * transcript comes back half empty.
 */
const LevelMeter: React.FC<LevelMeterProps> = ({ mic, system, paused }) => (
  <span className="pill-meter" aria-hidden="true">
    <MeterRow stream="mic" level={paused ? 0 : mic} />
    <MeterRow stream="system" level={paused ? 0 : system} />
  </span>
);

const MeterRow: React.FC<{ stream: "mic" | "system"; level: number }> = ({
  stream,
  level,
}) => (
  <span
    className="pill-meter-row"
    data-stream={stream}
    data-silent={String(level < SILENT_LEVEL)}
  >
    {Array.from({ length: METER_BARS }, (_, index) => (
      <span
        key={index}
        className="pill-meter-bar"
        style={barStyle(level, index)}
      />
    ))}
  </span>
);

/**
 * Height and opacity for one bar.
 *
 * The envelope is a raised cosine across the row, so a level reads as a shape
 * centred in the meter rather than as a flat block that is either on or off —
 * which is what makes a quiet voice visibly different from silence instead of
 * both rendering as "nothing".
 */
const barStyle = (level: number, index: number): React.CSSProperties => {
  const centred = (index - (METER_BARS - 1) / 2) / ((METER_BARS - 1) / 2);
  const envelope = 0.35 + 0.65 * Math.cos((centred * Math.PI) / 2) ** 2;
  const scale = Math.max(0.08, Math.min(1, level * envelope));
  return {
    transform: `scaleY(${scale})`,
    height: "100%",
    opacity: level < SILENT_LEVEL ? 0.3 : 0.55 + 0.45 * scale,
  };
};

export default MeetingPill;
