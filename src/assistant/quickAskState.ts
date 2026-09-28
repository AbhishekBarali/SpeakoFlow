/**
 * What the quick ask is showing, derived from the assistant's pipeline state.
 *
 * Pure, so the one decision that makes the surface feel responsive or dead —
 * "is there something to read yet?" — is a test rather than a screenshot.
 *
 * The quick ask is one question and one answer. It has no follow-up field and no
 * thread: a conversation is what the call is for. So every phase below belongs
 * to the single exchange in front of the user, and the next ask starts clean.
 */

export type AssistantState =
  | "idle"
  | "listening"
  | "transcribing"
  | "searching"
  | "thinking"
  | "speaking";

export type QuickAskPhase =
  /** Opened from the tray with nothing in flight: a field to type into. */
  | "prompt"
  /** The microphone is open. */
  | "listening"
  /** The recording is being turned into text. */
  | "transcribing"
  /** The model is working and has not written anything yet. */
  | "working"
  /** The answer is streaming in. */
  | "answering"
  /** The answer is complete. */
  | "done"
  /** The ask failed. */
  | "error";

/**
 * The surface's three shapes. The pill is for waiting, the bar is for typing, and
 * the card is for reading. They are one element that morphs, never a swap.
 */
export type QuickAskShape = "pill" | "bar" | "card";

export interface QuickAskInputs {
  state: AssistantState;
  /** The answer as it streams in, before the backend's final snapshot. */
  stream: string;
  /** The finished answer to the current question, if there is one. */
  answer: string;
  /** An error is on screen. */
  hasError: boolean;
}

export function quickAskPhase({
  state,
  stream,
  answer,
  hasError,
}: QuickAskInputs): QuickAskPhase {
  if (state === "listening") return "listening";
  if (state === "transcribing") return "transcribing";
  const busy = state !== "idle";
  if (hasError && !busy) return "error";
  // Text on screen means the card, from the very first token. Waiting for the
  // whole answer before showing any of it is what made the old surface feel
  // stuck: a long reply spent its entire generation behind a spinner.
  if (stream.trim()) return busy ? "answering" : "done";
  if (answer.trim()) return busy ? "answering" : "done";
  return busy ? "working" : "prompt";
}

export function quickAskShape(phase: QuickAskPhase): QuickAskShape {
  switch (phase) {
    case "prompt":
      return "bar";
    case "listening":
    case "transcribing":
    case "working":
      return "pill";
    default:
      return "card";
  }
}

/** A tool the current turn is running, as reported by the backend. */
export interface ToolActivity {
  /** Backend tool id, e.g. `web_search`. */
  name: string;
  /** The one argument worth showing — currently only the search query. */
  detail: string;
}

/**
 * The translation key for what the assistant is doing while the pill waits.
 *
 * Kept to four short states — listening, thinking, searching the web, looking
 * at the screen — because the pill is glanced at, not read. Transcription used
 * to have its own label, and every other tool a generic "Working on it"; to the
 * person waiting both are just the assistant thinking, and a pill that flips
 * through five labels in two seconds reads as noise. A tool that changes what
 * the user should expect (the web, their screen) still gets named, and beats
 * the pipeline state because it is the more specific answer.
 */
export function workingLabelKey(
  phase: QuickAskPhase,
  state: AssistantState,
  tool: ToolActivity | null,
): string {
  if (phase === "listening") return "assistant.status.listening";
  if (tool?.name === "web_search") return "assistant.status.searching";
  if (tool?.name === "capture_screen") return "assistant.tool.screen";
  if (!tool && state === "searching") return "assistant.status.searching";
  return "assistant.status.thinking";
}

/**
 * Where the surface sits inside its fixed frame, as sent by Rust
 * (`assistant-ask-layout`). The frame is placed at the dock zone the user chose;
 * this pins the surface to the edge of the frame that is flush with the screen
 * edge, so the pill lands exactly where it was docked and the card grows away
 * from that edge.
 */
export interface QuickAskLayout {
  align: "top" | "bottom";
  justify: "start" | "center" | "end";
}

export const DEFAULT_LAYOUT: QuickAskLayout = {
  align: "top",
  justify: "center",
};

/** Accept only a well-formed layout, falling back to the default. */
export function parseLayout(payload: unknown): QuickAskLayout {
  if (!payload || typeof payload !== "object") return DEFAULT_LAYOUT;
  const { align, justify } = payload as Record<string, unknown>;
  return {
    align: align === "bottom" ? "bottom" : "top",
    justify: justify === "start" || justify === "end" ? justify : "center",
  };
}
