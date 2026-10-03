/**
 * What first-run setup suggests downloading, decided from the machine it runs
 * on.
 *
 * Pure on purpose: every rule here is a guess about someone's hardware, and a
 * guess that turns out wrong should be a failing test, not a screenshot. The
 * component only gathers facts (`useHardwareFacts`) and renders what this
 * returns; the user can override every choice before anything downloads.
 */

import { KITTEN_SIZES, KOKORO_NATIVE_MODEL_ID } from "@/lib/nativeVoices";

/** Kitten's middle size: the smallest (44 MB), and still a clear voice. */
const KITTEN_MICRO_ID =
  KITTEN_SIZES.find((entry) => entry.size === "micro")?.modelId ??
  "kitten-micro-0.8";

export type SetupJob = "stt" | "cleanup" | "voice" | "assistant";

/** Dictation arrives first. A call's assistant must be ready before its voice. */
export const JOB_ORDER: readonly SetupJob[] = [
  "stt",
  "cleanup",
  "assistant",
  "voice",
];

export type SpeechChoice = "english" | "multilingual";
export type AssistantChoice = "quick" | "balanced";

/**
 * Where the voice runs:
 * - `webview`: Kokoro inside the app's web view, on the graphics card.
 * - `native`: Kokoro on every processor core, for a web view without WebGPU.
 * - `light`: Kitten micro, a much smaller voice for a small processor.
 * - `slow`: Kokoro on a single web-view thread, the only option left when
 *   neither the graphics card nor the native engine is available. It works,
 *   at about ten seconds a reply, so it is offered but not switched on.
 */
export type VoiceRoute = "webview" | "native" | "light" | "slow";

/** Why the assistant size was suggested, for the one-line reason under it. */
export type AssistantReason =
  | "gpu"
  | "unified"
  | "memory"
  | "tight"
  | "unknown";

/** Catalog ids, as the model manager exposes them. */
export const SETUP_MODELS = {
  speech: {
    english: "parakeet-unified-en-0.6b-gguf",
    multilingual: "nemotron-3.5-asr-streaming-0.6b-gguf",
  },
  cleanup: "speakoflow-mini",
  assistant: {
    quick: "gemma-4-e2b",
    balanced: "gemma-4-e4b",
  },
  voice: {
    native: KOKORO_NATIVE_MODEL_ID,
    light: KITTEN_MICRO_ID,
  },
} as const;

/**
 * The speech models setup offers, in the order shown, and the only ones it
 * offers. Setup is a short list of good choices, not the catalog: the Models
 * page has all of them. It used to append every other installed speech model
 * as well, and the catalog holds several near-namesakes (an ONNX and a GGUF
 * Canary 180M Flash, two Whisper Mediums, Parakeet V3 beside Parakeet 0.6B),
 * so the list read as the same model offered twice.
 *
 * `name` is one plain name that says which model this is ("Whisper Medium",
 * not "Whisper" plus a "Medium" label), because a catalog name ("Cohere
 * Transcribe 03-2026") is not what anyone calls it. Names are brands, not
 * translated. `about` keys the card's one sentence of why you would pick it.
 */
export const SETUP_SPEECH_OPTIONS: ReadonlyArray<{
  id: string;
  name: string;
  /** `onboarding.speech.about.*` */
  about: "english" | "languages" | "small" | "accurate" | "widest";
}> = [
  { id: SETUP_MODELS.speech.english, name: "Parakeet 0.6B", about: "english" },
  {
    id: SETUP_MODELS.speech.multilingual,
    name: "Nemotron 3.5",
    about: "languages",
  },
  { id: "canary-180m-flash-gguf", name: "Canary 180M Flash", about: "small" },
  {
    id: "cohere-transcribe-03-2026-gguf",
    name: "Cohere Transcribe",
    about: "accurate",
  },
  { id: "whisper-medium-gguf", name: "Whisper Medium", about: "widest" },
];

/**
 * The setup cards that carry a "Recommended" badge: Parakeet for English and
 * Nemotron for everything else. Both, whichever is preselected, because they
 * are the two equally good starting points and the language beside each name
 * says which one is for whom. Badging only the preselected one made Nemotron
 * look like a lesser choice to everyone on an English system.
 */
export const RECOMMENDED_SETUP_SPEECH: ReadonlySet<string> = new Set([
  SETUP_MODELS.speech.english,
  SETUP_MODELS.speech.multilingual,
]);

/** Distinct languages in a model's list: `en-US` and `en-GB` are one. */
export function baseLanguages(codes: ReadonlyArray<string>): string[] {
  return [
    ...new Set(
      codes
        .map((code) => code.trim().split(/[-_]/)[0].toLowerCase())
        .filter(Boolean),
    ),
  ];
}

/**
 * Language names for a model's language codes, in the UI language, sorted so
 * the ones most people look for come first. `Intl.DisplayNames` is in every
 * web view the app ships on; a code it does not know is shown as-is rather
 * than dropped.
 */
export function languageNames(
  codes: ReadonlyArray<string>,
  uiLanguage: string,
): string[] {
  let display: Intl.DisplayNames | null = null;
  try {
    display = new Intl.DisplayNames([uiLanguage, "en"], { type: "language" });
  } catch {
    display = null;
  }
  const FIRST = ["en", "es", "fr", "de", "pt", "it", "hi", "zh", "ja", "ko"];
  const rank = (code: string) => {
    const index = FIRST.indexOf(code.toLowerCase());
    return index < 0 ? FIRST.length : index;
  };
  return [...new Set(baseLanguages(codes))]
    .sort((a, b) => rank(a) - rank(b))
    .map((code) => {
      try {
        return display?.of(code) ?? code;
      } catch {
        return code;
      }
    });
}

