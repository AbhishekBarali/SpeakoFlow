import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, X, Check } from "lucide-react";
import { AudioWaveform } from "../components/shared";
import "./RecordingOverlay.css";
import { commands } from "@/bindings";
import i18n, { syncLanguageFromSettings } from "@/i18n";
import { getLanguageDirection } from "@/lib/utils/rtl";

type OverlayState = "recording" | "transcribing" | "processing";

/** Payload of the untyped Rust "stream-text" event (live transcription). */
type StreamTextPayload = { committed: string; tentative: string };

/**
 * Payload of the Rust "show-overlay" event. `streamingWindow` is true when the
 * opt-in live-transcription window is active (the overlay has been enlarged to
 * the readable card); false for the compact pill (the default).
 */
type ShowOverlayPayload = { state: OverlayState; streamingWindow: boolean };

/** Warm off-white that matches the editorial ink-on-dark palette. */
const ICON_COLOR = "#f5f5f4";

const RecordingOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [isVisible, setIsVisible] = useState(false);
  const [state, setState] = useState<OverlayState>("recording");
  const [locked, setLocked] = useState(false);
  const [levels, setLevels] = useState<number[]>([]);
  // Whether the mic has started delivering audio for this recording. Driven by
  // real mic-level events, so it reflects actual stream readiness rather than a
  // guessed delay — this is the cue that it's safe to start speaking.
  const [micLive, setMicLive] = useState(false);
  // Incremental live/streaming transcription text (opt-in). Both parts stay
  // empty unless the user has enabled live transcription; driven by the
  // "stream-text" event. `committed` is the stable, append-only prefix;
  // `tentative` is the volatile tail the model may still revise (populated by
  // the native transcribe.cpp streaming path; empty for cache-aware models
  // like Parakeet and for the VadChunked path).
  const [stream, setStream] = useState<StreamTextPayload>({
    committed: "",
    tentative: "",
  });
  // Whether the opt-in live-transcription window is active for this recording
  // (overlay enlarged to the readable card). Driven by the show-overlay event;
  // false means the compact pill (unchanged default behavior).
  const [streamingWindow, setStreamingWindow] = useState(false);
  // The scrollable transcript area of the live card; kept pinned to the bottom
  // as new words arrive so the latest text stays visible.
  const cardBodyRef = useRef<HTMLDivElement>(null);
  const direction = getLanguageDirection(i18n.language);

  useEffect(() => {
    // Collect every unlisten handle so all subscriptions are torn down on
    // unmount. Listeners are registered asynchronously, so we also guard
    // against the component unmounting mid-registration (any handle that
    // arrives after cleanup is unlistened immediately). This prevents the
    // webview process from leaking event subscriptions across long recording
    // sessions (mic-level fires many times per second).
    let unlisteners: UnlistenFn[] = [];
    let cancelled = false;

    const register = (unlisten: UnlistenFn) => {
      if (cancelled) {
        unlisten();
      } else {
        unlisteners.push(unlisten);
      }
    };

    const setupEventListeners = async () => {
      // Listen for show-overlay event from Rust
      register(
        await listen("show-overlay", async (event) => {
          // Sync language from settings each time overlay is shown
          await syncLanguageFromSettings();
          const payload = event.payload as ShowOverlayPayload;
          const overlayState = payload.state;
          setState(overlayState);
          // Whether to render the enlarged live-transcription card vs the
          // compact pill (opt-in; false by default).
          setStreamingWindow(payload.streamingWindow);
          if (overlayState === "recording") {
            setLocked(false);
            // Reset readiness each time a new recording starts; the next
            // mic-level event will flip it back on once the stream is live.
            setMicLive(false);
            // Clear any live transcript carried over from a prior recording.
            setStream({ committed: "", tentative: "" });
          }
          setIsVisible(true);
        }),
      );

      // Listen for hide-overlay event from Rust
      register(
        await listen("hide-overlay", () => {
          setIsVisible(false);
          setLocked(false);
          setStream({ committed: "", tentative: "" });
        }),
      );

      // Hands-free lock engaged (tap-to-lock)
      register(
        await listen<boolean>("recording-locked", (e) => {
          setLocked(e.payload);
        }),
      );

      // Listen for mic-level updates. Smoothing + resampling is handled
      // inside AudioWaveform, so we just forward the raw payload.
      register(
        await listen<number[]>("mic-level", (event) => {
          setLevels(event.payload as number[]);
          // mic-level updates are produced from real captured samples, so the
          // first one after a recording starts means the stream is genuinely
          // live and the user's words will now be captured.
          setMicLive(true);
        }),
      );

      // Incremental live/streaming transcription text (opt-in). No-op in
      // practice unless the user enabled live transcription. Keeps both the
      // committed prefix and the tentative tail so the overlay can render the
      // stable text solid and the still-revisable tail dimmed.
      register(
        await listen<StreamTextPayload>("stream-text", (event) => {
          setStream({
            committed: event.payload.committed,
            tentative: event.payload.tentative,
          });
        }),
      );
    };

    setupEventListeners();

    // Cleanup: invoke every captured unlisten so no subscription outlives the
    // component (the source of the WebKitWebProcess memory leak).
    return () => {
      cancelled = true;
      for (const unlisten of unlisteners) {
        unlisten();
      }
      unlisteners = [];
    };
  }, []);

  // Keep the live card's transcript scrolled to the newest words as they
  // stream in. No-op for the compact pill (the ref is only attached in card
  // mode).
  useEffect(() => {
    const el = cardBodyRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }, [stream.committed, stream.tentative, streamingWindow]);

  const isRecording = state === "recording";

  // Live-transcription ticker: shown in place of the waveform once streaming
  // text arrives (opt-in). The committed prefix renders solid; the tentative
  // tail renders dimmed so the user can see the still-revisable words without
  // mistaking them for final text. Concatenated with no separator to match the
  // backend's committed+tentative split.
  const hasLiveText =
    stream.committed.length > 0 || stream.tentative.length > 0;
  const streamTextBox = (
    <div className="stream-text-box">
      <span className="stream-text">{stream.committed}</span>
      {stream.tentative ? (
        <span className="stream-text stream-text-tentative">
          {stream.tentative}
        </span>
      ) : null}
    </div>
  );

  // Accessible label for the current state — the visible chip is intentionally
  // terse (an icon or just the waveform), so the full phrasing lives here.
  const ariaLabel = isRecording
    ? locked
      ? t("overlay.locked")
      : micLive
        ? t("overlay.recording", "Recording")
        : t("overlay.preparing")
    : state === "transcribing"
      ? t("overlay.transcribing")
      : t("overlay.processing");

  return (
    <div
      dir={direction}
      className={`overlay-root ${isVisible ? "fade-in" : ""}`}
    >
      {streamingWindow ? (
        /* Opt-in live-transcription window: the overlay is enlarged (400×120)
           into a readable card that shows the running committed (solid) +
           tentative (dimmed) transcript. Gated on the setting in Rust; here we
           only switch layout. */
        <div
          className={`overlay-card ${state}${locked ? " locked" : ""}`}
          role="status"
          aria-label={ariaLabel}
        >
          <div className="card-header">
            <div className="card-status">
              {isRecording ? (
                <div className="wave-box card-wave">
                  <AudioWaveform
                    levels={micLive ? levels : []}
                    size="sm"
                    barCount={14}
                    active={micLive}
                  />
                </div>
              ) : (
                <Loader2
                  className="load-spinner"
                  size={14}
                  strokeWidth={2.5}
                  color={ICON_COLOR}
                />
              )}
              <span className="card-label">{ariaLabel}</span>
            </div>
            <div className="card-actions">
              {isRecording && (
                <button
                  type="button"
                  className="card-btn card-cancel"
                  aria-label={t("overlay.cancel", "Cancel")}
                  onClick={() => {
                    commands.cancelOperation();
                  }}
                >
                  <X size={13} strokeWidth={2.5} color={ICON_COLOR} />
                </button>
              )}
              {isRecording && locked && (
                <button
                  type="button"
                  className="card-btn card-confirm"
                  aria-label={t("overlay.done", "Done")}
                  onClick={() => {
                    commands.commitRecording();
                  }}
                >
                  <Check size={13} strokeWidth={2.5} color={ICON_COLOR} />
                </button>
              )}
            </div>
          </div>
          <div className="card-body" ref={cardBodyRef}>
            {hasLiveText ? (
              <p className="card-text">
                <span className="card-committed">{stream.committed}</span>
                {stream.tentative ? (
                  <span className="card-tentative">{stream.tentative}</span>
                ) : null}
              </p>
            ) : (
              <p className="card-text card-placeholder">
                {t("overlay.listening", "Listening…")}
              </p>
            )}
          </div>
        </div>
      ) : (
        <div
          className={`overlay-pill ${state}${locked ? " locked" : ""}`}
          role="status"
          aria-label={ariaLabel}
        >
        {/* Live recording — the waveform carries the whole state. Before the
            first audio frame lands it simply rests as a calm row of dots, so
            the chip eases straight into motion the moment you speak instead of
            flashing a microphone glyph. Hands-free (locked) mode is signalled
            by the "done" tick easing in (see below), so the chip needs no extra
            badge and stays compact. */}
        {isRecording && (
          <div className="pill-wave">
            {hasLiveText ? (
              streamTextBox
            ) : (
              <div className="wave-box">
                <AudioWaveform
                  levels={micLive ? levels : []}
                  size="sm"
                  barCount={14}
                  active={micLive}
                />
              </div>
            )}
          </div>
        )}

        {/* Working (transcribing / post-processing) — keep the audio identity
            but settle the bars and tuck a quiet spinner alongside, instead of a
            word. The waveform freezing + spinner reads as "done capturing, now
            thinking". */}
        {(state === "transcribing" || state === "processing") && (
          <div className="pill-wave">
            <Loader2
              className="load-spinner"
              size={13}
              strokeWidth={2.5}
              color={ICON_COLOR}
            />
            {hasLiveText ? (
              streamTextBox
            ) : (
              <div className="wave-box">
                <AudioWaveform
                  levels={[]}
                  size="sm"
                  barCount={14}
                  active={false}
                />
              </div>
            )}
          </div>
        )}

        {/* Cancel stays out of the way until you reach for it. */}
        {isRecording && (
          <button
            type="button"
            className="pill-cancel"
            aria-label={t("overlay.cancel", "Cancel")}
            onClick={() => {
              commands.cancelOperation();
            }}
          >
            <X size={13} strokeWidth={2.5} color={ICON_COLOR} />
          </button>
        )}

        {/* Hands-free (locked) recording isn't ended by releasing a key, so it
            gets a persistent "done" tick to finish and transcribe — alongside
            re-pressing the hotkey. Hidden during a push-to-talk hold. */}
        {isRecording && locked && (
          <button
            type="button"
            className="pill-confirm"
            aria-label={t("overlay.done", "Done")}
            onClick={() => {
              commands.commitRecording();
            }}
          >
            <Check size={13} strokeWidth={2.5} color={ICON_COLOR} />
          </button>
        )}
      </div>
      )}
    </div>
  );
};

export default RecordingOverlay;
