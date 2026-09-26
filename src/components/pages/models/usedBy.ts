import type { ModelSlot } from "@/components/shell/navigation";

/** Which features lean on each job's model, as `modelsHub.usedBy.*` keys. */
export const USED_BY_KEYS: Record<ModelSlot, string[]> = {
  stt: ["dictation", "assistant", "meetings"],
  cleanup: ["cleanup", "meetingNotes"],
  assistant: ["assistant", "calls", "flow"],
  voice: ["spokenReplies", "calls"],
};
