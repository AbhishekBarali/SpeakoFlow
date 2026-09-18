import type { SettingTone } from "@/components/ui/tones";
import type {
  MeetingSegment,
  MeetingSpeaker,
  SegmentEvent,
  SpeakerSource,
} from "./api";

/** Speaker key the microphone stream carries (`SpeakerSource::Mic`). */
export const ME_SPEAKER_KEY = "me";
/** Placeholder key covering everyone on the system stream before diarization. */
export const OTHERS_SPEAKER_KEY = "them";

/**
 * The names `MeetingStore::create_meeting` seeds a meeting with.
 *
 * They are English, written in Rust before any locale is known. Treating a
 * display name that still equals its seeded value as "not yet named" is what
 * lets the UI show a translated label without a second column in the schema —
 * and the moment the user renames a speaker, their own words win.
 */
const SEEDED_NAMES: Readonly<Record<string, string>> = {
  [ME_SPEAKER_KEY]: "You",
  [OTHERS_SPEAKER_KEY]: "Others",
};

/** True while a speaker still carries the name the backend seeded. */
export const hasSeededName = (speaker: MeetingSpeaker): boolean =>
  SEEDED_NAMES[speaker.speaker_key] === speaker.display_name;

/**
 * Below this the diarization label is a guess, and the UI says so.
 *
 * Confidence is a cosine similarity in [-1, 1]. A confidently wrong speaker
 * name is worse than a visibly uncertain one, which is why this de-emphasises
 * rather than hides.
 */
export const LOW_CONFIDENCE = 0.4;

/** Brand teal is reserved for the user; everyone else cycles the rest. */
const OTHER_TONES: readonly SettingTone[] = [
  "violet",
  "amber",
  "sky",
  "rose",
  "indigo",
  "emerald",
] as const;

/**
 * A stable colour per speaker for one meeting.
 *
 * `others` is the list of non-user speaker keys in the order they should be
 * coloured, so Speaker 1 keeps its colour for the whole transcript instead of
 * changing as more pages load.
 */
export const speakerTone = (
  speakerKey: string,
  others: readonly string[],
): SettingTone => {
  if (speakerKey === ME_SPEAKER_KEY) return "teal";
  const position = others.indexOf(speakerKey);
  return OTHER_TONES[(position < 0 ? 0 : position) % OTHER_TONES.length];
};

/** One utterance, from either a stored segment or a live event. */
export interface TranscriptItem {
  /** Stable React key. Live items have no row id yet, hence the composite. */
  key: string;
  source: SpeakerSource;
  speakerKey: string;
  startMs: number;
  endMs: number;
  text: string;
  confidence: number | null;
}

/** A stored segment as a transcript item. */
export const itemFromSegment = (segment: MeetingSegment): TranscriptItem => ({
  key: `s${segment.id}`,
  source: segment.source,
  // Never inferred: the source always supplies a default key, and falling back
  // to the source's own default keeps a cleared label on the right side.
  speakerKey: segment.speaker_key ?? defaultSpeakerKey(segment.source),
  startMs: segment.start_ms,
  endMs: segment.end_ms,
  text: segment.text,
  confidence: segment.confidence,
});

/** A live `meeting-segment` payload as a transcript item. */
export const itemFromEvent = (event: SegmentEvent): TranscriptItem => ({
  key: `l${event.meeting_id}-${event.source}-${event.start_ms}-${event.end_ms}`,
  source: event.source,
  speakerKey: event.speaker_key,
  startMs: event.start_ms,
  endMs: event.end_ms,
  text: event.text,
  confidence: null,
});

/** The channel-derived key for a stream, mirroring `default_speaker_key`. */
export const defaultSpeakerKey = (source: SpeakerSource): string =>
  source === "mic" ? ME_SPEAKER_KEY : OTHERS_SPEAKER_KEY;

/** One speaker turn: consecutive utterances from the same person. */
export interface SpeakerTurn {
  key: string;
  speakerKey: string;
  source: SpeakerSource;
  startMs: number;
  endMs: number;
  text: string;
  /** The least confident part of the turn — the honest summary of the whole. */
  confidence: number | null;
  lowConfidence: boolean;
}

/**
 * Collapse items into turns.
 *
 * The chunker cuts on silence and at a 30-second cap, so one person talking for
 * two minutes arrives as five or six items. Rendering those as five labelled
 * bubbles reads as five turns in a conversation that only had one.
 */
