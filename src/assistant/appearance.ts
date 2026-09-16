/**
 * Shared assistant-appearance constants: the single source of truth for the
 * panel window AND the settings preview, so the two can never drift.
 */

/** Message text size (user setting "small" | "medium" | "large" | "extra_large").
 *
 * The scale used to run 12.5 / 13.5 / 15 px, which had two problems. The step
 * from small to medium was 1 px — not a choice anyone can see — and the top of
 * the scale was roughly what the rest of the app calls default, so a reader who
 * wanted bigger text had nothing left to pick. The panel deliberately does not
 * follow the main window's `ui_text_size` (it is sized for a small floating
 * surface, not a settings page), which means this list is the only lever there
 * is, and it has to actually reach a comfortable reading size.
 */
export const FONT_SIZES: Record<string, string> = {
  small: "13px",
  medium: "14.5px",
  large: "16px",
  extra_large: "18px",
};

/** Structured error codes emitted by the backend (`assistant-error`).
 *  `blocking` errors need a fix (settings/permissions); `transient` ones are
 *  worth retrying. The pill auto-dismisses transient errors only. */
export type AssistantErrorKind = "transient" | "blocking";

export const ERROR_KINDS: Record<string, AssistantErrorKind> = {
  no_provider: "blocking",
  no_model: "blocking",
  mic_denied: "blocking",
  mic_unavailable: "blocking",
  vision_unsupported: "blocking",
  engine_start: "transient",
  provider: "transient",
  empty_reply: "transient",
  screenshot_too_large: "transient",
  screen_capture: "transient",
  transcription: "transient",
  tts: "transient",
  tts_local: "transient",
  tts_blocked: "transient",
  tts_playback: "transient",
  mic_error: "transient",
  file_read: "transient",
};

export interface AssistantError {
  /** Stable backend code, or null when the failure came from the webview. */
  code: string | null;
  /** Raw provider/OS detail for the expanded view / unknown-code fallback. */
  detail: string;
}

export const errorKind = (error: AssistantError): AssistantErrorKind =>
  (error.code && ERROR_KINDS[error.code]) || "transient";
