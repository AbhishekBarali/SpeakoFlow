import type { AssistantHistorySummary, HistoryEntry } from "@/bindings";
import { VOICE_INTERRUPTED_MARKER } from "@/assistant/conversationPolicy";

/** Stable marker written by src-tauri/src/flow.rs. Existing successful Flow
 *  rows already carry this value, so they appear in the Flow filter too. */
export const FLOW_HISTORY_MARKER = "Generate with Flow";

/** Must match the marker constants in src-tauri/src/assistant.rs. */
const SCREENSHOT_MARKER = "[screenshot attached]";
const IMAGE_MARKER = "[image attached]";
const FILE_MARKER_PREFIX = "[file attached:";
const SELECTION_OPEN = "<selected_text>";
const SELECTION_CLOSE = "</selected_text>";
const SELECTION_LEAD_IN =
  "The user has this text selected in another application:";
const SELECTION_REQUEST_PREFIX = "Their request about it:";

export const isFlowHistoryEntry = (entry: HistoryEntry): boolean =>
  entry.post_process_prompt === FLOW_HISTORY_MARKER;

/**
 * One item in the merged history feed. Transcriptions and assistant
 * conversations are interleaved by time; `sortTime` is the seconds-epoch used
 * for ordering (last activity for conversations, recording time otherwise).
 */
export type FeedItem =
  | { kind: "transcription"; sortTime: number; entry: HistoryEntry }
  | { kind: "assistant"; sortTime: number; session: AssistantHistorySummary };

export type HistoryFilter = "all" | "recordings" | "flow" | "asks" | "calls";

/** What a row is, which is what the filters and the "All" list name. */
export type RowKind = "dictation" | "flow" | "ask" | "call";

export const rowKind = (item: FeedItem): RowKind => {
  if (item.kind === "transcription") {
    return isFlowHistoryEntry(item.entry) ? "flow" : "dictation";
  }
  return item.session.kind === "call" ? "call" : "ask";
};

const FILTER_KIND: Record<Exclude<HistoryFilter, "all">, RowKind> = {
  recordings: "dictation",
  flow: "flow",
  asks: "ask",
  calls: "call",
};

export const matchesFilter = (item: FeedItem, filter: HistoryFilter) =>
  filter === "all" || rowKind(item) === FILTER_KIND[filter];

/**
 * The tabs worth showing. A feature that is switched off gets no tab: a
 * "Flow" tab with Flow off is a door to an empty room. Anything it left behind
 * still shows under All, which is history rather than a feature.
 */
export const visibleFilters = ({
  flowEnabled,
  assistantEnabled,
}: {
  flowEnabled: boolean;
  assistantEnabled: boolean;
}): HistoryFilter[] => {
  const filters: HistoryFilter[] = ["all", "recordings"];
  if (flowEnabled) filters.push("flow");
  if (assistantEnabled) filters.push("asks", "calls");
  return filters;
};

/** Merge both lists into one feed, newest activity first. */
export const buildFeed = (
  entries: readonly HistoryEntry[],
  sessions: readonly AssistantHistorySummary[],
): FeedItem[] => {
  const items: FeedItem[] = [
    ...entries.map(
      (entry): FeedItem => ({
        kind: "transcription",
        sortTime: entry.timestamp,
        entry,
      }),
    ),
    ...sessions.map(
      (session): FeedItem => ({
        kind: "assistant",
        sortTime: session.updated_at,
        session,
      }),
    ),
  ];
  return items.sort((a, b) => b.sortTime - a.sortTime);
};

export interface CleanMessage {
  text: string;
  screenshot: boolean;
  files: string[];
  /** Characters of text the user had selected elsewhere, 0 when none. */
  selectionChars: number;
}

/**
 * A stored message as a person should read it. The backend writes markers the
 * model needs (attachments, a barge-in note) and, for a question about a
 * selection, a lead-in sentence and the selected text itself. The selection
 * is replaced by a count, the same way the assistant panel shows it, because
 * re-reading your own highlighted paragraph above every answer is noise.
 */
export const cleanMessageContent = (raw: string): CleanMessage => {
  let screenshot = false;
  let selectionChars = 0;
  let inSelection = false;
  const files: string[] = [];
  const kept: string[] = [];
  for (const line of raw.split("\n")) {
    const trimmed = line.trim();
    if (trimmed === SELECTION_OPEN) {
      inSelection = true;
      continue;
    }
    if (trimmed === SELECTION_CLOSE) {
      inSelection = false;
      continue;
    }
    if (inSelection) {
      selectionChars += line.length;
      continue;
    }
    if (trimmed === SELECTION_LEAD_IN) continue;
    if (trimmed === VOICE_INTERRUPTED_MARKER) continue;
    if (trimmed === IMAGE_MARKER) continue;
    if (trimmed === SCREENSHOT_MARKER) {
      screenshot = true;
      continue;
    }
    if (trimmed.startsWith(FILE_MARKER_PREFIX) && trimmed.endsWith("]")) {
      files.push(trimmed.slice(FILE_MARKER_PREFIX.length, -1).trim());
      continue;
    }
    kept.push(
      trimmed.startsWith(SELECTION_REQUEST_PREFIX)
        ? trimmed.slice(SELECTION_REQUEST_PREFIX.length).trimStart()
        : line,
    );
  }
  return { text: kept.join("\n").trim(), screenshot, files, selectionChars };
};
