import type { ConversationPace } from "@/bindings";

export interface VoiceTicket {
  session: number;
  turn: number;
}
/** Backend context marker; never render it as part of an answer. */
export const VOICE_INTERRUPTED_MARKER =
  "[Voice reply interrupted; some of this answer may not have been heard.]";
/**
 * The pace is a persisted setting (`assistant_conversation_pace`), so the union
 * comes from the generated bindings rather than being restated here — the pause
 * lengths below are the only part of it the frontend owns.
 */
export type { ConversationPace };
export const CONVERSATION_PACES: readonly ConversationPace[] = [
  "quick",
  "natural",
  "patient",
];
export const DEFAULT_CONVERSATION_PACE: ConversationPace = "natural";
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

/**
 * Find the browser device that corresponds to a device name stored by the
 * native audio engine.
 *
 * The two sides enumerate independently: settings hold a `cpal` device name,
 * while the panel sees `MediaDeviceInfo.label`. They agree often enough on
 * Windows to look like the same string and essentially never on Linux, where
 * cpal reports `sysdefault:CARD=PCH` for something Chromium calls "Built-in
 * Audio Analog Stereo". Chromium also decorates duplicates ("2 - Yeti") and
 * prefixes the default ("Default - Yeti"), so an exact comparison is the wrong
 * test even when both sides mean the same hardware.
 *
 * Matching is therefore progressively looser and gives up rather than guessing
 * between two equally good candidates. `null` means "use the system default",
 * never "fail".
 */
export function matchDeviceByName(
  devices: MediaDeviceInfo[],
  kind: MediaDeviceKind,
  wanted: string,
): MediaDeviceInfo | null {
  const target = wanted.trim().toLowerCase();
  if (!target) return null;
  // A label is the empty string until microphone permission is granted, and an
  // empty label can never identify anything.
  const candidates = devices.filter((d) => d.kind === kind && d.label.trim());
  const exact = candidates.filter(
    (d) => d.label.trim().toLowerCase() === target,
  );
  if (exact.length) return exact[0];
  const contains = candidates.filter((d) => {
    const label = d.label.trim().toLowerCase();
    return label.includes(target) || target.includes(label);
  });
  // Exactly one plausible reading, or none: anything else is a guess.
  return contains.length === 1 ? contains[0] : null;
}