export const groupIntoTurns = (
  items: readonly TranscriptItem[],
): SpeakerTurn[] => {
  const turns: SpeakerTurn[] = [];

  for (const item of items) {
    const text = item.text.trim();
    // A chunk of breath the transcriber answered with nothing. A bare speaker
    // label with no words is pure noise.
    if (!text) continue;

    const last = turns[turns.length - 1];
    if (last && last.speakerKey === item.speakerKey) {
      last.text = `${last.text} ${text}`;
      last.endMs = Math.max(last.endMs, item.endMs);
      last.confidence = leastConfident(last.confidence, item.confidence);
      last.lowConfidence = isLowConfidence(last.confidence);
      continue;
    }

    turns.push({
      key: item.key,
      speakerKey: item.speakerKey,
      source: item.source,
      startMs: item.startMs,
      endMs: item.endMs,
      text,
      confidence: item.confidence,
      lowConfidence: isLowConfidence(item.confidence),
    });
  }

  return turns;
};

const isLowConfidence = (confidence: number | null): boolean =>
  confidence !== null && confidence < LOW_CONFIDENCE;

const leastConfident = (a: number | null, b: number | null): number | null => {
  if (a === null) return b;
  if (b === null) return a;
  return Math.min(a, b);
};

/**
 * Non-user speaker keys in first-appearance order, so colours are assigned by
 * who spoke first rather than by how SQLite happened to sort the keys.
 */
export const otherSpeakerOrder = (
  turns: readonly SpeakerTurn[],
  speakers: readonly MeetingSpeaker[],
): string[] => {
  const order: string[] = [];
  for (const turn of turns) {
    if (turn.speakerKey === ME_SPEAKER_KEY) continue;
    if (!order.includes(turn.speakerKey)) order.push(turn.speakerKey);
  }
  // Speakers that exist but never spoke still need a colour for their rename
  // row, and a meeting that produced no transcript has only these.
  for (const speaker of speakers) {
    if (speaker.is_me || speaker.speaker_key === ME_SPEAKER_KEY) continue;
    if (!order.includes(speaker.speaker_key)) order.push(speaker.speaker_key);
  }
  return order;
};

/**
 * Localised words for the speaker keys that have no user-given name yet.
 *
 * Passed in rather than read from i18next here, so the resolver below stays a
 * pure function of its inputs and can be unit-tested without mounting an i18n
 * provider.
 */
export interface SpeakerLabels {
  /** The local user. */
  me: string;
  /** The whole far side, before diarization has told anyone apart. */
  others: string;
  /** A diarized voice, by its 1-based appearance order. */
  numbered: (position: number) => string;
}

/**
 * Build the speaker-key → display-name function for one meeting.
 *
 * Shared by the transcript view and the floating pill. The subtlety worth keeping
 * in one place is the three-way fallback:
 *
 * 1. A speaker the user has renamed wins outright — their words beat ours.
 * 2. A speaker still carrying the English name Rust seeded at creation counts as
 *    *not yet named*, so the label comes from i18next. That is what lets a
 *    translated label exist without a second column in the schema.
 * 3. `them` specifically must never render as "Speaker 1", because that would
 *    claim the app had told the far side apart when it has not. Only keys that
 *    came out of a diarization pass get a number, and the number is appearance
 *    order so it is stable as more of the transcript loads.
 *
 * `others` is the non-user speaker order from [`otherSpeakerOrder`].
 */
export const speakerNameResolver = (
  speakers: readonly MeetingSpeaker[],
  others: readonly string[],
  labels: SpeakerLabels,
): ((speakerKey: string) => string) => {
  const byKey = new Map(
    speakers.map((speaker) => [speaker.speaker_key, speaker]),
  );
  // Keys produced by diarization, in appearance order. The pre-diarization
  // placeholder for everyone is not one of them.
  const diarized = others.filter((key) => key !== OTHERS_SPEAKER_KEY);

  return (speakerKey: string): string => {
    const speaker = byKey.get(speakerKey);
    if (speaker && !hasSeededName(speaker)) return speaker.display_name;
    if (speakerKey === ME_SPEAKER_KEY) return labels.me;
    if (speakerKey === OTHERS_SPEAKER_KEY) return labels.others;
    const position = diarized.indexOf(speakerKey);
    return labels.numbered((position < 0 ? 0 : position) + 1);
  };
};

/** `m:ss` under an hour, `h:mm:ss` over it. */
export const formatClock = (totalMs: number): string => {
  const safe = Number.isFinite(totalMs) && totalMs > 0 ? totalMs : 0;
  const totalSeconds = Math.floor(safe / 1000);
  const seconds = totalSeconds % 60;
  const minutes = Math.floor(totalSeconds / 60) % 60;
  const hours = Math.floor(totalSeconds / 3600);
  const pad = (value: number) => String(value).padStart(2, "0");
  return hours > 0
    ? `${hours}:${pad(minutes)}:${pad(seconds)}`
    : `${minutes}:${pad(seconds)}`;
};

/** Recorded length of a meeting, or null while it is still recording. */
export const meetingDurationMs = (
  startedAt: number,
  endedAt: number | null,
): number | null =>
  endedAt === null ? null : Math.max(0, endedAt - startedAt) * 1000;
