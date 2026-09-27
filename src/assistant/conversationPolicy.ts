import type { ConversationPace, ConversationSensitivity } from "@/bindings";

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

export type { ConversationSensitivity };
export const CONVERSATION_SENSITIVITIES: readonly ConversationSensitivity[] = [
  "low",
  "normal",
  "high",
];
export const DEFAULT_CONVERSATION_SENSITIVITY: ConversationSensitivity =
  "normal";

/**
 * How each sensitivity level tunes the voice detector.
 *
 * The call used to run one fixed tuning — 0.65 to start, 160 ms of speech — which
 * is a quiet-room, headphones tuning. In a real room a keyboard, a chair or a
 * cough crossed it, and so did the assistant's own voice coming back through the
 * speakers, which the echo canceller only partly removes. Every one of those
 * started a turn, and a turn that starts while the assistant is talking cuts the
 * answer off: the call felt like it reacted to everything and never finished a
 * sentence.
 *
 * * `positiveSpeechThreshold` / `negativeSpeechThreshold` — the Silero speech
 *   probability a frame must reach to count as speech, and fall below to count as
 *   silence. Higher ignores more noise.
 * * `minSpeechMs` — how much speech a sound must contain before it is an
 *   utterance at all. Clicks and coughs are shorter than this.
 * * `bargeInMs` — how long speech must stay dense *while the assistant is
 *   talking* before it is allowed to interrupt. This is the echo guard: the
 *   assistant's own voice leaking into the microphone arrives in fragments, and
 *   a person who means to interrupt keeps talking. Speech that never gets there
 *   is dropped instead of cutting the reply off. Only applies while audio is
 *   actually playing; speech into silence starts a turn straight away.
 */
export const VAD_SENSITIVITY: Record<
  ConversationSensitivity,
  {
    positiveSpeechThreshold: number;
    negativeSpeechThreshold: number;
    minSpeechMs: number;
    bargeInMs: number;
  }
> = {
  low: {
    positiveSpeechThreshold: 0.85,
    negativeSpeechThreshold: 0.6,
    minSpeechMs: 400,
    bargeInMs: 800,
  },
  normal: {
    positiveSpeechThreshold: 0.75,
    negativeSpeechThreshold: 0.5,
    minSpeechMs: 260,
    bargeInMs: 450,
  },
  high: {
    positiveSpeechThreshold: 0.6,
    negativeSpeechThreshold: 0.4,
    minSpeechMs: 160,
    bargeInMs: 250,
  },
};

/** One Silero v5 frame: 512 samples at 16 kHz. */
export const VAD_FRAME_MS = 32;

/**
 * How far back before the detector's decision an utterance really began.
 *
 * # Why the first words went missing
 *
 * The detector does not know an utterance has started until one frame's speech
 * probability crosses `positiveSpeechThreshold` — 0.75 on Normal. Speech does not
 * arrive at 0.75. A first word spoken softly, or from arm's length, or a word that
 * opens on a fricative ("so", "sure", "hey") scores 0.2–0.6 for its whole length,
 * and the frame that finally crosses the line is often the second or third word.
 * Everything before that frame survived only if it fitted in a fixed 320 ms
 * pre-roll, so "hey, can you check this" reached transcription as "check this".
 * High "fixed" it only by crossing the line earlier, at the price of starting
 * turns on the keyboard, a chair, and the assistant's own voice.
 *
 * The two questions were one knob, and they should not be. Whether a sound is an
 * utterance is the threshold's job and stays exactly as strict as it was. Where
 * the utterance *begins* is answered afterwards, by looking back from the frame
 * that crossed: the lead-in reaches the earliest frame still plausibly speech
 * (`onsetProbability`), stepping over the short pauses between words (`maxGapMs`)
 * but not over a real silence, plus a small `marginMs` because the probability
 * rises a frame or two after the sound does. It never takes less than the old
 * fixed pre-roll (`minMs`), so no utterance gets a shorter start than it used to,
 * and never more than `maxMs`.
 */
export const SPEECH_LEAD = {
  minMs: 320,
  maxMs: 1600,
  onsetProbability: 0.2,
  maxGapMs: 320,
  marginMs: 128,
} as const;

export type SpeechLead = { [K in keyof typeof SPEECH_LEAD]: number };

/**
 * How many of the frames before a segment's first frame belong to it.
 *
 * `history` is every frame the detector saw before that first frame, oldest
 * first, as its speech probability — or `null` for a frame heard while the
 * assistant's own reply was playing. The look-back never extends past one of
 * those beyond `minMs`: a leaked reply scores as speech too, and reaching into
 * it would put the assistant's words at the start of the user's turn.
 *
 * Pure, so the rule is testable without a microphone.
 */
export function speechLeadFrames(
  history: readonly (number | null)[],
  lead: SpeechLead = SPEECH_LEAD,
  frameMs: number = VAD_FRAME_MS,
): number {
  const n = history.length;
  const frames = (ms: number) => Math.max(0, Math.round(ms / frameMs));
  const min = Math.min(n, frames(lead.minMs));
  const max = Math.min(n, frames(lead.maxMs));
  const maxGap = frames(lead.maxGapMs);

  let earliest = n;
  let reachable = max;
  let gap = 0;
  for (let i = n - 1; i >= n - max; i--) {
    const probability = history[i];
    if (probability === null) {
      reachable = n - 1 - i;
      break;
    }
    if (probability >= lead.onsetProbability) {
      earliest = i;
      gap = 0;
    } else if (++gap > maxGap) {
      break;
    }
  }

  const reach = earliest < n ? n - earliest + frames(lead.marginMs) : 0;
  return Math.max(min, Math.min(reach, reachable));
}

/** Frames the lead-in can ever need, plus the frame that started the segment. */
export const SPEECH_LEAD_HISTORY_FRAMES =
  Math.ceil(SPEECH_LEAD.maxMs / VAD_FRAME_MS) + 1;

/**
 * Share of frames in the `bargeInMs` window that must be speech before speech
 * over a playing reply may interrupt it. Residual echo is patchy and stays
 * well under this; someone talking over the assistant sits well above it.
 */
export const BARGE_IN_DENSITY = 0.6;
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
