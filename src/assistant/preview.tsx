import React, { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import {
  AlertCircle,
  Camera,
  CameraOff,
  Check,
  Globe,
  Loader2,
  Maximize2,
  Mic,
  Volume2,
  X,
} from "lucide-react";
import AudioWaveform, {
  type WaveMode,
} from "@/components/shared/AudioWaveform";
import "./AssistantPanel.css";

import "@fontsource/plus-jakarta-sans/400.css";
import "@fontsource/plus-jakarta-sans/500.css";
import "@fontsource/plus-jakarta-sans/600.css";

// THROWAWAY visual-verification harness for the collapsed assistant pill.
// Not shipped — for eyeballing every pill state in a plain browser (no
// Playwright, per the redesign brief). Mirrors the pill markup from
// AssistantPanel.tsx; ?theme=light flips the palette.

type Demo =
  | "idle"
  | "idle-armed"
  | "listening"
  | "listening-locked"
  | "transcribing"
  | "thinking"
  | "speaking"
  | "searching"
  | "error";

// Sample copy for the demo states (not user-facing — the harness is dev-only).
const SAMPLE_ERROR = "Mic access blocked";

/** Plausible live vocal spectrum (16 bands), regenerated each tick so the
 *  reactive listening states actually move. */
function useFakeLevels(active: boolean): number[] {
  const [levels, setLevels] = useState<number[]>(new Array(16).fill(0));
  useEffect(() => {
    if (!active) return;
    let t = 0;
    const id = window.setInterval(() => {
      t += 1;
      setLevels(
        Array.from({ length: 16 }, (_, i) => {
          const base = Math.sin(t * 0.5 + i * 0.6) * 0.5 + 0.5;
          return Math.max(0, Math.min(1, base * (0.5 + Math.random() * 0.5)));
        }),
      );
    }, 90);
    return () => window.clearInterval(id);
  }, [active]);
  return levels;
}

const Pill: React.FC<{ demo: Demo }> = ({ demo }) => {
  const isListening = demo === "listening" || demo === "listening-locked";
  const locked = demo === "listening-locked";
  const busy =
    isListening ||
    demo === "transcribing" ||
    demo === "thinking" ||
    demo === "speaking" ||
    demo === "searching";
  const showError = demo === "error";
  const armed = demo === "idle-armed";
  const working = demo === "thinking" || demo === "transcribing";
  const levels = useFakeLevels(isListening);

  const waveMode: WaveMode = isListening
    ? "reactive"
    : demo === "speaking"
      ? "flow"
      : "shimmer";

  return (
    <div
      className={`apill${isListening ? " listening" : ""}${
        showError ? " error" : ""
      }`}
      role="status"
    >
      {showError ? (
        <>
          <AlertCircle size={14} className="apill-error-icon" />
          <span className="apill-error-text">{SAMPLE_ERROR}</span>
          <button className="apill-cancel">
            <X size={13} strokeWidth={2.5} />
          </button>
        </>
      ) : locked ? (
        <>
          <button className="apill-btn danger">
            <X size={14} strokeWidth={2.5} />
          </button>
          <div className="apill-wave">
            <AudioWaveform
              levels={levels}
              size="sm"
              barCount={12}
              mode="reactive"
              active
            />
          </div>
          <button className="apill-btn apill-done">
            <Check size={14} strokeWidth={2.75} />
          </button>
        </>
      ) : busy ? (
        <>
          {demo === "searching" && (
            <span className="apill-glyph">
              <Globe size={13} strokeWidth={2} />
            </span>
          )}
          {demo === "speaking" && (
            <span className="apill-glyph">
              <Volume2 size={13} strokeWidth={2} />
            </span>
          )}
          <div className="apill-wave">
            <AudioWaveform
              levels={isListening ? levels : []}
              size="sm"
              barCount={12}
              mode={waveMode}
              active={isListening}
            />
          </div>
          {working && (
            <Loader2 size={12} strokeWidth={2.5} className="apill-spin" />
          )}
          <button className="apill-cancel">
            <X size={13} strokeWidth={2.5} />
          </button>
        </>
      ) : (
        <>
          <button className="apill-btn apill-mic">
            <Mic size={13} strokeWidth={2.25} />
          </button>
          <div className="apill-wave rest">
            <AudioWaveform
              levels={[]}
              size="sm"
              barCount={8}
              mode="reactive"
              active={false}
            />
          </div>
          <div className="apill-reveal">
            <button className="apill-btn">
              <Maximize2 size={12} strokeWidth={2.25} />
            </button>
            <button className="apill-btn danger">
              <X size={13} strokeWidth={2.5} />
            </button>
          </div>
        </>
      )}
      {(armed || demo === "listening") && !showError && (
        <button className="apill-screen">
          <Camera size={9} strokeWidth={2.5} className="apill-screen-on" />
          <CameraOff size={9} strokeWidth={2.5} className="apill-screen-off" />
        </button>
      )}
    </div>
  );
};

const DEMOS: Demo[] = [
  "idle",
  "idle-armed",
  "listening",
  "listening-locked",
  "transcribing",
  "thinking",
  "speaking",
  "searching",
  "error",
];

const App: React.FC = () => {
  useEffect(() => {
    document.body.dataset.bg = "dark";
  }, []);

  return (
    <div className="grid assistant-scope">
      {DEMOS.map((demo) => (
        <div className="row" key={demo}>
          <div className="label">{demo}</div>
          <div className="win">
            <Pill demo={demo} />
          </div>
        </div>
      ))}
    </div>
  );
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
