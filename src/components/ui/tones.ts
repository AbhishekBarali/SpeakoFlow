import type React from "react";

/**
 * Semantic color tones for the iOS-style setting rows: each row gets a soft
 * tinted icon tile on the left, and some controls (shortcut pills) pick up the
 * same hue. Tints stay low-saturation (`/15`) so they read as calm accents in
 * both light and dark themes rather than shouting. `teal` matches the brand
 * accent; the rest add just enough variety to make scanning easier.
 */
export type SettingTone =
  | "teal"
  | "rose"
  | "violet"
  | "amber"
  | "sky"
  | "emerald"
  | "indigo";

/** Icon component shape — compatible with lucide-react icons. */
export type SettingIcon = React.ComponentType<{
  className?: string;
  size?: number | string;
  strokeWidth?: number | string;
}>;

/** Soft-tinted tile: background wash + saturated glyph. */
export const TONE_TILE: Record<SettingTone, string> = {
  teal: "bg-teal-500/15 text-teal-600 dark:bg-teal-400/15 dark:text-teal-300",
  rose: "bg-rose-500/15 text-rose-600 dark:bg-rose-400/15 dark:text-rose-300",
  violet:
    "bg-violet-500/15 text-violet-600 dark:bg-violet-400/15 dark:text-violet-300",
  amber:
    "bg-amber-500/15 text-amber-600 dark:bg-amber-400/20 dark:text-amber-300",
  sky: "bg-sky-500/15 text-sky-600 dark:bg-sky-400/15 dark:text-sky-300",
  emerald:
    "bg-emerald-500/15 text-emerald-600 dark:bg-emerald-400/15 dark:text-emerald-300",
  indigo:
    "bg-indigo-500/15 text-indigo-600 dark:bg-indigo-400/15 dark:text-indigo-300",
};

/** Tinted value pill (e.g. a shortcut chip) that echoes the row's tone. */
export const TONE_PILL: Record<SettingTone, string> = {
  teal: "bg-teal-500/10 text-teal-700 dark:text-teal-300 border-teal-500/30",
  rose: "bg-rose-500/10 text-rose-700 dark:text-rose-300 border-rose-500/30",
  violet:
    "bg-violet-500/10 text-violet-700 dark:text-violet-300 border-violet-500/30",
  amber:
    "bg-amber-500/10 text-amber-700 dark:text-amber-300 border-amber-500/30",
  sky: "bg-sky-500/10 text-sky-700 dark:text-sky-300 border-sky-500/30",
  emerald:
    "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300 border-emerald-500/30",
  indigo:
    "bg-indigo-500/10 text-indigo-700 dark:text-indigo-300 border-indigo-500/30",
};
