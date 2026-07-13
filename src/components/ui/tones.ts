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
  teal: "bg-teal-500/15 text-teal-600 dark:bg-teal-400/25 dark:text-teal-300",
  rose: "bg-rose-500/15 text-rose-600 dark:bg-rose-400/25 dark:text-rose-300",
  violet:
    "bg-violet-500/15 text-violet-600 dark:bg-violet-400/25 dark:text-violet-300",
  amber:
    "bg-amber-500/15 text-amber-600 dark:bg-amber-400/25 dark:text-amber-300",
  sky: "bg-sky-500/15 text-sky-600 dark:bg-sky-400/25 dark:text-sky-300",
  emerald:
    "bg-emerald-500/15 text-emerald-600 dark:bg-emerald-400/25 dark:text-emerald-300",
  indigo:
    "bg-indigo-500/15 text-indigo-600 dark:bg-indigo-400/15 dark:text-indigo-300",
};
/** Vivid gradient tile: saturated gradient + white glyph (iOS feature-tile
 *  style). For hero moments — onboarding cards, nav cards — where the UI
 *  should feel alive; quiet settings rows keep the soft TONE_TILE wash. */
export const TONE_TILE_VIVID: Record<SettingTone, string> = {
  teal: "bg-gradient-to-br from-teal-400 to-teal-600 text-white shadow-sm",
  rose: "bg-gradient-to-br from-rose-400 to-rose-600 text-white shadow-sm",
  violet:
    "bg-gradient-to-br from-violet-400 to-violet-600 text-white shadow-sm",
  amber: "bg-gradient-to-br from-amber-400 to-orange-500 text-white shadow-sm",
  sky: "bg-gradient-to-br from-sky-400 to-blue-600 text-white shadow-sm",
  emerald:
    "bg-gradient-to-br from-emerald-400 to-emerald-600 text-white shadow-sm",
  indigo:
    "bg-gradient-to-br from-indigo-400 to-indigo-600 text-white shadow-sm",
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
