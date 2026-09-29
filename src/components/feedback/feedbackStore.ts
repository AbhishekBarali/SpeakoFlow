import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

/**
 * Feedback: who can open the dialog (sidebar "?", tray, Settings → About), the
 * commands behind it, and the small rules the form checks before sending.
 *
 * Commands go through `invoke` rather than the generated `commands.*` because
 * `bindings.ts` is only regenerated while the app runs in dev; these types
 * mirror `src-tauri/src/feedback.rs` by hand.
 */

export type FeedbackKind = "bug" | "idea" | "question";

export interface FeedbackSystemInfo {
  app_version: string;
  os: string;
  arch: string;
  install: string;
}

/** One screenshot on the wire; see `FeedbackAttachment` in feedback.rs. */
export interface FeedbackAttachment {
  media_type: string;
  data: string;
}

export interface FeedbackRequest {
  kind: FeedbackKind;
  message: string;
  email: string | null;
  include_system_info: boolean;
  attachments: FeedbackAttachment[];
}

export interface FeedbackOutcome {
  /** Screenshots the service could not store; the report arrived anyway. */
  attachments_dropped: number;
}

export const getFeedbackSystemInfo = () =>
  invoke<FeedbackSystemInfo>("get_feedback_system_info");

export const sendFeedback = (request: FeedbackRequest) =>
  invoke<FeedbackOutcome | null>("send_feedback", { request });

/** Same limits as the backend and the Worker. */
export const MIN_MESSAGE_CHARS = 3;
export const MAX_MESSAGE_CHARS = 5000;

export const messageLength = (message: string) =>
  Array.from(message.trim()).length;

export const canSend = (message: string, email: string) =>
  messageLength(message) >= MIN_MESSAGE_CHARS &&
  messageLength(message) <= MAX_MESSAGE_CHARS &&
  (email.trim() === "" || looksLikeEmail(email.trim()));

/** Loose on purpose: catch a typo, not police RFC 5322. Mirrors the backend. */
export function looksLikeEmail(email: string): boolean {
  if (email.length > 254 || /\s/.test(email)) return false;
  const parts = email.split("@");
  if (parts.length !== 2) return false;
  const [local, domain] = parts;
  return (
    local.length > 0 &&
    domain.includes(".") &&
    !domain.startsWith(".") &&
    !domain.endsWith(".") &&
    !domain.includes("..")
  );
}

/** One line for the "Include app version and system" checkbox. */
export const describeSystem = (info: FeedbackSystemInfo) =>
  `SpeakoFlow ${info.app_version} · ${info.os} · ${info.arch} · ${info.install}`;

// ── Draft ────────────────────────────────────────────────────────────────
// Closing the dialog, a failed send, or an app restart must not throw away a
// bug report someone took the time to write.

const DRAFT_KEY = "speakoflow.feedback.draft";
const EMAIL_KEY = "speakoflow.feedback.email";

export interface FeedbackDraft {
  kind: FeedbackKind;
  message: string;
}

const KINDS: FeedbackKind[] = ["bug", "idea", "question"];

export function parseDraft(raw: string | null): FeedbackDraft {
  const empty: FeedbackDraft = { kind: "bug", message: "" };
  if (!raw) return empty;
  try {
    const value = JSON.parse(raw) as Partial<FeedbackDraft>;
    return {
      kind: KINDS.includes(value.kind as FeedbackKind)
        ? (value.kind as FeedbackKind)
        : "bug",
      message: typeof value.message === "string" ? value.message : "",
    };
  } catch {
    return empty;
  }
}

const storage = (): Storage | null => {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
};

export const loadDraft = () =>
  parseDraft(storage()?.getItem(DRAFT_KEY) ?? null);
export const saveDraft = (draft: FeedbackDraft) =>
  storage()?.setItem(DRAFT_KEY, JSON.stringify(draft));
export const clearDraft = () => storage()?.removeItem(DRAFT_KEY);
/** The email is remembered separately: typing it once is enough. */
export const loadEmail = () => storage()?.getItem(EMAIL_KEY) ?? "";
export const saveEmail = (email: string) =>
  email.trim()
    ? storage()?.setItem(EMAIL_KEY, email.trim())
    : storage()?.removeItem(EMAIL_KEY);

// ── Open state ───────────────────────────────────────────────────────────

interface FeedbackDialogState {
  open: boolean;
  /** Kind to preselect, when the opener knows (e.g. a "report this" link). */
  kind: FeedbackKind | null;
  show: (kind?: FeedbackKind) => void;
  hide: () => void;
}

export const useFeedbackDialog = create<FeedbackDialogState>((set) => ({
  open: false,
  kind: null,
  show: (kind) => set({ open: true, kind: kind ?? null }),
  hide: () => set({ open: false, kind: null }),
}));
