import React, { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import {
  Check,
  Globe,
  Lock,
  Maximize2,
  Mic,
  Square,
  Volume2,
} from "lucide-react";
import AudioWaveform, {
  type WaveMode,
} from "@/components/shared/AudioWaveform";
import "./AssistantPanel.css";

import "@fontsource/plus-jakarta-sans/400.css";
import "@fontsource/plus-jakarta-sans/500.css";
import "@fontsource/plus-jakarta-sans/600.css";

// THROWAWAY visual-verification harness for the collapsed assistant pill.
// Not shipped — delete after design iteration. Reproduces the pill markup from
// AssistantPanel.tsx in every state so we can screenshot it in a browser.

type Demo =
  | "idle"
  | "listening"
  | "listening-locked"
  | "thinking"
  | "speaking"
  | "searching";

/** Plausible live vocal spectrum (16 bands), regenerated each tick so the
 *  reactive listening states actually move in the screenshots. */
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
  const pillStop =
    demo === "thinking" || demo === "speaking" || demo === "searching";
  const showPillWave = isListening || pillStop;
  const levels = useFakeLevels(isListening);

  const waveMode: WaveMode = isListening
    ? "reactive"
    : demo === "speaking"
      ? "flow"
      : "shimmer";

  const phaseIcon =
    demo === "searching" ? (
      <Globe size={14} strokeWidth={2} />
    ) : demo === "speaking" ? (
      <Volume2 size={14} strokeWidth={2} />
    ) : null;

  const label = demo === "idle" ? "Assistant" : "";

  return (
    <div className="assistant-pill" role="status">
      <div className="pill-mic-wrap">
        <button
          className={`pill-mic${isListening ? " recording" : ""}${
            pillStop ? " stopping" : ""
          }`}
        >
          {isListening ? (
            locked ? (
              <Check size={16} strokeWidth={2.75} />
            ) : (
              <Mic size={17} strokeWidth={2} />
            )
          ) : pillStop ? (
            <Square size={15} strokeWidth={2.5} />
          ) : (
            <Mic size={17} strokeWidth={2} />
          )}
        </button>
      </div>
      {showPillWave ? (
        <div className="pill-wave">
          {isListening && locked && (
            <Lock className="pill-lock-hint" size={13} strokeWidth={2.5} />
          )}
          <AudioWaveform
            levels={isListening ? levels : []}
            size="md"
            barCount={isListening && locked ? 13 : 16}
            mode={waveMode}
          />
        </div>
      ) : (
        <span className="pill-status">{label}</span>
      )}
      {phaseIcon && (
        <span className="pill-phase-icon" aria-hidden="true">
          {phaseIcon}
        </span>
      )}
      <button className="assistant-icon-button">
        <Maximize2 size={14} />
      </button>
    </div>
  );
};

const DEMOS: Demo[] = [
  "idle",
  "listening",
  "listening-locked",
  "thinking",
  "speaking",
  "searching",
];

const App: React.FC = () => {
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const theme = params.get("theme") === "light" ? "light" : "dark";
    document.documentElement.dataset.theme = theme;
    document.body.dataset.bg = theme;
  }, []);

  return (
    <div className="grid">
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