/**
 * Kokoro in the web view is not a catalog download (kokoro-js fetches it into
 * the web view's cache), so its size is known here rather than from the model
 * list: the fp32 graph used on a graphics card, and the q8 graph otherwise.
 */
export const WEBVIEW_VOICE_MB = { gpu: 326, cpu: 92 } as const;

export interface GpuFact {
  name: string;
  kind: string;
  vramMb: number;
}

export interface HardwareFacts {
  /** Installed memory in GiB; 0 when the backend could not tell. */
  memoryGb: number;
  /** The card that would run a local model, if any. */
  gpu: GpuFact | null;
  /** Logical processors; 0 when unknown. */
  cores: number;
  /** Apple Silicon shares one pool of memory between processor and GPU. */
  unifiedMemory: boolean;
  /** Whether this web view hands out a working WebGPU adapter. */
  webgpu: boolean;
  /** Whether this platform has the native voice engine. */
  nativeVoice: boolean;
  /** The person's system or app language is English. */
  prefersEnglish: boolean;
}

export interface Recommendation {
  speech: SpeechChoice;
  cleanup: boolean;
  assistant: {
    choice: AssistantChoice;
    enabled: boolean;
    reason: AssistantReason;
  };
  voice: { route: VoiceRoute; enabled: boolean };
}

/** A card this big runs Gemma 4 E4B fully on the GPU (8 GB cards report a
 *  little under 8192 MB, hence the margin). */
const LARGE_MODEL_VRAM_MB = 7500;
/** Below this, a 4 GB model plus the app plus the user's own work does not fit
 *  comfortably, and a cloud assistant is the better first experience. */
const LOCAL_ASSISTANT_MIN_GB = 12;
/** Apple Silicon runs the larger model well from this much shared memory. */
const UNIFIED_LARGE_MODEL_GB = 16;
/** Kokoro on four logical cores gets two synthesis threads, which is slow
 *  enough to suggest the lighter voice (as Settings does). */
const SMALL_PROCESSOR_CORES = 4;

/** Pick the device a local model would run on: a dedicated card before an
 *  unknown one before an integrated one, then the most memory. */
export function pickGpu(
  devices: ReadonlyArray<{ name: string; kind: string; total_vram_mb: number }>,
): GpuFact | null {
  const rank = (kind: string) =>
    kind === "dedicated" ? 2 : kind === "unknown" ? 1 : 0;
  let best: (typeof devices)[number] | null = null;
  for (const device of devices) {
    if (
      !best ||
      rank(device.kind) > rank(best.kind) ||
      (rank(device.kind) === rank(best.kind) &&
        device.total_vram_mb > best.total_vram_mb)
    ) {
      best = device;
    }
  }
  return best
    ? { name: best.name, kind: best.kind, vramMb: best.total_vram_mb }
    : null;
}

/** Whether the person's primary locale is English. Only the first entry
 *  counts: WebView2 routinely lists `en-US` second on a non-English system,
 *  and "English appears somewhere" would hand an English-only speech model to
 *  someone who does not dictate in English. */
export function prefersEnglish(languages: ReadonlyArray<string>): boolean {
  const primary = languages.find((lang) => lang.trim().length > 0);
  return !!primary && primary.trim().toLowerCase().startsWith("en");
}

export function recommendAssistant(
  facts: HardwareFacts,
): Recommendation["assistant"] {
  const strongGpu =
    facts.gpu?.kind === "dedicated" && facts.gpu.vramMb >= LARGE_MODEL_VRAM_MB;
  const strongUnified =
    facts.unifiedMemory && facts.memoryGb >= UNIFIED_LARGE_MODEL_GB;
  if (strongGpu || strongUnified) {
    return {
      choice: "balanced",
      enabled: false,
      reason: strongGpu ? "gpu" : "unified",
    };
  }
  if (facts.memoryGb > 0 && facts.memoryGb < LOCAL_ASSISTANT_MIN_GB) {
    return { choice: "quick", enabled: false, reason: "tight" };
  }
  return {
    choice: "quick",
    enabled: false,
    reason: facts.memoryGb > 0 ? "memory" : "unknown",
  };
}

export function recommendVoiceRoute(facts: HardwareFacts): VoiceRoute {
  if (facts.webgpu) return "webview";
  if (!facts.nativeVoice) return "slow";
  if (facts.cores > 0 && facts.cores <= SMALL_PROCESSOR_CORES) return "light";
  return "native";
}

export function recommend(facts: HardwareFacts): Recommendation {
  const assistant = recommendAssistant(facts);
  const route = recommendVoiceRoute(facts);
  return {
    speech: facts.prefersEnglish ? "english" : "multilingual",
    // First-run setup only downloads dictation. Every extra feature is an
    // explicit choice when the user explores it, regardless of hardware.
    cleanup: false,
    assistant,
    voice: { route, enabled: false },
  };
}

/** The catalog id a voice route downloads, or null for the web-view voice. */
export function voiceModelId(route: VoiceRoute): string | null {
  switch (route) {
    case "native":
      return SETUP_MODELS.voice.native;
    case "light":
      return SETUP_MODELS.voice.light;
    default:
      return null;
  }
}

/** The engine settings a voice route leaves behind once it is ready. */
export function voiceEngineFor(route: VoiceRoute): {
  engine: string;
  model: string | null;
} {
  return route === "light"
    ? { engine: "kitten", model: SETUP_MODELS.voice.light }
    : { engine: "kokoro", model: null };
}
