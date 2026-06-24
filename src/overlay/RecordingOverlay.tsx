import { listen } from "@tauri-apps/api/event";
import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Lock, Loader2, X, Check } from "lucide-react";
import { AudioWaveform } from "../components/shared";
import "./RecordingOverlay.css";
import { commands } from "@/bindings";
import i18n, { syncLanguageFromSettings } from "@/i18n";
import { getLanguageDirection } from "@/lib/utils/rtl";

type OverlayState = "recording" | "transcribing" | "processing";

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
  const direction = getLanguageDirection(i18n.language);

  useEffect(() => {
    const setupEventListeners = async () => {
      // Listen for show-overlay event from Rust
      const unlistenShow = await listen("show-overlay", async (event) => {
        // Sync language from settings each time overlay is shown
        await syncLanguageFromSettings();
        const overlayState = event.payload as OverlayState;
        setState(overlayState);
        if (overlayState === "recording") {
          setLocked(false);
          // Reset readiness each time a new recording starts; the next
          // mic-level event will flip it back on once the stream is live.
          setMicLive(false);
        }
        setIsVisible(true);
      });

      // Listen for hide-overlay event from Rust
      const unlistenHide = await listen("hide-overlay", () => {
        setIsVisible(false);
        setLocked(false);
      });

      // Hands-free lock engaged (tap-to-lock)
      const unlistenLocked = await listen<boolean>("recording-locked", (e) => {
        setLocked(e.payload);
      });

      // Listen for mic-level updates. Smoothing + resampling is handled
      // inside AudioWaveform, so we just forward the raw payload.
      const unlistenLevel = await listen<number[]>("mic-level", (event) => {
        setLevels(event.payload as number[]);
        // mic-level updates are produced from real captured samples, so the
        // first one after a recording starts means the stream is genuinely
        // live and the user's words will now be captured.
        setMicLive(true);
      });

      // Cleanup function
      return () => {
        unlistenShow();
        unlistenHide();
        unlistenLocked();
        unlistenLevel();
      };
    };

    setupEventListeners();
  }, []);

  const isRecording = state === "recording";

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
      <div
        className={`overlay-pill ${state}${locked ? " locked" : ""}`}
        role="status"
        aria-label={ariaLabel}
      >
        {/* Live recording — the waveform carries the whole state. Before the
            first audio frame lands it simply rests as a calm row of dots, so
            the chip eases straight into motion the moment you speak instead of
            flashing a microphone glyph. A small lock badge appears for
            hands-free so the chip stays compact. */}
        {isRecording && (
          <div className="pill-wave">
            {locked && (
              <Lock
                className="lock-badge"
                size={12}
                strokeWidth={2.25}
                color={ICON_COLOR}
              />
            )}
            <div className="wave-box">
              <AudioWaveform
                levels={micLive ? levels : []}
                size="sm"
                barCount={14}
                active={micLive}
              />
            </div>
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
            <div className="wave-box">
              <AudioWaveform
                levels={[]}
                size="sm"
                barCount={14}
                active={false}
              />
            </div>
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
    </div>
  );
};

export default RecordingOverlay;
