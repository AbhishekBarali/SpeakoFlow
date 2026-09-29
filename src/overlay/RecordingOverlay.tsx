import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import AudioWaveform from "../components/shared/AudioWaveform";
import CompletionMark from "./CompletionMark";
import { voiceEnergy } from "../components/shared/waveformSignal";
import i18n, { syncLanguageFromSettings } from "@/i18n";
import { preventBrowserContextMenu } from "@/lib/contextMenu";
import { getLanguageDirection } from "@/lib/utils/rtl";
import "./RecordingOverlay.css";
import "@/lib/windowActivity.css";

type OverlayState =
  | "recording"
  | "transcribing"
  | "processing"
  | "generating"
  | "vision"
  | "notice";
type StreamTextPayload = { committed: string; tentative: string };
/** One run of committed text, rendered as its own element.
 *
 * Committed text only grows, so each extension becomes a new chunk and a
 * chunk's text never changes once it is on screen. That is what keeps a
 * selection alive while words are still arriving: React updates a changed
 * string by rewriting its text node, which collapses any selection inside it,
 * so a transcript drawn as one growing string lost the selection on every
 * word. `arriving` marks the newest chunk, the only text that animates in. */
type Chunk = { key: number; text: string; arriving: boolean };
type Transcript = {
  committed: string;
  chunks: Chunk[];
  tentative: string;
  /** Bumped when the final text replaces a different live one. It keys the
   * paragraph, so the finished version fades in instead of jumping in place. */
  reveal: number;
};
type ShowOverlayPayload = {
  state: OverlayState;
  streamingWindow: boolean;
  /** Whether the card takes the pointer in this state. The backend decides per
   * platform (`live_card_takes_pointer` in overlay.rs): only where the window
   * cannot take keyboard focus, so a click can never steal the paste target. */
  interactive?: boolean;
  notice?: string;
};
type WaveShape = { bars: number; pitch?: number; barWidth?: number };

const EMPTY_LEVELS: number[] = [];
const EMPTY_TRANSCRIPT: Transcript = {
  committed: "",
  chunks: [],
  tentative: "",
  reveal: 0,
};

/** The card's indicator slot is 23px: five 3px bars on a 5px pitch, the same
 * footprint as the check that replaces them, so a change of state moves
 * nothing. */
const CARD_WAVE: WaveShape = { bars: 5, pitch: 5, barWidth: 3 };
const PILL_WAVE: WaveShape = { bars: 14 };
const LABELED_PILL_WAVE: WaveShape = { bars: 9 };

/** Coalesce transcript updates onto the display's own clock. The engines emit
 * on their decoding cadence (a burst per audio chunk, and a partial rewrite can
 * arrive between two commits), so applying each event as it lands paints text
 * several times per frame at irregular intervals — visible as stutter. One
 * update per frame is both smoother and less work. Falls back to applying
 * immediately where there is no frame clock, which keeps tests synchronous. */
const onNextFrame =
  typeof requestAnimationFrame === "function"
    ? requestAnimationFrame
    : (run: FrameRequestCallback) => {
        run(0);
        return 0;
      };

/** Distance from the end, in px, that still counts as "at the newest words".
 * Scroll positions are fractional on scaled displays. */
const AT_END_PX = 2;

/** A drag longer than this stops holding the live text still, in case its
 * release was never reported. Selecting a phrase takes a second or two. */
const HOLD_LIMIT_MS = 10_000;

/** States that carry a written label instead of leaning on the waveform. The
 * backend widens the window for exactly these (see OVERLAY_LABEL_WIDTH). */
const LABELED: readonly OverlayState[] = ["generating", "vision", "notice"];

/** How long a copy confirms before the header returns to normal. */
const COPIED_FEEDBACK_MS = 1600;

/** A hop that faded the overlay out and never heard back fades it in anyway.
 * The backend's own fade is 60ms (`HOP_FADE_OUT`), so this only fires when its
 * "in" was lost. */
const HOP_TIMEOUT_MS = 400;

