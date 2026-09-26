import React, { useId, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";

/**
 * The gradient banner at the top of a feature page — the one loud surface in
 * the app. It carries the page's single most useful fact (usually the
 * shortcut, drawn as frosted keys you can click to change) so the rest of the
 * page can be plain settings.
 *
 * Pages with room to spare can opt into `FlowArt` behind the content (`art`):
 * a few quiet lines that start as a gentle wave and settle into a paragraph,
 * drawn once when the page opens. Only Dictionary does; on busier heroes it
 * competed with the title and the keys.
 */

const ART_W = 600;
const ART_H = 200;
/**
 * Four lines of a short paragraph. Each starts as a small, soft wave on the
 * left and flattens into a straight line, ragged at the end like real text.
 * Amplitudes stay under half the line spacing, so lines never cross.
 */
const LINES = [
  { y: 80, end: 590, amplitude: 8, phase: 0.3, opacity: 0.34, width: 1.5 },
  { y: 98, end: 546, amplitude: 7, phase: 2.1, opacity: 0.26, width: 1.3 },
  { y: 116, end: 594, amplitude: 8, phase: 3.4, opacity: 0.3, width: 1.4 },
  { y: 134, end: 450, amplitude: 6, phase: 4.6, opacity: 0.2, width: 1.3 },
];

const smoothstep = (edge0: number, edge1: number, x: number) => {
  const t = Math.min(1, Math.max(0, (x - edge0) / (edge1 - edge0)));
  return t * t * (3 - 2 * t);
};

/** Catmull-Rom through the points, as cubic Béziers: smooth, no overshoot. */
const toPath = (points: Array<[number, number]>): string => {
  const f = (n: number) => n.toFixed(1);
  let d = `M${f(points[0][0])} ${f(points[0][1])}`;
  for (let i = 0; i < points.length - 1; i += 1) {
    const p0 = points[i - 1] ?? points[i];
    const p1 = points[i];
    const p2 = points[i + 1];
    const p3 = points[i + 2] ?? p2;
    d += ` C${f(p1[0] + (p2[0] - p0[0]) / 6)} ${f(p1[1] + (p2[1] - p0[1]) / 6)} ${f(
      p2[0] - (p3[0] - p1[0]) / 6,
    )} ${f(p2[1] - (p3[1] - p1[1]) / 6)} ${f(p2[0])} ${f(p2[1])}`;
  }
  return d;
};

/** One line of the art, as a path, plus where it ends. */
interface ArtLine {
  d: string;
  opacity: number;
  width: number;
}

/** Deterministic geometry — the same art every render, no randomness. */
const buildLines = (): ArtLine[] =>
  LINES.map((line) => {
    const point = (x: number): [number, number] => {
      // A soft wave on the left that flattens out: speech becoming text.
      const calm = 1 - smoothstep(0.25, 0.7, x / ART_W);
      return [
        x,
        line.y + line.amplitude * calm * Math.sin(0.028 * x + line.phase),
      ];
    };
    const points: Array<[number, number]> = [];
    for (let x = 0; x < line.end; x += 12) points.push(point(x));
    points.push(point(line.end));
    return { d: toPath(points), opacity: line.opacity, width: line.width };
  });

/**
 * Four quiet lines that start as a gentle wave and settle into a paragraph —
 * speech becoming text. Lives in the band between the title and the shortcut
 * keys (see `Hero`), anchored to its right edge, fading in from the left.
 */
export const FlowArt: React.FC<{ className?: string }> = ({
  className = "",
}) => {
  const lines = useMemo(buildLines, []);
  // Unique per instance: kept-alive pages each carry a hero, and a gradient
  // referenced by id from inside a hidden page would paint nothing.
  const uid = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  const fadeId = `flow-fade-${uid}`;
  const maskId = `flow-mask-${uid}`;

  return (
    <svg
      aria-hidden="true"
      viewBox={`0 0 ${ART_W} ${ART_H}`}
      preserveAspectRatio="xMaxYMid slice"
      className={`flow-art pointer-events-none ${className}`}
    >
      <defs>
        <linearGradient id={fadeId} x1="0" x2="1" y1="0" y2="0">
          <stop offset="0" stopColor="#fff" stopOpacity="0" />
          <stop offset="0.35" stopColor="#fff" stopOpacity="1" />
          <stop offset="1" stopColor="#fff" stopOpacity="1" />
        </linearGradient>
        <mask id={maskId}>
          <rect width={ART_W} height={ART_H} fill={`url(#${fadeId})`} />
        </mask>
      </defs>
      <g
        mask={`url(#${maskId})`}
        fill="none"
        stroke="#fff"
        strokeLinecap="round"
      >
        {lines.map((line, index) => (
          <path
            key={index}
            className="flow-ribbon"
            d={line.d}
            pathLength={1}
            strokeOpacity={line.opacity}
            strokeWidth={line.width}
            style={{ "--i": index } as React.CSSProperties}
          />
        ))}
      </g>
    </svg>
  );
};

export const Hero: React.FC<{
  title: React.ReactNode;
  /** One short line under the title. */
  subtitle?: React.ReactNode;
  children?: React.ReactNode;
  /** Right-hand column (shortcut keys, a toggle). */
  aside?: React.ReactNode;
  /** Makes the hero closable. The caller persists the choice. */
  onDismiss?: () => void;
  className?: string;
  /** Draw the decorative line art. Off by default: it is kept for pages with
   *  room to spare (Dictionary), and elsewhere it competed with the content. */
  art?: boolean;
}> = ({
  title,
  subtitle,
  children,
  aside,
  onDismiss,
  className = "",
  art = false,
}) => {
  const { t } = useTranslation();
  return (
    <section
      className={`hero-surface rounded-[1.25rem] px-7 py-7 sm:px-9 sm:py-8 ${className}`}
    >
      {onDismiss && (
        <button
          type="button"
          onClick={onDismiss}
          aria-label={t("common.close")}
          title={t("common.close")}
          className="glass-ghost absolute end-3.5 top-3.5 grid h-8 w-8 cursor-pointer place-items-center rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/70"
        >
          <X className="h-4 w-4" aria-hidden="true" />
        </button>
      )}
      <div className="flex flex-col gap-7 @3xl:flex-row @3xl:items-center">
        <div className="min-w-0 max-w-xl">
          <h2 className="font-display text-[1.875rem] text-white">{title}</h2>
          {subtitle && (
            <p className="mt-2 max-w-md text-[0.9375rem] leading-relaxed text-white/80 text-pretty">
              {subtitle}
            </p>
          )}
          {children && <div className="mt-6">{children}</div>}
        </div>
        {/* The art takes exactly the room between the title and the keys, so
            it can never run behind either of them. It bleeds to the hero's
            top and bottom edges, which clip it, and it only draws when the gap
            is wide enough to read as ribbons rather than a stray line. */}
        {art ? (
          <div
            aria-hidden="true"
            className="@container relative hidden min-h-24 min-w-0 flex-1 self-stretch @3xl:block"
          >
            <FlowArt className="absolute -top-8 start-0 hidden h-[calc(100%+4rem)] w-full @[11rem]:block" />
          </div>
        ) : (
          <div className="hidden flex-1 @3xl:block" />
        )}
        {aside && <div className="shrink-0">{aside}</div>}
      </div>
    </section>
  );
};

/** A labelled shortcut on the hero: "Dictate" over its frosted keys. */
export const HeroShortcut: React.FC<{
  label: string;
  hint?: React.ReactNode;
  children: React.ReactNode;
}> = ({ label, hint, children }) => (
  <div className="flex flex-col gap-2">
    <span className="text-[0.8125rem] font-medium text-white/75">{label}</span>
    {children}
    {hint && <span className="text-xs text-white/65">{hint}</span>}
  </div>
);
