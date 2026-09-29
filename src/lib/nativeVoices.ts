/**
 * The voices that run natively on this computer (`src-tauri/src/native_tts.rs`):
 * Kitten in three sizes, Kyutai's Pocket TTS, and Supertone's Supertonic 3.
 * Each is an ordinary catalog download to the model manager; this file maps an
 * engine to the pack it speaks with and lists its voices, mirroring
 * `native_tts::pack_for_engine` and `native_tts::voice_names`.
 */

/** Catalog id of Kokoro's processor pack (not a native-only engine: Kokoro
 *  normally speaks inside the WebView). */
export const KOKORO_NATIVE_MODEL_ID = "kokoro-82m-native";

export type KittenSize = "nano" | "micro" | "mini";

/** Kitten's sizes, smallest first. The chosen one is stored as the engine's
 *  model setting (`assistant_tts_model`), as its catalog id. */
export const KITTEN_SIZES: readonly { size: KittenSize; modelId: string }[] = [
  { size: "nano", modelId: "kitten-nano-0.8" },
  { size: "micro", modelId: "kitten-micro-0.8" },
  { size: "mini", modelId: "kitten-mini-0.8" },
];

export const POCKET_MODEL_ID = "pocket-tts";
export const SUPERTONIC_MODEL_ID = "supertonic-3-int8";

/** Engine ids that only ever speak natively. */
export const NATIVE_ENGINES = ["kitten", "pocket", "supertonic"] as const;
export type NativeEngine = (typeof NATIVE_ENGINES)[number];

export const isNativeEngine = (engine: string): engine is NativeEngine =>
  (NATIVE_ENGINES as readonly string[]).includes(engine);

/** The Kitten size a model setting names. Anything else is nano, the size
 *  every earlier version installed (mirrors `native_tts::kitten_pack`). */
export const kittenSizeFor = (model: string | null | undefined): KittenSize => {
  const value = (model ?? "").trim().toLowerCase();
  return (
    KITTEN_SIZES.find(
      (entry) =>
        entry.modelId === value || `kitten-${value}-0.8` === entry.modelId,
    )?.size ?? "nano"
  );
};

/** Catalog id of the pack an engine speaks with, or null for any engine
 *  that is not native-only. */
export const nativePackId = (
  engine: string,
  model: string | null | undefined,
): string | null => {
  switch (engine) {
    case "kitten": {
      const size = kittenSizeFor(model);
      return KITTEN_SIZES.find((entry) => entry.size === size)!.modelId;
    }
    case "pocket":
      return POCKET_MODEL_ID;
    case "supertonic":
      return SUPERTONIC_MODEL_ID;
    default:
      return null;
  }
};

export interface NativeVoice {
  id: string;
  female: boolean;
  /** Supertonic's speakers have numbers rather than names. */
  numbered?: number;
}

const named = (female: boolean, ...ids: string[]): NativeVoice[] =>
  ids.map((id) => ({ id, female }));

/** Voices per engine, women first. Kitten's names are its authors' own;
 *  Pocket's are Kyutai's preset names, best-scoring first, without the five
 *  whose recordings made them sound bad or mislabelled (see
 *  `native_tts::POCKET_VOICES`). */
const VOICES: Record<NativeEngine, readonly NativeVoice[]> = {
  kitten: [
    ...named(true, "Bella", "Luna", "Rosie", "Kiki"),
    ...named(false, "Jasper", "Bruno", "Hugo", "Leo"),
  ],
  pocket: [
    ...named(
      true,
      "Mary",
      "Caro",
      "Azelma",
      "Vera",
      "Eve",
      "Anna",
      "Jane",
      "Fantine",
      "Eponine",
    ),
    ...named(false, "George", "Bill", "Peter", "Stuart", "Paul"),
  ],
  supertonic: [true, false].flatMap((female) =>
    [1, 2, 3, 4, 5].map((n) => ({
      id: `${female ? "F" : "M"}${n}`,
      female,
      numbered: n,
    })),
  ),
};

export const nativeVoices = (engine: NativeEngine): readonly NativeVoice[] =>
  VOICES[engine];

/** The voice the engine speaks with when none (or an unknown one) is set,
 *  matching each engine's `default_voice` in `tts.rs`. */
export const DEFAULT_NATIVE_VOICE: Record<NativeEngine, string> = {
  kitten: "Bella",
  pocket: "Mary",
  supertonic: "F1",
};

/** The stored voice if this engine knows it (case-insensitively, as the
 *  backend matches it), otherwise the engine's default. */
export const effectiveNativeVoice = (
  engine: NativeEngine,
  stored: string | null | undefined,
): string => {
  const value = (stored ?? "").trim().toLowerCase();
  return (
    VOICES[engine].find((voice) => voice.id.toLowerCase() === value)?.id ??
    DEFAULT_NATIVE_VOICE[engine]
  );
};
