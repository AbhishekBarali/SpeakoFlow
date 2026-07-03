/**
 * Shared assistant-appearance constants: the single source of truth for the
 * panel window AND the settings preview, so the two can never drift.
 */

/** Message text size (user setting "small" | "medium" | "large"). */
export const FONT_SIZES: Record<string, string> = {
  small: "12.5px",
  medium: "13.5px",
  large: "15px",
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