/** Fold one engine update into what is on screen. */
function nextTranscript(
  previous: Transcript,
  update: StreamTextPayload,
): Transcript {
  const { committed, tentative } = update;
  if (committed === previous.committed) {
    // Tentative-only update. The chunks are untouched, so the newest chunk's
    // arrival keeps animating instead of snapping to full opacity.
    return previous.tentative === tentative
      ? previous
      : { ...previous, tentative };
  }
  if (previous.committed && committed.startsWith(previous.committed)) {
    const chunks = previous.chunks.map((chunk) =>
      chunk.arriving ? { ...chunk, arriving: false } : chunk,
    );
    chunks.push({
      key: previous.committed.length,
      text: committed.slice(previous.committed.length),
      arriving: true,
    });
    return { ...previous, committed, chunks, tentative };
  }
  // The first words of a recording arrive like any other. A rewrite of words
  // already on screen is adopted whole and silently: animating a line the
  // decoder just revised reads as a glitch.
  return {
    ...previous,
    committed,
    chunks: committed
      ? [{ key: 0, text: committed, arriving: !previous.committed }]
      : [],
    tentative,
  };
}

const transcriptOf = (text: string, reveal: number): Transcript => ({
  committed: text,
  chunks: text ? [{ key: 0, text, arriving: false }] : [],
  tentative: "",
  reveal,
});

const selectedText = () =>
  (typeof window === "undefined"
    ? ""
    : (window.getSelection?.()?.toString() ?? "")
  ).trim();

/** How far the body can scroll. `|| 0` keeps an unmeasured node from
 * producing NaN. */
const maxScroll = (body: HTMLElement) =>
  Math.max(0, body.scrollHeight - body.clientHeight) || 0;

const scrollBodyTo = (
  body: HTMLElement,
  top: number,
  behavior: ScrollBehavior,
) => {
  if (typeof body.scrollTo === "function") body.scrollTo({ top, behavior });
  else body.scrollTop = top;
};

/** Whether a press landed on the body's scrollbar rather than its text.
 * `clientLeft` includes a scrollbar drawn on the left, so this holds in RTL. */
const onScrollbar = (body: HTMLElement, clientX: number) => {
  if (typeof body.getBoundingClientRect !== "function") return false;
  const x = clientX - body.getBoundingClientRect().left;
  return x < body.clientLeft || x >= body.clientLeft + body.clientWidth;
};

const RecordingOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [isVisible, setIsVisible] = useState(false);
  const [state, setState] = useState<OverlayState>("recording");
  const [notice, setNotice] = useState<string | null>(null);
  const [locked, setLocked] = useState(false);
  const [levels, setLevels] = useState<number[]>(EMPTY_LEVELS);
  const [micLive, setMicLive] = useState(false);
  const [transcript, setTranscript] = useState<Transcript>(EMPTY_TRANSCRIPT);
  const [streamingWindow, setStreamingWindow] = useState(false);
  const [interactive, setInteractive] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  const [completed, setCompleted] = useState(false);
  const [fading, setFading] = useState(false);
  /** Moving to another display: faded out while the window moves under it. */
  const [hopping, setHopping] = useState(false);
  /** Whether there is text past the top or bottom edge, which is what fades
   * that edge instead of cutting a line in half. */
  const [edges, setEdges] = useState({ above: false, below: false });
  const completionEpoch = useRef<number | null>(null);
  const copyGeneration = useRef(0);
  const cardRef = useRef<HTMLDivElement>(null);
  const cardBodyRef = useRef<HTMLDivElement>(null);
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hopTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingText = useRef<StreamTextPayload | null>(null);
  const pendingReplace = useRef<Transcript | null>(null);
  const textFrame = useRef(false);
  /** What is on screen, so the next update can tell new words from ones the
   * user has already read. A ref, not derived from state: React may re-run an
   * updater function, and `nextTranscript` is not idempotent across calls. */
  const shown = useRef<Transcript>(EMPTY_TRANSCRIPT);
  const revealCount = useRef(0);
  /** The pointer went down on the transcript and has not come up yet. */
  const pressed = useRef(false);
  /** Updates wait while a press is held, because text growing and scrolling
   * under the pointer turns a selection into a fight. */
  const holding = useRef(false);
  const holdTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** Pinned to the newest words. Only the user moving back through the text
   * turns it off; reaching the end again turns it back on. */
  const follow = useRef(true);
  /** Last scroll position seen, to tell which way a scroll went. The card only
   * ever scrolls itself down, so moving up is always the user. */
  const lastTop = useRef(0);
  /** Where the last follow scroll was sent, so repeated updates on one line do
   * not restart a smooth scroll that is already heading there. */
  const requestedTop = useRef<number | null>(null);
  /** The next follow scroll replaces the text wholesale and should not glide. */
  const jumpNext = useRef(false);

  // These helpers only touch refs and state setters, so the listeners below
  // can safely keep the copies from the first render.
  const present = (next: Transcript, jump: boolean) => {
    if (next === shown.current) return;
    shown.current = next;
    if (jump) jumpNext.current = true;
    setTranscript(next);
  };
  const applyPending = () => {
    const latest = pendingText.current;
    if (!latest || holding.current) return;
    pendingText.current = null;
    present(nextTranscript(shown.current, latest), false);
  };
  const queueText = (next: StreamTextPayload) => {
    pendingText.current = next;
    if (textFrame.current) return;
    textFrame.current = true;
    onNextFrame(() => {
      textFrame.current = false;
      applyPending();
    });
  };
  /** Replace the transcript wholesale, dropping any engine update still
   * waiting for a frame so it cannot paint stale words over the replacement.
   * The final text waits for the end of a drag, so the words being selected
   * are not swapped out from under the pointer; `force` is for a new recording
   * or a hide, which end the drag instead. */
  const replaceText = (next: Transcript, force: boolean) => {
    pendingText.current = null;
    if (holding.current && !force) {
      pendingReplace.current = next;
      return;
    }
    pendingReplace.current = null;
    present(next, true);
  };
  const clearHoldTimer = () => {
    if (holdTimer.current) clearTimeout(holdTimer.current);
    holdTimer.current = null;
  };
  /** Fade out for a move to another display, or back in once it has landed. */
  const setHop = (out: boolean) => {
    if (hopTimer.current) clearTimeout(hopTimer.current);
    hopTimer.current = out
      ? setTimeout(() => {
          hopTimer.current = null;
          setHopping(false);
        }, HOP_TIMEOUT_MS)
      : null;
    setHopping(out);
  };
  /** Resume updates and catch up on whatever arrived while the text was held
   * still. */
  const stopHolding = () => {
    clearHoldTimer();
    if (!holding.current) return;
    holding.current = false;
    const replacement = pendingReplace.current;
    if (replacement) {
      pendingReplace.current = null;
      present(replacement, true);
    } else {
      applyPending();
    }
  };

  const copyText = async (text: string) => {
    if (!text) return;
    const generation = ++copyGeneration.current;
    setCopyFailed(false);
    try {
      // Native clipboard access works without stealing focus from the editor.
      await invoke<void>("copy_overlay_transcript", { text });
      if (generation !== copyGeneration.current) return;
      setCopied(true);
      if (copiedTimer.current) clearTimeout(copiedTimer.current);
      copiedTimer.current = setTimeout(
        () => setCopied(false),
        COPIED_FEEDBACK_MS,
      );
    } catch (error) {
      if (generation !== copyGeneration.current) return;
      setCopyFailed(true);
      console.error("Could not copy the transcript:", error);
    }
  };

  /** End a press. Releasing a selection copies it: the overlay can never hold
   * the keyboard (it would steal the paste target), so Ctrl+C would reach the
   * user's editor instead, and a floating HUD has no context menu. The
   * selection is read before the catch-up re-renders anything. */
  const releasePress = () => {
    if (!pressed.current) return;
    pressed.current = false;
    const selected = selectedText();
    stopHolding();
    if (selected) void copyText(selected);
  };

  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    let cancelled = false;
    let visible = false;
    let recording = false;
    let finished = false;
    const register = (unlisten: UnlistenFn) => {
      if (cancelled) unlisten();
      else unlisteners.push(unlisten);
    };
    const endPress = () => {
      pressed.current = false;
      holding.current = false;
      pendingReplace.current = null;
      clearHoldTimer();
    };
    // Register together so a slow registration cannot leave hide unobserved.
    void Promise.all([
      listen<ShowOverlayPayload>("show-overlay", ({ payload }) => {
        completionEpoch.current = null;
        copyGeneration.current++;
        finished = false;
        setCompleted(false);
        setFading(false);
        // A show places the window itself, so any hop it interrupted is over.
        setHop(false);
        setCopyFailed(false);
        visible = true;
        recording = payload.state === "recording";
        setState(payload.state);
        setNotice(payload.notice ?? null);
        setStreamingWindow(payload.streamingWindow);
        setInteractive(!!payload.interactive);
        if (recording) {
          endPress();
          follow.current = true;
          lastTop.current = 0;
          setLocked(false);
          setMicLive(false);
          setLevels(EMPTY_LEVELS);
          replaceText(EMPTY_TRANSCRIPT, true);
          setCopied(false);
        }
        setIsVisible(true);
        // Never await language I/O before applying an event: a late show could
        // otherwise resurrect an overlay that has already been hidden.
        void syncLanguageFromSettings();
      }).then(register),
      listen("hide-overlay", () => {
        completionEpoch.current = null;
        copyGeneration.current++;
        finished = false;
        endPress();
        setFading(false);
        setHop(false);
        visible = false;
        recording = false;
        setIsVisible(false);
        setInteractive(false);
        setLocked(false);
        setMicLive(false);
        setLevels(EMPTY_LEVELS);
        replaceText(EMPTY_TRANSCRIPT, true);
        setCopied(false);
      }).then(register),
      listen<{ epoch: number; text: string; notice?: string }>(
        "finish-overlay",
        ({ payload }) => {
          if (!visible) return;
          completionEpoch.current = payload.epoch;
          copyGeneration.current++;
          recording = false;
          finished = true;
          setCompleted(true);
          setFading(false);
          setMicLive(false);
          setNotice(payload.notice ?? null);
          const final = payload.text.trim();
          const live =
            `${shown.current.committed}${shown.current.tentative}`.trim();
          const reveal =
            final && final !== live
              ? ++revealCount.current
              : shown.current.reveal;
          replaceText(transcriptOf(payload.text, reveal), false);
          setCopied(false);
          setCopyFailed(false);
        },
      ).then(register),
      listen<number>("fade-overlay", ({ payload }) => {
        if (completionEpoch.current === payload) setFading(true);
      }).then(register),
      listen<number>("restore-overlay", ({ payload }) => {
        if (completionEpoch.current === payload) setFading(false);
      }).then(register),
      listen<boolean>("recording-locked", ({ payload }) => {
        if (recording) setLocked(payload);
      }).then(register),
      listen<"out" | "in">("overlay-hop", ({ payload }) => {
        if (visible) setHop(payload === "out");
      }).then(register),
      listen<number[]>("mic-level", ({ payload }) => {
        if (!recording) return;
        setLevels(voiceEnergy(payload) === 0 ? EMPTY_LEVELS : payload);
        setMicLive(true);
      }).then(register),
      // After completion the card shows the final text; a late live update
      // must not paint the raw transcript back over it.
      listen<StreamTextPayload>("stream-text", ({ payload }) => {
        if (visible && !finished) queueText(payload);
      }).then(register),
    ]).catch((error) => console.error("Overlay listeners failed:", error));
    return () => {
      cancelled = true;
      pendingText.current = null;
      if (hopTimer.current) clearTimeout(hopTimer.current);
      for (const unlisten of unlisteners) unlisten();
    };
  }, []);

  // A press released outside the transcript still ends. Chromium captures the
  // pointer for a selection drag, so the release is reported here even off the
  // window; and a pointer that moves with no button down means the release was
  // lost after all, which must not leave the live text frozen.
  useEffect(() => {
    if (
      typeof window === "undefined" ||
      typeof window.addEventListener !== "function"
    )
      return;
    const release = () => releasePress();
    const move = (event: PointerEvent) => {
      if (pressed.current && (event.buttons & 1) === 0) releasePress();
    };
    window.addEventListener("pointerup", release);
    window.addEventListener("pointercancel", release);
    window.addEventListener("pointermove", move);
    return () => {
      window.removeEventListener("pointerup", release);
      window.removeEventListener("pointercancel", release);
      window.removeEventListener("pointermove", move);
    };
  }, []);

  const measureEdges = (body: HTMLElement) => {
    const above = body.scrollTop > AT_END_PX;
    // While following, the view is at the newest words by definition, and a
    // smooth scroll on its way there must not flash the bottom fade.
    const below =
      !follow.current && maxScroll(body) - body.scrollTop > AT_END_PX;
    setEdges((previous) =>
      previous.above === above && previous.below === below
        ? previous
        : { above, below },
    );
  };

  useLayoutEffect(() => {
    const body = cardBodyRef.current;
    if (!body) return;
    const jump = jumpNext.current;
    jumpNext.current = false;
    if (jump) requestedTop.current = null;
    if (follow.current && !holding.current) {
      const top = maxScroll(body);
      if (jump || top !== requestedTop.current) {
        requestedTop.current = top;
        // A new line glides into view. Replaced text is already where it
        // belongs and fades in, so it jumps rather than sliding past the eye.
        scrollBodyTo(body, top, jump ? "instant" : "smooth");
        if (jump) lastTop.current = body.scrollTop;
      }
    }
    measureEdges(body);
  }, [transcript, streamingWindow]);

  useEffect(
    () => () => {
      if (copiedTimer.current) clearTimeout(copiedTimer.current);
      if (holdTimer.current) clearTimeout(holdTimer.current);
    },
    [],
  );

  useEffect(() => {
    if (isVisible && streamingWindow) {
      void emit(
        "overlay-hover",
        !!cardRef.current?.matches(":hover, :has(:focus-visible)"),
      );
    }
  }, [isVisible, streamingWindow, state, completed]);

  const isRecording = state === "recording" && !completed;
  const hasLiveText = !!(transcript.committed || transcript.tentative);
  const live = isRecording && micLive;
  const labeled = LABELED.includes(state);
  const working = state !== "recording" && state !== "notice";
  // A finished card takes the pointer on every platform; before that, only
  // where the backend could give it one without risking the paste target.
  // Offering a button or a selectable line that the window then passes
  // straight through to the app underneath is worse than offering nothing.
  const canInteract = completed || interactive;

  const busyLabel =
    state === "notice"
      ? t(`overlay.notices.${notice ?? "flowFailed"}`)
      : t(`overlay.${state}`);
  const doneLabel = notice ? t(`overlay.notices.${notice}`) : t("overlay.done");
  // The pill has nowhere to put a second line, so its one label is the invitation
  // to speak. The card keeps the state word in its header and lets the body hold
  // the "Listening…" placeholder, so the two never say the same thing twice.
  const pillLabel = completed
    ? doneLabel
    : isRecording
      ? live
        ? t("overlay.listening")
        : t("overlay.preparing")
      : busyLabel;
  const cardLabel = completed
    ? doneLabel
    : isRecording
      ? live
        ? t("overlay.recording")
        : t("overlay.preparing")
      : busyLabel;
  const ariaLabel =
    isRecording && locked
      ? t("overlay.locked")
      : completed
        ? cardLabel
        : pillLabel;

  const reportHover = (hovered: boolean) => {
    if (!streamingWindow) return;
    const holdingOpen =
      hovered || !!cardRef.current?.matches(":hover, :has(:focus-visible)");
    if (holdingOpen) setFading(false);
    void emit("overlay-hover", holdingOpen).catch((error) =>
      console.error("Overlay hover failed:", error),
    );
  };

  const copyTranscript = () =>
    copyText(`${transcript.committed}${transcript.tentative}`.trim());

  const onBodyScroll = () => {
    const body = cardBodyRef.current;
    if (!body) return;
    const top = body.scrollTop;
    const movedUp = top < lastTop.current - 1;
    lastTop.current = top;
    if (maxScroll(body) - top <= AT_END_PX) {
      // Back at the newest words, by whatever means: follow them again.
      follow.current = true;
    } else if (movedUp) {
      // The user went back through the text. Stop pulling them to the end,
      // and forget the last target so returning there re-pins cleanly.
      follow.current = false;
      requestedTop.current = null;
    }
    measureEdges(body);
  };
  const onBodyPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    const body = cardBodyRef.current;
    if (!canInteract || !hasLiveText || event.button !== 0 || !body) return;
    // Dragging the scrollbar needs no hold: new words land below the thumb.
    if (onScrollbar(body, event.clientX)) return;
    pressed.current = true;
    holding.current = true;
    clearHoldTimer();
    holdTimer.current = setTimeout(stopHolding, HOLD_LIMIT_MS);
  };

  const transcriptLine = (
    <span className="transcript-line">
      {transcript.chunks.map((chunk) => (
        <span
          key={chunk.key}
          className={chunk.arriving ? "transcript-arriving" : undefined}
        >
          {chunk.text}
        </span>
      ))}
      <span className="transcript-tentative">{transcript.tentative}</span>
    </span>
  );
  const indicator = (shape: WaveShape) =>
    completed ? (
      <CompletionMark label={cardLabel} />
    ) : (
      // One waveform for listening and for working, in the same element, so the
      // bars the user spoke into settle into the working ripple instead of being
      // swapped for a different indicator. Working carries the progressbar role
      // (indeterminate: nothing here can report a fraction).
      <span
        className="overlay-wave"
        role={working ? "progressbar" : undefined}
        aria-label={working ? busyLabel : undefined}
        aria-valuemin={working ? 0 : undefined}
        aria-valuemax={working ? 100 : undefined}
      >
        <AudioWaveform
          barCount={shape.bars}
          pitch={shape.pitch}
          barWidth={shape.barWidth}
          levels={live ? levels : EMPTY_LEVELS}
          size="sm"
          active={isVisible}
          mode={working ? "working" : "reactive"}
        />
      </span>
    );

  return (
    <div
      dir={getLanguageDirection(i18n.language)}
      className={`overlay-root ${isVisible ? "fade-in" : "native-window-hidden"}${fading ? " is-fading" : ""}${hopping ? " is-hopping" : ""}`}
      onContextMenu={preventBrowserContextMenu}
    >
      {streamingWindow ? (
        <div
          ref={cardRef}
          className={`overlay-card ${state}${completed ? " completed" : ""}`}
          role="group"
          aria-label={ariaLabel}
          onMouseEnter={() => reportHover(true)}
          onMouseLeave={() => reportHover(false)}
          onFocusCapture={() => reportHover(true)}
          onBlurCapture={() => reportHover(false)}
        >
          <div className="card-header">
            <div className="card-status" role="status">
              <div className="card-wave">{indicator(CARD_WAVE)}</div>
              {/* Keyed so a change of state (Recording → Transcribing) arrives
                  with a short fade instead of the word swapping in place. A
                  plain completion shows the check alone: "Done" beside it only
                  repeated the mark. A completion notice still has words worth
                  reading, so it keeps its label. */}
              {!(completed && !notice) && (
                <span key={cardLabel} className="card-label">
                  {cardLabel}
                </span>
              )}
            </div>
            <div className="card-actions">
              {/* Announced through the live region below; this is the part
                  sighted users can actually notice. */}
              {(copied || copyFailed) && (
                <span className="card-copied" aria-hidden="true">
                  {copyFailed ? t("overlay.copyFailed") : t("overlay.copied")}
                </span>
              )}
              {hasLiveText && canInteract && (
                <button
                  type="button"
                  className={`overlay-copy${copied ? " is-done" : ""}${copyFailed ? " is-error" : ""}`}
                  aria-label={
                    copyFailed
                      ? t("overlay.copyFailed")
                      : copied
                        ? t("overlay.copied")
                        : t("overlay.copy")
                  }
                  onClick={copyTranscript}
                >
                  {copied ? (
                    <Check size={13} strokeWidth={1.6} />
                  ) : (
                    <Copy size={12} strokeWidth={1.5} />
                  )}
                </button>
              )}
            </div>
          </div>
          <span className="overlay-sr-only" role="status">
            {copyFailed
              ? t("overlay.copyFailed")
              : copied
                ? t("overlay.copied")
                : ""}
          </span>
          <div
            className={`card-body${canInteract ? " is-selectable" : ""}`}
            ref={cardBodyRef}
            data-above={edges.above ? "" : undefined}
            data-below={edges.below ? "" : undefined}
            tabIndex={hasLiveText ? 0 : undefined}
            aria-label={t("overlay.transcript")}
            onScroll={onBodyScroll}
            onPointerDown={onBodyPointerDown}
            onPointerUp={releasePress}
          >
            {hasLiveText ? (
              <p
                key={transcript.reveal}
                className={`card-text${transcript.reveal ? " is-revealed" : ""}`}
              >
                {transcriptLine}
              </p>
            ) : (
              isRecording && (
                <p className="card-text card-hint">{t("overlay.listening")}</p>
              )
            )}
          </div>
        </div>
      ) : (
        <div
          className={`overlay-pill ${state}${labeled ? " labeled" : ""}${
            !labeled && hasLiveText && !working ? " has-text" : ""
          }`}
          role="group"
          aria-label={ariaLabel}
        >
          {labeled ? (
            <>
              {state !== "notice" && (
                <div className="pill-wave compact">
                  {indicator(LABELED_PILL_WAVE)}
                </div>
              )}
              <span className="pill-label" role="status">
                {pillLabel}
              </span>
            </>
          ) : hasLiveText && !working ? (
            <div className="stream-text-box" role="status">
              {transcriptLine}
            </div>
          ) : (
            <div className="pill-wave" role="status" aria-label={pillLabel}>
              {indicator(PILL_WAVE)}
            </div>
          )}
        </div>
      )}
    </div>
  );
};
export default RecordingOverlay;
