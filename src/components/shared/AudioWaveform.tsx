import React, { useEffect, useId, useRef, useState } from "react";
import {
  AUDIO_STALE_MS,
  REST_HEIGHT,
  SPEECH_RELEASE_MS,
  speechWave,
  stepSpring,
  voiceEnergy,
  waveTargets,
  type Spring,
} from "./waveformSignal";
import { useReducedMotion } from "../../hooks/useReducedMotion";
import "./AudioWaveform.css";

export type WaveMode = "reactive" | "shimmer" | "flow";
export interface AudioWaveformProps {
  levels: number[];
  barCount?: number;
  active?: boolean;
  mode?: WaveMode;
  size?: "sm" | "md";
  className?: string;
}

const WORKING_LEVELS = [0.15, 0.4, 0.6, 0.3, 0.2, 0.45, 0.65, 0.4, 0.15];

const AudioWaveform: React.FC<AudioWaveformProps> = ({
  levels,
  barCount,
  active = true,
  mode = "reactive",
  size = "sm",
  className = "",
}) => {
  const count = Math.max(
    1,
    Math.min(80, Math.round(barCount ?? (size === "sm" ? 14 : 20)) || 14),
  );
  const gradient = useId();
  const svgRef = useRef<SVGSVGElement>(null);
  const speaking = useRef(false);
  const lastAudio = useRef(0);
  const lastSpeech = useRef(0);
  const wake = useRef<(() => void) | null>(null);
  const reducedMotion = useReducedMotion();
  const [heights, setHeights] = useState(() =>
    Array<number>(count).fill(REST_HEIGHT),
  );

  useEffect(() => {
    const rest = Array<number>(count).fill(REST_HEIGHT);
    let springs: Spring[] = rest.map((position) => ({ position, velocity: 0 }));
    speaking.current = false;
    lastSpeech.current = 0;
    setHeights(rest);
    if (
      mode !== "reactive" ||
      !active ||
      typeof requestAnimationFrame !== "function"
    )
      return;
    let frame: number | null = null;
    let last = 0;
    let nextPaint = 0;
    let phase = 0;
    const step = (now: number) => {
      frame = null;
      if (svgRef.current?.closest(".native-window-hidden")) {
        setHeights(rest);
        return;
      }
      // High-refresh displays need no more than 60 geometry updates per second
      // for this small indicator. Quiet still stops the clock entirely.
      if (now + 0.5 < nextPaint) {
        frame = requestAnimationFrame(step);
        return;
      }
      nextPaint = Math.max(nextPaint + 1000 / 60, now);
      const dt = (now - last) / 1000;
      last = now;
      const speechActive =
        speaking.current &&
        now - lastAudio.current <= AUDIO_STALE_MS &&
        now - lastSpeech.current <= SPEECH_RELEASE_MS;
      // The wave keeps travelling through the release, so speech that resumes
      // after a syllable gap does not restart from the identical crest.
      if (!reducedMotion) phase += Math.min(dt, 0.05);
      const target = speechActive ? speechWave(count, phase) : rest;
      if (!speechActive) speaking.current = false;
      let changed = false,
        settled = true;
      springs = springs.map((spring, index) => {
        const next = stepSpring(spring, target[index], dt);
        changed ||= next.position !== spring.position;
        settled &&= next.position === target[index] && next.velocity === 0;
        return next;
      });
      if (changed) setHeights(springs.map((spring) => spring.position));
      // Keep smoothing while speech arrives, then release and park completely.
      // An idle microphone does not run an animation loop.
      if (!settled || target.some((value) => value > REST_HEIGHT))
        frame = requestAnimationFrame(step);
    };
    wake.current = () => {
      if (frame !== null || svgRef.current?.closest(".native-window-hidden"))
        return;
      last = performance.now();
      nextPaint = last;
      frame = requestAnimationFrame(step);
    };
    return () => {
      if (frame !== null) cancelAnimationFrame(frame);
      wake.current = null;
    };
  }, [active, mode, count, reducedMotion]);

  useEffect(() => {
    if (mode !== "reactive" || !active) return;
    lastAudio.current = performance.now();
    if (voiceEnergy(levels) > 0) {
      speaking.current = true;
      lastSpeech.current = lastAudio.current;
      wake.current?.();
    }
    // The running clock bridges syllable gaps, releases, and then parks.
  }, [levels, active, mode, count, reducedMotion]);

  const display =
    mode === "reactive" ? heights : waveTargets(WORKING_LEVELS, count);
  const width = (count - 1) * 4 + 2;
  return (
    <svg
      ref={svgRef}
      className={`audio-waveform ${size} ${mode} ${active ? "" : "is-idle"} ${className}`}
      viewBox={`0 0 ${width} 24`}
      aria-hidden="true"
      focusable="false"
    >
      <defs>
        <linearGradient
          id={gradient}
          gradientUnits="userSpaceOnUse"
          x1="0"
          y1="22"
          x2="0"
          y2="2"
        >
          <stop offset="0%" className="wave-base" />
          <stop offset="100%" className="wave-tip" />
        </linearGradient>
      </defs>
      {Array.from({ length: count }, (_, index) => {
        const height =
          active || mode !== "reactive"
            ? (display[index] ?? REST_HEIGHT)
            : REST_HEIGHT;
        return (
          <line
            key={index}
            className="wave-bar"
            x1={1 + index * 4}
            x2={1 + index * 4}
            y1={12 - height * 10}
            y2={12 + height * 10}
            stroke={`url(#${gradient})`}
            strokeWidth="2"
            strokeLinecap="round"
            vectorEffect="non-scaling-stroke"
            style={{ "--phase": `${index * -0.085}s` } as React.CSSProperties}
          />
        );
      })}
    </svg>
  );
};
export default AudioWaveform;
