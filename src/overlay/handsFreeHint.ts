/**
 * The hint a held dictation's pill shows the first few times: the hands-free
 * key on a keycap, so someone holding Ctrl + Win learns that one more key lets
 * them let go. It is how the feature is found, because nobody reads a settings
 * row about a gesture before they need it.
 *
 * It stops for good once the user has gone hands-free from a hold, which is
 * the proof they know, or after `HINT_TIMES` showings, so it never becomes
 * furniture. Kept in `localStorage`, which every window of the app shares.
 */

const KEY = "speakoflow.overlay.handsFreeHint";

/** How many held dictations show it, at most. */
export const HINT_TIMES = 3;
/** How long a dictation has to be held first, so a quick one never flashes
 *  it, and a hold that turns hands-free at once never needs it. */
export const HINT_DELAY_MS = 900;
/** How long it stays before the waveform comes back. */
export const HINT_SHOW_MS = 3500;

export type HintRecord = { shown: number; learned: boolean };

type Storage = Pick<globalThis.Storage, "getItem" | "setItem">;

const store = (): Storage | null => {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
};

export function readHint(storage: Storage | null = store()): HintRecord {
  try {
    const value = JSON.parse(storage?.getItem(KEY) ?? "null") as unknown;
    if (value && typeof value === "object") {
      const { shown, learned } = value as Partial<HintRecord>;
      return {
        shown: typeof shown === "number" && shown >= 0 ? shown : 0,
        learned: learned === true,
      };
    }
  } catch {
    // A damaged value starts over.
  }
  return { shown: 0, learned: false };
}

const write = (record: HintRecord, storage: Storage | null) => {
  try {
    storage?.setItem(KEY, JSON.stringify(record));
  } catch {
    // Not remembered, then: it shows again next time, which is harmless.
  }
};

/** Whether the next held dictation should show it. */
export const hintDue = (record: HintRecord): boolean =>
  !record.learned && record.shown < HINT_TIMES;

/** It was shown once more. */
export function markHintShown(storage: Storage | null = store()) {
  const record = readHint(storage);
  write({ ...record, shown: record.shown + 1 }, storage);
}

/** A held dictation went hands-free: the user knows, so it never shows
 *  again. */
export function markHintLearned(storage: Storage | null = store()) {
  write({ ...readHint(storage), learned: true }, storage);
}
