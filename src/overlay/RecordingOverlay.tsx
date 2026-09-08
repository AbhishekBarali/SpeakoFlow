import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import AudioWaveform from "../components/shared/AudioWaveform";
import OverlayProgress from "./OverlayProgress";
import { voiceEnergy } from "../components/shared/waveformSignal";
import i18n, { syncLanguageFromSettings } from "@/i18n";
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
/** `settled` is how much of `committed` was already on screen last frame. The
 * remainder is what just arrived, and it is the only part that animates in — a
 * transcript that re-animates from the top on every update is the flicker that
 * made streaming look janky rather than live. */
type StreamTextPayload = { committed: string; tentative: string };
type StreamState = StreamTextPayload & { settled: number };
type ShowOverlayPayload = {
  state: OverlayState;
  streamingWindow: boolean;
  notice?: string;
};
const EMPTY_LEVELS: number[] = [];
const EMPTY_TEXT: StreamState = { committed: "", tentative: "", settled: 0 };

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

/** How far from the tail the transcript may be scrolled before it stops being
 * pinned, so following the text never fights a user reading further back. */
const FOLLOW_SLACK_PX = 48;

/** States that carry a written label instead of leaning on the waveform. The
 * backend widens the window for exactly these (see OVERLAY_LABEL_WIDTH). */
const LABELED: readonly OverlayState[] = ["generating", "vision", "notice"];

/** How long the copy button confirms before returning to its normal icon. */
const COPIED_FEEDBACK_MS = 1600;

const RecordingOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [isVisible, setIsVisible] = useState(false);
  const [state, setState] = useState<OverlayState>("recording");
  const [notice, setNotice] = useState<string | null>(null);
  const [locked, setLocked] = useState(false);
  const [levels, setLevels] = useState<number[]>(EMPTY_LEVELS);
  const [micLive, setMicLive] = useState(false);
  const [stream, setStream] = useState<StreamState>(EMPTY_TEXT);
  const [streamingWindow, setStreamingWindow] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  const [completed, setCompleted] = useState(false);
  const [fading, setFading] = useState(false);
  const completionEpoch = useRef<number | null>(null);
  const copyGeneration = useRef(0);
  const cardRef = useRef<HTMLDivElement>(null);
  const cardBodyRef = useRef<HTMLDivElement>(null);
  const followTail = useRef(true);
  const copiedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingText = useRef<StreamTextPayload | null>(null);
  const textFrame = useRef(false);
  /** What is currently on screen, so the next update can tell the newly decoded
   * tail from words the user has already read. */
  const shown = useRef({ committed: "", settled: 0 });

  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    let cancelled = false;
    let visible = false;
    let recording = false;
    const register = (unlisten: UnlistenFn) => {
      if (cancelled) unlisten();
      else unlisteners.push(unlisten);
    };
    /** Applies whatever the engine sent most recently, once per frame. Anything
     * that replaces the transcript wholesale (a new recording, the final
     * cleaned text) drops the buffer first so a frame in flight cannot paint
     * stale words over it. */
    const showText = (next: StreamState) => {
      pendingText.current = null;
      shown.current = { committed: next.committed, settled: next.settled };
      setStream(next);
    };
    const queueText = (next: StreamTextPayload) => {
      pendingText.current = next;
      if (textFrame.current) return;
      textFrame.current = true;
      onNextFrame(() => {
        textFrame.current = false;
        const latest = pendingText.current;
        if (!latest) return;
        pendingText.current = null;
        // Tracked in a ref, not derived from the previous state: React may
        // re-run an updater function, and this derivation is not idempotent —
        // a second pass would report the new tail as already settled.
        const previous = shown.current;
        const settled =
          latest.committed === previous.committed
            ? // Tentative-only update. Holding the boundary keeps the tail's
              // animation running instead of snapping it to full opacity.
              previous.settled
            : // Committed text only ever grows, so an extension means the tail
              // is genuinely new. A rewrite is adopted whole and silently:
              // animating a line the decoder just revised reads as a glitch.
              latest.committed.startsWith(previous.committed)
              ? previous.committed.length
              : latest.committed.length;
        shown.current = { committed: latest.committed, settled };
        setStream({ ...latest, settled });
      });
    };
    // Register together so a slow registration cannot leave hide unobserved.
    void Promise.all([
      listen<ShowOverlayPayload>("show-overlay", ({ payload }) => {
        completionEpoch.current = null;
        copyGeneration.current++;
        setCompleted(false);
        setFading(false);
        setCopyFailed(false);
        visible = true;
        recording = payload.state === "recording";
        setState(payload.state);
        setNotice(payload.notice ?? null);
        setStreamingWindow(payload.streamingWindow);
        if (recording) {
          followTail.current = true;
          setLocked(false);
          setMicLive(false);
          setLevels(EMPTY_LEVELS);
          showText(EMPTY_TEXT);
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
        setFading(false);
        visible = false;
        recording = false;
        setIsVisible(false);
        setLocked(false);
        setMicLive(false);
        setLevels(EMPTY_LEVELS);
        showText(EMPTY_TEXT);
        setCopied(false);
      }).then(register),
      listen<{ epoch: number; text: string; notice?: string }>(
        "finish-overlay",
        ({ payload }) => {
          if (!visible) return;
          completionEpoch.current = payload.epoch;
          copyGeneration.current++;
          recording = false;
          setCompleted(true);
          setFading(false);
          setMicLive(false);
          setNotice(payload.notice ?? null);
          showText({
            committed: payload.text,
            tentative: "",
            settled: payload.text.length,
          });
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
      listen<number[]>("mic-level", ({ payload }) => {
        if (!recording) return;
        setLevels(voiceEnergy(payload) === 0 ? EMPTY_LEVELS : payload);
        setMicLive(true);
      }).then(register),
      listen<StreamTextPayload>("stream-text", ({ payload }) => {
        if (visible) queueText(payload);
      }).then(register),
    ]).catch((error) => console.error("Overlay listeners failed:", error));
    return () => {
      cancelled = true;
      pendingText.current = null;
      for (const unlisten of unlisteners) unlisten();
    };
  }, []);

  useLayoutEffect(() => {
    const body = cardBodyRef.current;
    if (!body || !followTail.current) return;
    // Remember the user's position before text grows; measuring after a large
    // incoming chunk could incorrectly stop following the latest words.
    body.scrollTop = body.scrollHeight;
  }, [stream.committed, stream.tentative, streamingWindow]);

  useEffect(
    () => () => {
      if (copiedTimer.current) clearTimeout(copiedTimer.current);
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
  const hasLiveText = !!(stream.committed || stream.tentative);
  const live = isRecording && micLive;
  const labeled = LABELED.includes(state);
  const working = state !== "recording" && state !== "notice";

  const busyLabel =
    state === "notice"
      ? t(`overlay.notices.${notice ?? "flowFailed"}`)
      : t(`overlay.${state}`);
  // The pill has nowhere to put a second line, so its one label is the invitation
  // to speak. The card keeps the state word in its header and lets the body hold
  // the "Listening…" placeholder, so the two never say the same thing twice.
  const pillLabel = isRecording
    ? live
      ? t("overlay.listening")
      : t("overlay.preparing")
    : busyLabel;
  const cardLabel = completed
    ? notice
      ? t(`overlay.notices.${notice}`)
      : t("overlay.done")
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
    const holding =
      hovered || !!cardRef.current?.matches(":hover, :has(:focus-visible)");
    if (holding) setFading(false);
    void emit("overlay-hover", holding).catch((error) =>
      console.error("Overlay hover failed:", error),
    );
  };

  const copyTranscript = async () => {
    const text = `${stream.committed}${stream.tentative}`.trim();
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
  const settled = stream.committed.slice(0, stream.settled);
  const arriving = stream.committed.slice(stream.settled);
  const transcript = (
    <span className="transcript-line">
      <span>{settled}</span>
      {arriving && (
        // Remounting on a new tail is what restarts the animation; a stable key
        // during tentative-only updates is what stops it from restarting.
        <span
          key={`${stream.settled}:${arriving.length}`}
          className="transcript-arriving"
        >
          {arriving}
        </span>
      )}
      <span className="transcript-tentative">{stream.tentative}</span>
    </span>
  );
  const indicator = (barCount: number) =>
    working ? (
      <OverlayProgress
        label={completed ? t("overlay.done") : busyLabel}
        completed={completed}
        active={isVisible}
      />
    ) : (
      <AudioWaveform
        barCount={barCount}
        levels={live ? levels : EMPTY_LEVELS}
        size="sm"
        active={isVisible && !completed}
        mode="reactive"
      />
    );

  return (
    <div
      dir={getLanguageDirection(i18n.language)}
      className={`overlay-root ${isVisible ? "fade-in" : "native-window-hidden"}${fading ? " is-fading" : ""}`}
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
              <div className={`card-wave${working ? " is-progress" : ""}`}>
                {completed && !working ? (
                  <Check size={14} strokeWidth={1.8} />
                ) : (
                  indicator(14)
                )}
              </div>
              <span className="card-label">{cardLabel}</span>
            </div>
            <div className="card-actions">
              {hasLiveText && (
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
                  title={
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
            className="card-body"
            ref={cardBodyRef}
            tabIndex={hasLiveText ? 0 : undefined}
            aria-label={t("overlay.transcript")}
            onScroll={() => {
              const body = cardBodyRef.current;
              if (body)
                followTail.current =
                  body.scrollHeight - body.scrollTop - body.clientHeight <=
                  FOLLOW_SLACK_PX;
            }}
          >
            {hasLiveText ? (
              <p className="card-text">{transcript}</p>
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
                <div className="pill-wave compact">{indicator(9)}</div>
              )}
              <span className="pill-label" role="status">
                {pillLabel}
              </span>
            </>
          ) : hasLiveText && !working ? (
            <div className="stream-text-box" role="status">
              {transcript}
            </div>
          ) : (
            <div className="pill-wave" role="status" aria-label={pillLabel}>
              {indicator(14)}
            </div>
          )}
        </div>
      )}
    </div>
  );
};
export default RecordingOverlay;
