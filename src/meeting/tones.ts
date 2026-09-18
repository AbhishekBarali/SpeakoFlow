import type { SettingTone } from "@/components/ui/tones";

/**
 * Speaker colours as literal hex, for the pill's own window.
 *
 * The app's `TONE_TILE` / `TONE_PILL` are Tailwind class strings that resolve
 * against the settings window's light/dark theme variables. The pill is a
 * standalone always-dark surface that defines its own tokens
 * (`MeetingPill.css`), so those classes would render against variables that do
 * not exist here — a speaker label with no colour at all.
 *
 * These are the Tailwind `-300` steps, which is what `TONE_TILE` resolves to in
 * dark mode, so a speaker keeps the same hue whether it is read in the pill or in
 * Settings. Keyed on `SettingTone` so `speakerTone()` stays the single source of
 * which speaker gets which colour.
 */
export const TONE_HEX: Record<SettingTone, string> = {
  teal: "#5eead4",
  rose: "#fda4af",
  violet: "#c4b5fd",
  amber: "#fcd34d",
  sky: "#7dd3fc",
  emerald: "#6ee7b7",
  indigo: "#a5b4fc",
};
