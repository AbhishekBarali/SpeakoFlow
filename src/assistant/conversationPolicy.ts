export interface VoiceTicket {
  session: number;
  turn: number;
}
/** Backend context marker; never render it as part of an answer. */
export const VOICE_INTERRUPTED_MARKER =
  "[Voice reply interrupted; some of this answer may not have been heard.]";
export type ConversationPace = "quick" | "natural" | "patient";
export const TURN_PAUSE_MS: Record<ConversationPace, number> = {
  quick: 450,
  natural: 700,
  patient: 1100,
};
export const MAX_UTTERANCE_MS = 60_000;
export function sameVoiceTicket(
  current: VoiceTicket | null,
  incoming: VoiceTicket,
): boolean {
  return (
    current?.session === incoming.session && current.turn === incoming.turn
  );
}

/** A local generation changes synchronously, before cancellation crosses IPC. */
export class VoiceTurnGate {
  private generation = 0;
  next(): number {
    return ++this.generation;
  }
  accepts(generation: number): boolean {
    return this.generation === generation;
  }
}
