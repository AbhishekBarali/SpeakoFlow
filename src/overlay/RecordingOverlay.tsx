import { listen } from "@tauri-apps/api/event";
import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Mic, AudioLines, Sparkles, X } from "lucide-react";
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

  // Clean Lucide glyphs, matching the icon language of the rest of the app.
  const renderIcon = () => {
    if (isRecording) {
      return <Mic size={17} strokeWidth={2} color={ICON_COLOR} />;
    }
    if (state === "processing") {
      return <Sparkles size={16} strokeWidth={2} color={ICON_COLOR} />;
    }
    return <AudioLines size={17} strokeWidth={2} color={ICON_COLOR} />;
  };

  return (
    <div
      dir={direction}
      className={`recording-overlay ${state} ${isVisible ? "fade-in" : ""}`}
    >
      <div className="overlay-left">
        <span className={`overlay-icon ${state}`}>{renderIcon()}</span>
      </div>

      <div className="overlay-middle">
        {isRecording && locked && (
          <div className="overlay-text">{t("overlay.locked")}</div>
        )}
        {isRecording && !locked && (
          <AudioWaveform
            levels={levels}
            size="sm"
            barCount={15}
            active={isVisible}
          />
        )}
        {state === "transcribing" && (
          <div className="overlay-text shimmer">{t("overlay.transcribing")}</div>
        )}
        {state === "processing" && (
          <div className="overlay-text shimmer">{t("overlay.processing")}</div>
        )}
      </div>

      <div className="overlay-right">
        {isRecording && (
          <button
            type="button"
            className="cancel-button"
            onClick={() => {
              commands.cancelOperation();
            }}
          >
            <X size={16} strokeWidth={2.5} color={ICON_COLOR} />
          </button>
        )}
      </div>
    </div>
  );
};

export default RecordingOverlay;
