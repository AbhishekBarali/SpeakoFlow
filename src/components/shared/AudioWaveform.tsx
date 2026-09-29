import React, { useEffect, useId, useRef, useState } from "react";
import {
  AUDIO_STALE_MS,
  REST_HEIGHT,
  SPEECH_RELEASE_MS,
  WORK_REST_HEIGHT,
  speechWave,
  stepSpring,
  voiceEnergy,
  workingWave,
  type Spring,
} from "./waveformSignal";
import { useReducedMotion } from "../../hooks/useReducedMotion";
import "./AudioWaveform.css";

/** `reactive` follows the microphone. `working` is the slow ripple shown while
 * the app transcribes or cleans up; switching between the two is animated, so
 * the bars the user spoke into settle into it instead of being replaced. */
export type WaveMode = "reactive" | "working";
export interface AudioWaveformProps {
  levels: number[];
  barCount?: number;
  active?: boolean;
  mode?: WaveMode;
  size?: "sm" | "md";
  /** Distance between bar centres, in viewBox units (the box is 24 high). */
  pitch?: number;
  /** Bar thickness, in the same units. */
  barWidth?: number;
  className?: string;
}

const restingSprings = (count: number): Spring[] =>
  Array.from({ length: count }, () => ({ position: REST_HEIGHT, velocity: 0 }));

const AudioWaveform: React.FC<AudioWaveformProps> = ({
  levels,
  barCount,
  active = true,
  mode = "reactive",
  size = "sm",
  pitch = 4,
  barWidth = 2,
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
  /** Where every bar is and how fast it is moving. Kept across a change of
   * mode, because that hand-off is the transition from listening to working. */
  const springs = useRef<Spring[]>([]);
  const reducedMotion = useReducedMotion();
  const [heights, setHeights] = useState(() =>
    Array<number>(count).fill(REST_HEIGHT),
  );

  useEffect(() => {
    const rest = Array<number>(count).fill(REST_HEIGHT);
    const park = () => {
      springs.current = restingSprings(count);
      setHeights(rest);
    };
    // Only a new bar count or a stopped indicator starts over from rest.
    if (springs.current.length !== count || !active) park();
    speaking.current = false;
    lastSpeech.current = 0;
    if (!active || typeof requestAnimationFrame !== "function") return;
    const working = mode === "working";
    const workRest = Array<number>(count).fill(WORK_REST_HEIGHT);
    let frame: number | null = null;
    let last = 0;
    let nextPaint = 0;
    let phase = 0;
    const step = (now: number) => {
      frame = null;
      if (svgRef.current?.closest(".native-window-hidden")) {
        park();
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
        !working &&
        speaking.current &&
        now - lastAudio.current <= AUDIO_STALE_MS &&
        now - lastSpeech.current <= SPEECH_RELEASE_MS;
      // The wave keeps travelling through the release, so speech that resumes
      // after a syllable gap does not restart from the identical crest.
      if (!reducedMotion) phase += Math.min(dt, 0.05);
      const target = working
        ? reducedMotion
          ? workRest
          : workingWave(count, phase)
        : speechActive
          ? speechWave(count, phase)
          : rest;
      if (!speechActive) speaking.current = false;
      let changed = false,
        settled = true;
      springs.current = springs.current.map((spring, index) => {
        const next = stepSpring(spring, target[index], dt);
        changed ||= next.position !== spring.position;
        settled &&= next.position === target[index] && next.velocity === 0;
        return next;
      });
      if (changed) setHeights(springs.current.map((spring) => spring.position));
      // Keep smoothing while speech arrives or work is under way, then release
      // and park completely. An idle microphone does not run an animation loop.
      if (!settled || speechActive || (working && !reducedMotion))
        frame = requestAnimationFrame(step);
    };
    const start = () => {
      if (frame !== null || svgRef.current?.closest(".native-window-hidden"))
        return;
      last = performance.now();
      nextPaint = last;
      frame = requestAnimationFrame(step);
    };
    wake.current = working ? null : start;
    // Working runs on its own clock. Back in reactive mode, a shape the
    // previous mode left behind still has to settle to rest.
    if (
      working ||
      springs.current.some(
        (spring) => spring.position !== REST_HEIGHT || spring.velocity !== 0,
      )
    )
      start();
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

  const width = (count - 1) * pitch + barWidth;
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
        const height = active ? (heights[index] ?? REST_HEIGHT) : REST_HEIGHT;
        return (
          <line
            key={index}
            className="wave-bar"
            x1={barWidth / 2 + index * pitch}
            x2={barWidth / 2 + index * pitch}
            y1={12 - height * 10}
            y2={12 + height * 10}
            stroke={`url(#${gradient})`}
            strokeWidth={barWidth}
            strokeLinecap="round"
            vectorEffect="non-scaling-stroke"
          />
        );
      })}
    </svg>
  );
};
export default AudioWaveform;
