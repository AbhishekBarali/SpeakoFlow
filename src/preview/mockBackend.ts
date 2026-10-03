/*
 * Browser preview of the main window against a fake backend.
 *
 * Dev-only: not a Vite build input, never shipped. It exists so the UI can be
 * looked at — and screenshotted headlessly — without building the Rust side.
 * Every command the main window calls on load answers with plausible data;
 * the handful of setters that change what a page shows update the fake state,
 * so switching a model or a mode in the preview behaves like the app.
 *
 * URL knobs: ?page=home|history|assistant|meetings|cleanup|dictionary|models
 *            &tab=stt|cleanup|assistant|voice   &settings=<tab>
 *            &stt=device|cloud   &theme=light|dark   &lang=<locale>
 *            &fresh=1 (no history)
 *            &readme=1 (the README screenshots: everyday dictations, a
 *                       generic meeting, Kokoro already downloaded)
 *            &memory=on (assistant memory switched on)
 *            &voice=<engine id> (the voice engine in use; default elevenlabs)
 *            &update=1 (a newer version is available)   &feedback=1 (dialog open)
 *            &new=1 or &installed=0 (nothing installed or configured)
 *            &installed=1 (existing setup when previewing onboarding)
 *            &failDownload=stt|cleanup|assistant|voice|all (comma-separated)
 *            &failOnce=1 (failed downloads succeed on retry) &failLoad=stt
 *            &meeting=live (a meeting recording now) &sysaudio=0 &paused=1
 *            &notes=writing (the newest meeting's notes still being written)
 *            &pill=collapsed|expanded|offer (only in src/preview/meeting.html)
 */
import {
  mockConvertFileSrc,
  mockIPC,
  mockWindows,
} from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";

const params = new URLSearchParams(window.location.search);
const newInstall =
  params.get("new") === "1" ||
  params.get("installed") === "0" ||
  (!!params.get("onboarding") && params.get("installed") !== "1");
const fresh = params.get("fresh") === "1" || newInstall;
const readme = params.get("readme") === "1" && !fresh;
const voiceEngine =
  params.get("voice") ?? (newInstall ? "kokoro" : "elevenlabs");
const onEleven = voiceEngine === "elevenlabs";

// `?readme=1`: Kokoro's weights "in the cache", which is how the voice page
// decides the model is on this device. Settles long before that page mounts.
if (readme && typeof caches !== "undefined") {
  void caches
    .open("transformers-cache")
    .then((cache) =>
      cache.put(
        "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/main/onnx/model.onnx",
        new Response(""),
      ),
    )
    .catch(() => {});
}

(window as unknown as Record<string, unknown>).__TAURI_OS_PLUGIN_INTERNALS__ = {
  platform: "windows",
  os_type: "windows",
  family: "windows",
  eol: "\r\n",
  version: "10.0.26100",
  arch: "x86_64",
  exe_extension: "exe",
};

const now = Math.floor(Date.now() / 1000);

type Json = Record<string, unknown>;

/**
 * Six months of plausible, deterministic daily activity: someone who dictates
 * on weekdays, now and then on a Sunday, took two weeks off in early summer,
 * and has slowly started using it more. Deliberately an ordinary user rather
 * than a power user, so the Insights page reads calm: most days sit in the
 * middle of the scale, Saturdays stay empty, and only recent weeks are bright.
 */
const recentDays = (): Json[] => {
  const pad = (value: number) => String(value).padStart(2, "0");
  const today = new Date();
  const days: Json[] = [];
  const span = 182;
  for (let index = 0; index < span; index += 1) {
    const date = new Date(
      today.getFullYear(),
      today.getMonth(),
      today.getDate() - (span - 1 - index),
    );
    const isToday = index === span - 1;
    const weekday = date.getDay();
    // The backend only sends days that had a dictation. Today always has a
    // little, so the streak reads as alive whatever day the preview runs on.
    if (!isToday) {
      if (weekday === 6) continue;
      if (weekday === 0 && index % 3 !== 0) continue;
      if (index >= 52 && index < 66) continue;
      if (index % 17 === 5) continue;
    }
    const ramp = 0.55 + (0.45 * index) / span;
    const wave =
      0.8 + 0.2 * Math.sin(index * 0.9) + 0.12 * Math.sin(index * 2.3);
    let words = Math.round(ramp * 360 * wave);
    // A long-dictation day every few weeks sets the top of the colour scale,
    // so ordinary days read as mid-tone instead of all lighting up.
    if (weekday !== 0 && index % 19 === 7) words = Math.round(words * 2.1);
    if (weekday === 0 || weekday === 6) words = Math.round(words * 0.35);
    if (isToday) words = 186;
    days.push({
      day: `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`,
      dictations: Math.max(1, Math.round(words / 26)),
      words,
      audio_seconds: Math.round(words / 2.3),
    });
  }
  return days;
};

/** `get_usage_stats` derived from `recentDays()`, so the cards, the grid and
 *  the streaks all agree, the way the backend's numbers do. */
const usageStats = (): Json => {
  const days = recentDays() as Array<{
    day: string;
    dictations: number;
    words: number;
    audio_seconds: number;
  }>;
  const sum = (key: "dictations" | "words" | "audio_seconds") =>
    days.reduce((total, day) => total + day[key], 0);
  // Consecutive calendar days, as `history::compute_streaks` counts them.
  const dayNumber = (key: string) => {
    const [y, m, d] = key.split("-").map(Number);
    return Math.round(Date.UTC(y, m - 1, d) / 86_400_000);
  };
  const numbers = days.map((day) => dayNumber(day.day));
  let longest = 0;
  let run = 0;
  numbers.forEach((value, index) => {
    run = index > 0 && value === numbers[index - 1] + 1 ? run + 1 : 1;
    longest = Math.max(longest, run);
  });
  const today = days[days.length - 1];
  return {
    total_dictations: sum("dictations"),
    total_words: sum("words"),
    timed_words: sum("words"),
    total_audio_seconds: sum("audio_seconds"),
    today_words: today.words,
    today_dictations: today.dictations,
    current_streak_days: run,
    longest_streak_days: longest,
    active_days: days.length,
    recent_days: days,
  };
};

const model = (overrides: Json): Json => ({
  description: "",
  filename: `${overrides.id}.gguf`,
  url: "https://example.invalid/model",
  sha256: null,
  size_mb: 500,
  is_downloaded: false,
  is_downloading: false,
  partial_size: 0,
  is_directory: false,
  engine_type: "TranscribeCpp",
  accuracy_score: 0.8,
  speed_score: 0.8,
  supports_translation: false,
  supports_streaming: false,
  is_recommended: false,
  recommended_rank: null,
  supported_languages: ["en"],
  supports_language_selection: false,
  is_custom: false,
  is_cleanup_specialist: false,
  local_path: null,
  local_folder: null,
  ...overrides,
});

const models: Json[] = [
  model({
    id: "parakeet-unified-en-0.6b",
    name: "Parakeet Unified EN 0.6B",
    description: "Blazingly fast, great for everyday dictation. English only.",
    size_mb: 697,
    is_downloaded: true,
    supports_streaming: true,
    is_recommended: true,
    recommended_rank: 1,
    accuracy_score: 0.9,
    speed_score: 0.85,
  }),
  model({
    id: "nemotron-streaming-3.5",
    name: "Nemotron Streaming 3.5",
    description: "Real-time transcription in 28 languages.",
    size_mb: 716,
    is_downloaded: true,
    supports_streaming: true,
    is_recommended: true,
    recommended_rank: 2,
    supported_languages: ["en", "de", "fr", "es", "it"],
    supports_language_selection: true,
  }),
  model({
    id: "canary-180m-flash",
    name: "Canary 180M Flash",
    description: "Tiny and instant. Runs well on any machine.",
    size_mb: 208,
    supports_translation: true,
    supported_languages: ["en", "de", "fr", "es"],
    supports_language_selection: true,
  }),
  model({
    id: "whisper-medium",
    name: "Whisper Medium",
    description: "Good accuracy at a medium speed.",
    engine_type: "Whisper",
    size_mb: 469,
    supports_translation: true,
    supported_languages: ["en", "de", "fr", "es", "ja", "zh"],
    supports_language_selection: true,
  }),
  model({
    id: "speakoflow-mini",
    name: "SpeakoFlow Mini",
    description:
      "Our own cleanup model. Fixes filler, grammar, and spoken edits.",
    engine_type: "LlamaCpp",
    size_mb: 795,
    is_downloaded: true,
    is_cleanup_specialist: true,
  }),
  model({
    id: "gemma-3-1b",
    name: "Gemma 3 1B",
    description: "The smallest general model that can clean up text.",
    engine_type: "LlamaCpp",
    size_mb: 806,
  }),
  model({
    id: "gemma-4-e2b",
    name: "Gemma 4 E2B (Vision)",
    description: "The quickest current Gemma for everyday conversation.",
    engine_type: "LlamaCpp",
    size_mb: 4096,
    is_downloaded: true,
  }),
  model({
    id: "gemma-4-e4b",
    name: "Gemma 4 E4B (Vision)",
    description: "A stronger quality-and-speed balance.",
    engine_type: "LlamaCpp",
    size_mb: 5837,
    is_downloaded: true,
  }),
  model({
    id: "gemma-4-12b",
    name: "Gemma 4 12B (Vision)",
    description: "More capable for nuanced questions, best with a strong GPU.",
    engine_type: "LlamaCpp",
    size_mb: 6861,
  }),
  model({
    id: "custom-qwen3.5-0.8b-q8",
    name: "Qwen3.5 0.8B Q8 0",
    description: "From bartowski/Qwen_Qwen3.5-0.8B-GGUF on Hugging Face.",
    engine_type: "LlamaCpp",
    size_mb: 794,
    is_downloaded: true,
    is_custom: true,
  }),
  // The awkward names real imports arrive with, from the user's own list.
  model({
    id: "local-flowqwen",
    name: "FLOWQwen3.5 0.8B.Q8 0",
    filename: "FLOWQwen3.5-0.8B.Q8_0.gguf",
    description: "C:\\Models\\FLOWQwen3.5-0.8B.Q8_0.gguf",
    engine_type: "LlamaCpp",
    size_mb: 794,
    is_custom: true,
    local_path: "C:\\Models\\FLOWQwen3.5-0.8B.Q8_0.gguf",
  }),
  model({
    id: "hf-qwen-q4km",
    name: "Qwen Qwen3.5 0.8B (Q4_K_M)",
    filename: "Qwen_Qwen3.5-0.8B-Q4_K_M.gguf",
    description:
      "From bartowski/Qwen_Qwen3.5-0.8B-GGUF on Hugging Face. Supports vision.",
    engine_type: "LlamaCpp",
    size_mb: 751,
    is_downloaded: true,
    is_custom: true,
  }),
  model({
    id: "local-qwen-q4km",
    name: "Qwen3.5 0.8B.Q4 K M",
    filename: "Qwen3.5-0.8B.Q4_K_M.gguf",
    description: "D:\\LLM\\Qwen3.5-0.8B.Q4_K_M.gguf",
    engine_type: "LlamaCpp",
    size_mb: 516,
    is_downloaded: true,
    is_custom: true,
    local_path: "D:\\LLM\\Qwen3.5-0.8B.Q4_K_M.gguf",
  }),
  model({
    id: "hf-tiger-gemma",
    name: "Tiger Gemma 9B v3 (Q4_K_M)",
    filename: "Tiger-Gemma-9B-v3-Q4_K_M.gguf",
    description: "From TheDrummer/Tiger-Gemma-9B-v3-GGUF on Hugging Face.",
    engine_type: "LlamaCpp",
    size_mb: 5530,
    is_custom: true,
  }),
  // `?onboarding=…`: a fresh machine's catalog, under the ids first-run setup
  // asks for, with nothing downloaded yet.
  ...(params.get("onboarding") || newInstall
    ? [
        model({
          id: "parakeet-unified-en-0.6b-gguf",
          name: "Parakeet Unified EN 0.6B",
          size_mb: 697,
          supports_streaming: true,
        }),
        model({
          id: "nemotron-3.5-asr-streaming-0.6b-gguf",
          name: "Nemotron 3.5 ASR Streaming 0.6B",
          size_mb: 716,
          supports_streaming: true,
          // As in src-tauri/src/catalog/catalog.json.
          supported_languages: [
            "en",
            "es",
            "fr",
            "it",
            "pt",
            "nl",
            "de",
            "tr",
            "ru",
            "ar",
            "hi",
            "ja",
            "ko",
            "vi",
            "uk",
            "pl",
            "sv",
            "cs",
            "nb",
            "da",
            "bg",
            "fi",
            "hr",
            "sk",
            "zh",
            "hu",
            "ro",
            "et",
          ],
        }),
        // The rest of the catalog's recommended speech models, as setup's
        // "More models" lists them (sizes and languages from catalog.json).
        model({
          id: "canary-180m-flash-gguf",
          name: "Canary 180M Flash",
          size_mb: 208,
          supported_languages: ["en", "de", "fr", "es"],
        }),
        model({
          id: "cohere-transcribe-03-2026-gguf",
          name: "Cohere Transcribe",
          size_mb: 2299,
          supported_languages: [
            "en",
            "de",
            "fr",
            "es",
            "it",
            "pt",
            "nl",
            "pl",
            "ja",
            "ko",
            "zh",
            "ar",
            "vi",
            "el",
          ],
        }),
        model({
          id: "whisper-medium-gguf",
          name: "Whisper Medium",
          size_mb: 793,
          supported_languages: Array.from({ length: 99 }, (_, i) =>
            i === 0 ? "en" : `x${i}`,
          ),
        }),
        model({
          id: "kokoro-82m-native",
          name: "Kokoro 82M (processor)",
          engine_type: "NativeTts",
          size_mb: 350,
        }),
        model({
          id: "kitten-micro-0.8",
          name: "Kitten Micro",
          engine_type: "NativeTts",
          size_mb: 45,
        }),
      ]
    : []),
];

// A clean install has no downloaded models, including imported test fixtures.
if (newInstall) {
  for (const entry of models) {
    entry.is_downloaded = false;
  }
}

const downloadJob = (entry: Json): string =>
  entry.engine_type === "NativeTts"
    ? "voice"
    : entry.is_cleanup_specialist
      ? "cleanup"
      : entry.engine_type === "LlamaCpp"
        ? "assistant"
        : "stt";

const matchesFailure = (name: string, modelId: string, entry: Json): boolean =>
  (params.get(name) ?? "")
    .split(",")
    .some((value) =>
      ["all", modelId, downloadJob(entry)].includes(value.trim()),
    );

const attempts = new Map<string, number>();
const activeDownloads = new Map<
  string,
  { promise: Promise<null>; cancel: () => void }
>();

/** A download that walks its progress bar over a few seconds, like the real
 *  one reports it, then lands. Everything is local; no model URL is fetched. */
const simulateDownload = (modelId: string): Promise<null> => {
  const running = activeDownloads.get(modelId);
  if (running) return running.promise;
  const entry = models.find((m) => m.id === modelId);
  if (!entry) return Promise.reject("Unknown preview model");
  if (entry.is_downloaded) return Promise.resolve(null);
  const attempt = (attempts.get(modelId) ?? 0) + 1;
  attempts.set(modelId, attempt);
  const fail =
    matchesFailure("failDownload", modelId, entry) &&
    (params.get("failOnce") !== "1" || attempt === 1);
  entry.is_downloading = true;
  let cancel = () => {};
  const promise = new Promise<null>((resolve, reject) => {
    const total = Number(entry.size_mb ?? 500) * 1024 * 1024;
    let downloaded = 0;
    const timer = setInterval(() => {
      downloaded = Math.min(total, downloaded + total / 14);
      entry.partial_size = downloaded;
      void emit("model-download-progress", {
        model_id: modelId,
        downloaded,
        total,
        percentage: (downloaded / total) * 100,
      });
      if (fail && downloaded >= total / 2) {
        clearInterval(timer);
        entry.is_downloading = false;
        activeDownloads.delete(modelId);
        const error = "Preview download failed. Try again.";
        void emit("model-download-failed", { model_id: modelId, error });
        reject(error);
      } else if (downloaded >= total) {
        clearInterval(timer);
        entry.is_downloaded = true;
        entry.is_downloading = false;
        entry.partial_size = 0;
        activeDownloads.delete(modelId);
        void emit("model-download-complete", modelId);
        resolve(null);
      }
    }, 220);
    cancel = () => {
      clearInterval(timer);
      entry.is_downloading = false;
      activeDownloads.delete(modelId);
      void emit("model-download-cancelled", modelId);
      reject("Preview download cancelled");
    };
  });
  activeDownloads.set(modelId, { promise, cancel });
  return promise;
};

const binding = (id: string, current: string): Json => ({
  id,
  name: id,
  description: "",
  default_binding: current,
  current_binding: current,
});

const settings: Json = {
  // The Windows defaults from `settings::get_default_settings`.
  bindings: {
    transcribe: binding("transcribe", "ctrl_left+super"),
    transcribe_with_post_process: binding(
      "transcribe_with_post_process",
      "ctrl_left+super+shift",
    ),
    assistant: binding("assistant", "ctrl_left+alt_left"),
    assistant_call: binding("assistant_call", "ctrl_left+alt_left+c"),
    cancel: binding("cancel", "escape"),
  },
  push_to_talk: true,
  audio_feedback: false,
  theme: params.get("theme") ?? "light",
  stt_engine_mode: params.get("stt") === "device" ? "local" : "cloud",
  cloud_stt_provider_id: "elevenlabs",
  cloud_stt_models: {
    elevenlabs: "scribe_v2_realtime",
    openrouter: "microsoft/mai-transcribe-2",
  },
  cloud_stt_api_keys: { openrouter: "sk-preview", elevenlabs: "xi-preview" },
  cloud_stt_base_urls: {},
  cloud_stt_streaming: true,
  cloud_stt_send_custom_words: true,
  cloud_stt_no_verbatim: false,
  selected_language: "auto",
  app_language: params.get("lang") ?? "en",
  translate_to_english: false,
  selected_model: "parakeet-unified-en-0.6b",
  custom_words: fresh
    ? []
    : ["Abhishek Barali", "SpeakoFlow", "firecrawl", "Gintama", "Tauri"],
  post_process_enabled: true,
  post_process_on_dictation: params.get("cleanupShortcut") === "dictation",
  post_process_provider_id:
    params.get("cleanup") === "device" ? "builtin" : "bedrock_mantle",
  post_process_last_cloud_provider_id: "bedrock_mantle",
  post_process_providers: [
    { id: "builtin", label: "On this computer", base_url: "" },
    { id: "openai", label: "OpenAI", base_url: "https://api.openai.com/v1" },
    {
      id: "openrouter",
      label: "OpenRouter",
      base_url: "https://openrouter.ai/api/v1",
    },
    { id: "anthropic", label: "Anthropic", base_url: "" },
    {
      id: "bedrock_mantle",
      label: "AWS Bedrock (Mantle)",
      base_url: "",
    },
    { id: "groq", label: "Groq", base_url: "" },
    { id: "custom", label: "Custom", base_url: "", allow_base_url_edit: true },
  ],
  post_process_api_keys: {
    bedrock_mantle: "key",
    openrouter: "key",
    groq: "key",
  },
  post_process_models: {
    bedrock_mantle: "google.gemma-4-26b-a4b",
    builtin: "speakoflow-mini",
    openrouter: "google/gemma-3-27b-it",
  },
  post_process_prompts: [
    {
      id: "speakoflow_readable",
      name: "Readable (recommended)",
      prompt: "Turn the transcript into readable text.",
    },
    {
      id: "default_improve_transcriptions",
      name: "Improve transcriptions",
      prompt: "Clean up the transcript.",
    },
    {
      id: "speakoflow_mini_cleanup",
      name: "SpeakoFlow Mini",
      prompt: "Clean.",
    },
  ],
  post_process_selected_prompt_id: "default_improve_transcriptions",
  post_process_selected_tone_id: "none",
  post_process_tone: "none",
  post_process_custom_tones: [
    {
      id: "custom-fix",
      name: "Fix",
      instruction:
        "Fix issues and make it clear. Turn spoken emoji names into emoji.",
    },
    {
      id: "custom-natural",
      name: "Natural",
      instruction: "Make it read like someone chatting, lowercase and relaxed.",
    },
  ],
  post_process_timeout_secs: 20,
  assistant_enabled: true,
  assistant_provider_id: params.get("brain") === "device" ? "builtin" : "groq",
  assistant_last_cloud_provider_id: "groq",
  assistant_models: {
    builtin: "gemma-4-e4b",
    groq: "openai/gpt-oss-120b",
    openrouter: "google/gemini-2.5-flash",
  },
  assistant_ask_screen_access: true,
  assistant_call_screen_access: false,
  assistant_web_search_enabled: true,
  assistant_web_search_provider: "tinyfish",
  assistant_tts_enabled: true,
  assistant_tts_engine: voiceEngine,
  assistant_tts_model: onEleven ? "eleven_v3_conversational" : "",
  assistant_tts_api_key: onEleven ? "xi-preview" : "",
  assistant_tts_api_keys: { elevenlabs: "xi-preview" },
  assistant_tts_remote_voice: onEleven ? "JBFqnCBsd6RMkjVDRZzb" : "",
  assistant_tts_remote_voices: { elevenlabs: "JBFqnCBsd6RMkjVDRZzb" },
  assistant_tts_models: { elevenlabs: "eleven_v3_conversational" },
  assistant_tts_base_urls: {},
  assistant_characters: [
    {
      id: "default",
      name: "Assistant",
      builtin: true,
      description: "General help",
    },
    {
      id: "quick",
      name: "Quick",
      builtin: true,
      description: "Short, direct answers",
    },
    {
      id: "companion",
      name: "Companion",
      builtin: true,
      description: "Warm and chatty",
    },
    {
      id: "coach",
      name: "Writing coach",
      builtin: false,
      description: "Tightens drafts",
    },
  ],
  assistant_active_character_id: "default",
  assistant_memory_enabled: params.get("memory") === "on",
  assistant_memory_detail: "detailed",
  assistant_memory: {
    about_you:
      "Product designer at a small software studio. Writes a lot of email and design reviews, and prefers short, direct answers with the main point first.",
    notes: [
      ["Leads design for a team of six.", "high", "user"],
      ["Prefers answers as short bullet points.", "high", "auto"],
      ["Works mostly in Figma and Notion.", "high", "auto"],
      ["Writes client emails in a friendly but brief tone.", "high", "auto"],
      ["Based in Lisbon (UTC+0).", "high", "user"],
      ["Has a weekly design review on Tuesday mornings.", "medium", "auto"],
      ["Prefers metric units.", "medium", "auto"],
      ["Is learning Portuguese.", "medium", "auto"],
      ["Wants reminders phrased as short commands.", "low", "auto"],
      ["Likes examples before theory.", "medium", "auto"],
      ["Avoids meetings after 4 pm.", "low", "auto"],
      ["Keeps separate work and personal calendars.", "medium", "auto"],
    ].map(([text, confidence, source], index) => ({
      id: `note-${index}`,
      text,
      confidence,
      source,
      updated: `2026-09-${String(24 - (index % 20)).padStart(2, "0")}`,
    })),
  },
  assistant_response_length: "default",
  local_llm_context_size: 16384,
  local_llm_unload_timeout: "min5",
  experimental_enabled: false,
  debug_mode: false,
  ui_text_size: "medium",
  external_script_path: null,
  model_folders: [],
  text_replacements: [],
  replacements_enabled: false,
};

if (newInstall) {
  // Optional features have no provider/model selected on first run. Keeping
  // the ordinary preview's fake cloud choices here would hide the real
  // onboarding download buttons and make everything look already enabled.
  Object.assign(settings, {
    stt_engine_mode: params.get("stt") === "cloud" ? "cloud" : "local",
    selected_model: "",
    cloud_stt_api_keys: {},
    assistant_enabled: false,
    assistant_provider_id: "builtin",
    assistant_last_cloud_provider_id: "",
    assistant_models: { builtin: "" },
    assistant_ask_screen_access: false,
    assistant_call_screen_access: false,
    assistant_tts_enabled: false,
    assistant_tts_api_key: "",
    assistant_tts_api_keys: {},
    assistant_tts_remote_voice: "",
    assistant_tts_remote_voices: {},
    assistant_tts_model: "",
    assistant_tts_models: {},
    post_process_enabled: false,
    post_process_provider_id: "builtin",
    post_process_last_cloud_provider_id: "",
    post_process_models: { builtin: "" },
    post_process_api_keys: {},
  });
} else if (params.get("onboarding")) {
  // The ids setup uses are separate from the older main-window fixtures.
  for (const entry of models) {
    if (
      entry.id === "parakeet-unified-en-0.6b-gguf" ||
      entry.id === "nemotron-3.5-asr-streaming-0.6b-gguf"
    ) {
      entry.is_downloaded = true;
    }
  }
}

const cloudProviders: Json[] = [
  {
    id: "elevenlabs",
    label: "ElevenLabs",
    base_url: "https://api.elevenlabs.io",
    kind: "eleven_labs",
    default_model: "scribe_v2",
    models: ["scribe_v2", "scribe_v2_realtime"],
    supports_streaming: true,
  },
  {
    id: "openrouter",
    label: "OpenRouter",
    base_url: "https://openrouter.ai/api/v1",
    kind: "open_ai_compatible",
    default_model: "openai/whisper-large-v3",
    models: ["microsoft/mai-transcribe-2", "openai/whisper-large-v3"],
    supports_streaming: false,
    api_key_url: "https://openrouter.ai/keys",
  },
  {
    id: "openai",
    label: "OpenAI",
    base_url: "https://api.openai.com/v1",
    kind: "open_ai_compatible",
    default_model: "gpt-4o-transcribe",
    supports_translation: true,
  },
  {
    id: "deepgram",
    label: "Deepgram",
    base_url: "https://api.deepgram.com",
    kind: "deepgram",
    default_model: "nova-3",
    supports_streaming: true,
  },
  {
    id: "azure",
    label: "Azure AI Speech",
    base_url: "",
    allow_base_url_edit: true,
    kind: "azure_speech",
    default_model: "MAI-Transcribe-2",
    models: ["MAI-Transcribe-2", "MAI-Transcribe-1.5"],
    honors_keyterms: true,
  },
];

const history = fresh
  ? []
  : (readme
      ? [
          "Thanks for the quick turnaround. I've left two comments on the draft, both small. Happy for it to go out once those are in.",
          "The import fails because the config path is relative. Let's resolve it against the project root and add a test for a folder with a space in its name.",
          "Can you send the invoice to Sara before the call on Friday?",
          "Moving our one-on-one to Thursday at ten works for me.",
          "Picking up coffee and printer paper on the way in tomorrow.",
          "Remind me to review the pull request after lunch.",
        ]
      : [
          "Dit is de planning voor volgende week, and the rest of the update is in English.",
          "The loading spinner hangs on slow connections. Let's add a timeout and show a retry button after ten seconds.",
          "Let's try the smaller model first and compare the results before we switch everything over.",
          "I liked the second layout better. The spacing feels calmer.",
          "Can you send the invoice to Sara before the call on Friday?",
          "Remind me to review the pull request after lunch.",
        ]
    ).map((text, index) => ({
      id: 100 - index,
      file_name: `rec-${index}.wav`,
      timestamp: now - (index + 1) * 5400 - (index > 2 ? 86400 * 2 : 0),
      saved: false,
      title: "",
      transcription_text: text,
      post_processed_text: index % 2 === 1 ? text : null,
      post_process_prompt: null,
      post_process_requested: index % 2 === 1,
    }));

const assistantSessions = fresh
  ? []
  : [
      [
        "What's the fastest way to rename a git branch?",
        "Run `git branch -m new-name` on the branch, then push it with `git push -u origin new-name` and delete the old one on the remote.",
        2400,
      ],
      [
        "Summarize the error on my screen",
        "The build failed because `serde_json` is imported but not in Cargo.toml. Add it under [dependencies] and rebuild.",
        86400 + 3600,
      ],
    ].map(([question, answer, ago], index) => ({
      id: 7 - index,
      timestamp: now - (ago as number) - 600,
      updated_at: now - (ago as number),
      title: question,
      messages: [
        { role: "user", content: question },
        { role: "assistant", content: answer },
      ],
      meeting_id: index === 1 ? 40 : null,
      meeting_title:
        index === 1
          ? readme
            ? "Website relaunch planning"
            : "The meeting was an informal technical discussion"
          : null,
    }));

/* ── Meetings ──
 * `?meeting=live` (or the pill harness) records one right now; `&sysaudio=0`
 * loses the far side; `&paused=1` pauses it; `&notes=writing` opens a meeting
 * whose notes are still being written. */
const liveMeeting =
  params.get("meeting") === "live" || params.has("pill")
    ? params.get("pill") !== "offer"
    : false;
const LIVE_MEETING_ID = 41;

const RELEASE_NOTES = `## Summary

A planning call for the 1.6 release. The team agreed to ship the meetings feature as a beta behind its own switch, and to hold the Linux overlay fix for a patch release so it does not block the date.

## Key takeaways

- Meetings ships in 1.6 as a **beta**, off by default.
- The release date stays on **Friday the 14th**.
- Speaker identification needs a one-time 27 MB download, offered on first use.
- The Linux overlay fix moves to 1.6.1.

## Topics

### Release scope

Priya walked through the open issues. Everything tagged for 1.6 is done except the overlay on GNOME, which needs another round of testing on Wayland.

### Speaker labels

The diarization model is accurate on two or three voices and drifts on larger calls. Labels below a confidence threshold are shown as uncertain rather than hidden.

## Decisions

- Ship meetings as a beta in 1.6 (agreed by everyone).
- Move the GNOME overlay fix to 1.6.1 (Priya).

## Next steps

- [x] **You** — Write the release notes draft (Wednesday)
- [ ] **Priya** — Re-test the overlay on GNOME Wayland (Thursday)
- [ ] **Marco** — Record the meetings demo for the site (before Friday)
`;

/** `?readme=1`: a meeting any reader recognises, rather than one about this
 *  app's own release. */
const RELAUNCH_NOTES = `## Summary

A planning call for the website relaunch. The team kept the launch on Friday the 14th and agreed to ship the new pricing page a week later, once legal has reviewed the refund wording.

## Key takeaways

- The relaunch stays on **Friday the 14th**.
- The new pricing page follows **a week later**.
- All 140 blog posts are migrated, and redirects are tested for the top fifty.
- Old URLs with no traffic go to the archive page instead of a 404.

## Topics

### Launch scope

Priya walked through the tracker. Everything for launch is done except the pricing page, whose copy needs a rewrite and a legal review.

### Redirects

Marco moved all 140 posts. The fifty most-visited URLs are tested; the rest fall back to the archive page.

## Decisions

- Launch on the 14th without the new pricing page (agreed by everyone).
- Ship the pricing page a week later, after legal review (Priya).

## Next steps

- [x] **You** — Draft the launch announcement (Wednesday)
- [ ] **Priya** — Get legal sign-off on the refund wording (Thursday)
- [ ] **Marco** — Test the next hundred redirects (before Friday)
`;

const MEETING_NOTES = readme ? RELAUNCH_NOTES : RELEASE_NOTES;

type Turn = [
  speaker: string,
  source: "mic" | "system",
  at: number,
  text: string,
];
const RELEASE_TURNS: Turn[] = [
  [
    "me",
    "mic",
    4,
    "Okay, I think everyone's here. The main thing today is the 1.6 release and whether meetings goes in.",
  ],
  [
    "spk_1",
    "system",
    12,
    "I went through the board this morning. Everything tagged for 1.6 is closed except the overlay on GNOME.",
  ],
  [
    "spk_1",
    "system",
    21,
    "It works on X11, but on Wayland it still drops behind full-screen windows about one time in five.",
  ],
  [
    "me",
    "mic",
    33,
    "Is that something we can fix this week, or is it a deeper problem?",
  ],
  [
    "spk_1",
    "system",
    40,
    "Honestly, I'd rather not rush it. I want another round on a clean Fedora install before we call it done.",
  ],
  [
    "spk_2",
    "system",
    52,
    "Then let's not hold the release for it. We can ship it in a patch a week later. Nobody on Linux is worse off than today.",
  ],
  [
    "me",
    "mic",
    64,
    "Agreed. So 1.6.1 for the overlay. What about meetings itself — are we comfortable calling it ready?",
  ],
  [
    "spk_2",
    "system",
    75,
    "Ready as a beta, yes. The transcript is solid. Speaker labels are good on two or three people and get shaky on bigger calls.",
  ],
  [
    "spk_1",
    "system",
    88,
    "And the low-confidence ones are marked as uncertain now, so it doesn't claim a name it isn't sure about.",
  ],
  [
    "me",
    "mic",
    99,
    "Good. Then it ships off by default, behind its own switch, with the beta badge. I'll write the release notes.",
  ],
  [
    "spk_2",
    "system",
    110,
    "I can record the demo for the site. Two people, ten minutes, so the labels look right.",
  ],
  [
    "me",
    "mic",
    121,
    "Perfect. Friday the 14th still stands. Thanks, everyone.",
  ],
];

const RELAUNCH_TURNS: Turn[] = [
  [
    "me",
    "mic",
    4,
    "Okay, I think everyone's here. The main thing today is the website relaunch and whether we can still hit the 14th.",
  ],
  [
    "spk_1",
    "system",
    12,
    "I went through the tracker this morning. Everything for launch is done except the new pricing page. The design is signed off, but the copy still reads like a feature list, and legal hasn't seen the refund wording.",
  ],
  [
    "spk_1",
    "system",
    21,
    "Everything else is in. Marco finished the blog migration yesterday.",
  ],
  [
    "me",
    "mic",
    33,
    "Can the pricing page make the 14th, or is that a stretch?",
  ],
  [
    "spk_1",
    "system",
    40,
    "Honestly, I'd rather not rush it. Legal needs a few days with the refund section.",
  ],
  [
    "spk_2",
    "system",
    52,
    "Then let's launch without it and ship pricing a week later. The current page is fine for one more week.",
  ],
  ["me", "mic", 64, "Agreed. Marco, where are we on redirects?"],
  [
    "spk_2",
    "system",
    75,
    "All 140 posts are moved. Redirects are tested for the top fifty URLs by traffic.",
  ],
  [
    "spk_1",
    "system",
    88,
    "And anything with no traffic goes to the archive page instead of a 404.",
  ],
  [
    "me",
    "mic",
    99,
    "Good. Then the 14th stands. I'll draft the launch announcement.",
  ],
  [
    "spk_2",
    "system",
    110,
    "I'll test the next hundred redirects before Friday.",
  ],
  ["me", "mic", 121, "Perfect. Thanks, everyone."],
];

const MEETING_TURNS = readme ? RELAUNCH_TURNS : RELEASE_TURNS;

const segmentsFor = (meetingId: number): Json[] =>
  MEETING_TURNS.map(([speaker, source, at, text], index) => ({
    id: meetingId * 100 + index,
    meeting_id: meetingId,
    source,
    speaker_key: speaker,
    start_ms: at * 1000,
    end_ms: (at + 8) * 1000,
    text,
    // One low-confidence label, so the "uncertain" treatment is on show.
    confidence: speaker === "me" ? null : index === 8 ? 0.31 : 0.82,
  }));

const day = 86400;
const meetings: Json[] = fresh
  ? []
  : (
      [
        [
          readme
            ? "Website relaunch planning"
            : "Planning the 1.6 release and the meetings beta",
          2 * 3600,
          1834,
          "complete",
          true,
        ],
        [
          "Weekly sync with the design team",
          day + 3 * 3600,
          2410,
          "complete",
          true,
        ],
        [
          "Interview: frontend engineer",
          day + 7 * 3600,
          2705,
          "complete",
          false,
        ],
        [
          readme
            ? "Quick check-in with Sara"
            : "A brief technical check focused on verifying the build",
          3 * day,
          61,
          "complete",
          false,
        ],
        [
          readme
            ? "Quarterly budget review"
            : "Reviewing the performance of the new cleanup model",
          4 * day,
          463,
          "interrupted",
          false,
        ],
        ["Customer call with Northwind", 12 * day, 3302, "complete", true],
      ] as const
    ).map(([title, ago, secs, status, notes], index) => ({
      id: 40 - index,
      title,
      started_at: now - ago,
      ended_at: now - ago + secs,
      status,
      mic_file: "mic.wav",
      system_file: index === 3 ? null : "system.wav",
      my_notes:
        index === 0
          ? readme
            ? "Ask Marco for the redirect list.\nCheck when legal can review."
            : "Ask Marco about the demo length.\nCheck the 27 MB figure."
          : "",
      notes:
        notes || index === 0
          ? params.get("notes") === "writing" && index === 0
            ? null
            : MEETING_NOTES
          : null,
      notes_template: "general",
      language: null,
      diarized: index !== 4,
      segment_count: MEETING_TURNS.length,
    }));

if (liveMeeting) {
  const startedAt = now - 754;
  meetings.unshift({
    id: LIVE_MEETING_ID,
    // Composed the way `MeetingsSection.start` composes it.
    title: `Meeting ${new Intl.DateTimeFormat("en", {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date(startedAt * 1000))}`,
    started_at: startedAt,
    ended_at: null,
    status: "recording",
    mic_file: "mic.wav",
    system_file: "system.wav",
    my_notes: "",
    notes: null,
    notes_template: null,
    language: null,
    diarized: false,
    segment_count: 0,
  });
}

const liveState = (): Json => ({
  meeting_id: liveMeeting ? LIVE_MEETING_ID : null,
  status: liveMeeting ? "recording" : "complete",
  paused: params.get("paused") === "1",
  system_audio: params.get("sysaudio") !== "0",
  system_audio_error:
    params.get("sysaudio") === "0"
      ? "No loopback device: the default output is a Bluetooth headset in hands-free mode."
      : null,
  elapsed_ms: liveMeeting ? 754_000 : 0,
  dropped_chunks: 0,
});

const meetingChat: Json[] = fresh
  ? []
  : [
      {
        role: "user",
        content: "What did we decide about the overlay?",
        images: [],
      },
      {
        role: "assistant",
        content:
          "It moves to **1.6.1**. Priya wants another test round on a clean Fedora install, and the team agreed not to hold the release for it, since nobody on Linux is worse off than today.",
        images: [],
      },
      { role: "user", content: "Who owns what?", images: [] },
      {
        role: "assistant",
        content:
          "- **You** — the release notes draft\n- **Priya** — re-testing the overlay on GNOME Wayland\n- **Marco** — recording the meetings demo for the site",
        images: [],
      },
    ];

/** A live call, arriving the way the recorder announces it. */
const startLiveFeed = () => {
  if (!liveMeeting) return;
  const turns = MEETING_TURNS.slice(0, params.has("pill") ? 7 : 5);
  // Spread out like a real call, so a page opened a moment later still sees
  // some of it arrive.
  turns.forEach(([, source, at, text], index) => {
    window.setTimeout(
      () => {
        // Live segments carry the channel key; diarization comes after.
        void emit("meeting-segment", {
          meeting_id: LIVE_MEETING_ID,
          source,
          speaker_key: source === "mic" ? "me" : "them",
          start_ms: at * 1000,
          end_ms: (at + 8) * 1000,
          text,
        });
      },
      350 + index * 700,
    );
  });
  let phase = 0;
  window.setInterval(() => {
    phase += 1;
    void emit("meeting-level", {
      mic: phase % 40 < 14 ? 0.35 + 0.3 * Math.abs(Math.sin(phase)) : 0.01,
      system:
        params.get("sysaudio") === "0"
          ? 0
          : phase % 40 >= 18
            ? 0.3 + 0.4 * Math.abs(Math.sin(phase * 1.3))
            : 0.01,
    });
  }, 90);
};

const readiness = (): Json => {
  const providerId = settings.post_process_provider_id as string;
  const modelsMap = settings.post_process_models as Record<string, string>;
  const providers = settings.post_process_providers as Json[];
  if (!(modelsMap[providerId] ?? "").trim()) {
    return {
      state: "unavailable",
      reason: "no_model_configured",
      source: "dedicated_cleanup_selection",
      provider_id: providerId,
      provider_label:
        providers.find((p) => p.id === providerId)?.label ?? providerId,
    };
  }
  return {
    state: "ready",
    source: "dedicated_cleanup_selection",
    provider_id: providerId,
    provider_label:
      (providers.find((p) => p.id === providerId)?.label as string) ??
      providerId,
    model: modelsMap[providerId] ?? "",
  };
};

const handlers: Record<string, (args: Json) => unknown> = {
  get_app_settings: () => structuredClone(settings),
  get_default_settings: () => structuredClone(settings),
  get_available_models: () => structuredClone(models),
  get_current_model: () => settings.selected_model,
  has_any_models_available: () =>
    models.some(
      (entry) =>
        entry.is_downloaded &&
        (entry.engine_type === "Whisper" ||
          entry.engine_type === "TranscribeCpp"),
    ),
  get_transcription_model_status: () => settings.selected_model,
  get_post_process_readiness: () => readiness(),
  get_cloud_stt_providers: () => structuredClone(cloudProviders),
  get_cloud_stt_key_status: () =>
    cloudProviders.map((p) => [
      p.id,
      !!(settings.cloud_stt_api_keys as Record<string, string>)[p.id as string],
    ]),
  get_history_entries: () => ({ entries: history, has_more: false }),
  get_assistant_history_entries: () => ({
    entries: assistantSessions.map(({ messages, ...summary }) => ({
      ...summary,
      message_count: messages.length,
    })),
    has_more: false,
  }),
  get_assistant_history_entry: (args) =>
    assistantSessions.find((session) => session.id === args.id) ?? null,
  list_assistant_conversations: (args) => {
    const filter = (args.filter ?? {}) as {
      query?: string | null;
      meetings_only?: boolean;
      meeting_id?: number | null;
    };
    const words = (filter.query ?? "")
      .toLowerCase()
      .split(/\s+/)
      .filter(Boolean);
    const matching = assistantSessions
      .filter((s) => !filter.meetings_only || s.meeting_id !== null)
      .filter(
        (s) =>
          filter.meeting_id === null ||
          filter.meeting_id === undefined ||
          s.meeting_id === filter.meeting_id,
      )
      .filter((s) => {
        const text = [
          s.title,
          s.meeting_title ?? "",
          ...s.messages.map((m) => m.content),
        ]
          .join(" ")
          .toLowerCase();
        return words.every((word) => text.includes(word));
      })
      .sort((a, b) => b.updated_at - a.updated_at);
    const offset = Number(args.offset ?? 0);
    const limit = Number(args.limit ?? 30);
    return {
      entries: matching
        .slice(offset, offset + limit)
        .map(({ messages, ...summary }) => ({
          ...summary,
          message_count: messages.length,
        })),
      has_more: matching.length > offset + limit,
    };
  },
  assistant_conversation_meeting: () => null,
  assistant_discuss_meeting: () => null,
  get_usage_stats: () =>
    fresh
      ? {
          total_dictations: 0,
          total_words: 0,
          timed_words: 0,
          total_audio_seconds: 0,
          today_words: 0,
          today_dictations: 0,
          current_streak_days: 0,
          longest_streak_days: 0,
          active_days: 0,
          recent_days: [],
        }
      : usageStats(),
  list_meetings: () => ({ meetings, has_more: false }),
  get_meeting: ({ meetingId }) =>
    structuredClone(meetings.find((entry) => entry.id === meetingId) ?? null),
  get_meeting_segments: ({ meetingId, offset }) =>
    meetingId === LIVE_MEETING_ID || Number(offset ?? 0) > 0
      ? { segments: [], has_more: false, total: 0 }
      : {
          segments: segmentsFor(meetingId as number),
          has_more: false,
          total: MEETING_TURNS.length,
        },
  // The seeded pair is English, as Rust writes it; the UI translates it. One
  // diarized voice has been renamed by the user and one has not.
  get_meeting_speakers: ({ meetingId }) => [
    { speaker_key: "me", display_name: "You", is_me: true },
    { speaker_key: "them", display_name: "Others", is_me: false },
    ...(meetingId === LIVE_MEETING_ID
      ? []
      : [
          { speaker_key: "spk_1", display_name: "Priya", is_me: false },
          { speaker_key: "spk_2", display_name: "Speaker 2", is_me: false },
        ]),
  ],
  get_meeting_state: () => liveState(),
  is_meeting_notes_running: () => params.get("notes") === "writing",
  get_meeting_chat: ({ meetingId }) =>
    meetingId === 40 || meetingId === LIVE_MEETING_ID
      ? structuredClone(meetingChat)
      : [],
  get_diarization_status: () => ({ installed: false, download_mb: 27 }),
  get_meeting_pill_expanded: () => params.get("pill") === "expanded",
  get_system_audio_status: () => ({ supported: true, help: null }),
  get_call_detection_status: () => ({ supported: true, enabled: false }),
  get_meeting_indicator: () => liveMeeting,
  get_auto_learn_status: () => ({
    supported: true,
    enabled: false,
    learned: [],
    max_learned: 50,
  }),
  list_reminders: () =>
    fresh
      ? []
      : [
          ["Call the bank about the card", 25 * 60],
          ["Send the invoice to Sara", 3 * 3600],
          ["Review the pull request", 26 * 3600],
        ].map(([text, inSeconds], index) => ({
          id: `rem-${index}`,
          text,
          note: null,
          due_at: new Date(
            Date.now() + (inSeconds as number) * 1000,
          ).toISOString(),
          created_at: new Date().toISOString(),
          snoozes: 0,
          fired: false,
        })),
  get_available_microphones: () => [
    { index: "0", name: "Microphone (USB Audio)", is_default: true },
  ],
  get_available_output_devices: () => [
    { index: "0", name: "Speakers", is_default: true },
  ],
  get_local_llm_status: () => ({
    running: false,
    model_id: null,
    engine_present: true,
    port: 11435,
    error: null,
  }),
  get_system_memory_gb: () => 32,
  get_available_accelerators: () => ({
    whisper: ["auto", "cpu", "gpu"],
    ort: ["cpu"],
    gpu_devices: [
      {
        id: 0,
        name: "NVIDIA GeForce RTX 4070 Ti SUPER",
        kind: "dedicated",
        total_vram_mb: 16076,
      },
    ],
    transcribe_cpp_devices: [],
  }),
  // Two screens, so the floating panel's screen picker is on show.
  list_assistant_displays: () => [
    {
      id: "DISPLAY1",
      name: "DISPLAY1",
      width: 2560,
      height: 1440,
      is_primary: true,
      is_current: true,
    },
    {
      id: "DISPLAY2",
      name: "DISPLAY2",
      width: 1920,
      height: 1080,
      is_primary: false,
      is_current: false,
    },
  ],
  get_model_folders: () => [],
  download_model: ({ modelId }) => simulateDownload(modelId as string),
  cancel_download: ({ modelId }) => {
    activeDownloads.get(modelId as string)?.cancel();
    return null;
  },
  get_local_voice_status: () => ({
    route: "webview",
    webgpu: "unknown",
    native_supported: true,
    native_load_failed: false,
    kokoro_native_ready: false,
    kitten_ready: false,
  }),
  get_windows_microphone_permission_status: () => ({
    supported: params.get("permissions") === "denied",
    overall_access:
      params.get("permissions") === "denied" ? "denied" : "allowed",
  }),
  is_recording: () => false,
  is_portable: () => false,
  is_laptop: () => false,
  get_app_dir_path: () => "C:\\Users\\preview\\AppData\\Roaming\\SpeakoFlow",
  get_log_dir_path: () =>
    "C:\\Users\\preview\\AppData\\Local\\SpeakoFlow\\logs",
  check_custom_sounds: () => ({ start: false, stop: false }),
  assistant_list_tts_voices: () =>
    settings.assistant_tts_engine === "elevenlabs"
      ? [
          { id: "JBFqnCBsd6RMkjVDRZzb", label: "George" },
          { id: "EXAVITQu4vr4xnSDxMaL", label: "Sarah" },
          { id: "IKne3meq5aSn9XLyUdCD", label: "Charlie" },
        ]
      : [
          { id: "alloy", label: "alloy" },
          { id: "verse", label: "verse" },
        ],
  assistant_list_tts_models: () => [],
  fetch_post_process_models: ({ providerId }) =>
    (
      ({
        anthropic: ["claude-sonnet-4-5", "claude-haiku-4-5", "claude-opus-4-1"],
        openai: ["gpt-5-mini", "gpt-5", "gpt-4.1-mini"],
        groq: ["openai/gpt-oss-120b", "llama-3.3-70b-versatile"],
      }) as Record<string, string[]>
    )[providerId as string]?.map((id) => ({ id, label: id })) ?? [],
  preview_recording_retention: () => 0,
  "plugin:app|version": () => "1.5.0",
  "plugin:window|is_maximized": () => false,
  "plugin:os|locale": () => "en-US",
  "plugin:updater|check": () =>
    params.get("update") === "1"
      ? {
          rid: 1,
          currentVersion: "1.5.0",
          version: "1.6.0",
          date: "2026-10-12T09:00:00Z",
          body: "## What's new\n\n- **Updates install themselves.** No more downloading every version by hand.\n- Send feedback from the **?** next to Settings.\n- Fixed the overlay hiding behind full-screen apps.",
          rawJson: {},
        }
      : null,
  get_update_support: () => ({
    mode: "in_app",
    release_page:
      "https://github.com/AbhishekBarali/SpeakoFlow/releases/latest",
  }),
  get_feedback_system_info: () => ({
    app_version: "1.5.0",
    os: "Windows 10.0.26100",
    arch: "x86_64",
    install: "nsis",
  }),
  // A short wait, so the preview shows the "Sending…" state like the app does.
  send_feedback: () =>
    new Promise((resolve) =>
      setTimeout(() => resolve({ attachments_dropped: 0 }), 600),
    ),

  // Setters that change what the pages show.
  set_stt_engine_mode: ({ mode }) => {
    settings.stt_engine_mode = mode;
    return null;
  },
  set_cloud_stt_provider: ({ providerId }) => {
    settings.cloud_stt_provider_id = providerId;
    return null;
  },
  set_active_model: ({ modelId }) => {
    const entry = models.find((item) => item.id === modelId);
    if (!entry?.is_downloaded)
      return Promise.reject("The preview model is not installed");
    if (matchesFailure("failLoad", modelId as string, entry)) {
      void emit("model-state-changed", {
        event_type: "loading_failed",
        model_id: modelId,
        error: "The preview model could not be loaded",
      });
      return Promise.reject("The preview model could not be loaded");
    }
    settings.selected_model = modelId;
    void emit("model-state-changed", {
      event_type: "loading_completed",
      model_id: modelId,
    });
    return null;
  },
  set_assistant_provider: ({ providerId }) => {
    settings.assistant_provider_id = providerId;
    return null;
  },
  change_assistant_model_setting: ({ providerId, model: id }) => {
    (settings.assistant_models as Json)[providerId as string] = id;
    return null;
  },
  set_cleanup_local_model: ({ modelId }) => {
    const generalPrompts = [
      "default_improve_transcriptions",
      "speakoflow_readable",
    ];
    const selected = settings.post_process_selected_prompt_id as string;
    if (modelId === "speakoflow-mini" && generalPrompts.includes(selected)) {
      settings.post_process_selected_prompt_id = "speakoflow_mini_cleanup";
    } else if (
      modelId !== "speakoflow-mini" &&
      selected === "speakoflow_mini_cleanup"
    ) {
      settings.post_process_selected_prompt_id = "speakoflow_readable";
    }
    settings.post_process_provider_id = "builtin";
    (settings.post_process_models as Json).builtin = modelId;
    return null;
  },
  set_post_process_provider: ({ providerId }) => {
    settings.post_process_provider_id = providerId;
    return null;
  },
  change_post_process_tone_setting: ({ tone }) => {
    settings.post_process_selected_tone_id = tone;
    return null;
  },
  change_post_process_enabled_setting: ({ enabled }) => {
    settings.post_process_enabled = enabled;
    return null;
  },
  change_post_process_on_dictation_setting: ({ enabled }) => {
    settings.post_process_on_dictation = enabled;
    return null;
  },
  set_assistant_enabled: ({ enabled }) => {
    settings.assistant_enabled = enabled;
    return null;
  },
  set_assistant_tts_enabled: ({ enabled }) => {
    settings.assistant_tts_enabled = enabled;
    return null;
  },
  set_assistant_tts_engine: ({ engine }) => {
    // Like the backend: the engine's own saved values become the live ones.
    settings.assistant_tts_engine = engine;
    const id = engine as string;
    const from = (map: string) =>
      ((settings[map] as Record<string, string> | undefined) ?? {})[id] ?? "";
    settings.assistant_tts_api_key = from("assistant_tts_api_keys");
    settings.assistant_tts_remote_voice = from("assistant_tts_remote_voices");
    settings.assistant_tts_model = from("assistant_tts_models");
    // Hosted engines have a fixed endpoint; only Azure and Custom keep one.
    settings.assistant_tts_base_url = from("assistant_tts_base_urls");
    return null;
  },
  // The per-engine TTS setters write the live field and the engine's own slot.
  ...Object.fromEntries(
    (
      [
        ["set_assistant_tts_api_key", "apiKey", "assistant_tts_api_key"],
        [
          "set_assistant_tts_remote_voice",
          "voice",
          "assistant_tts_remote_voice",
        ],
        ["set_assistant_tts_model", "model", "assistant_tts_model"],
        ["set_assistant_tts_base_url", "baseUrl", "assistant_tts_base_url"],
      ] as const
    ).map(([command, arg, key]) => [
      command,
      (payload: Json) => {
        settings[key] = payload[arg];
        const map = `${key}s`;
        settings[map] = {
          ...((settings[map] as Json | undefined) ?? {}),
          [settings.assistant_tts_engine as string]: payload[arg],
        };
        return null;
      },
    ]),
  ),
  change_post_process_api_key_setting: ({ providerId, apiKey }) => {
    (settings.post_process_api_keys as Json)[providerId as string] = apiKey;
    return null;
  },
  change_post_process_model_setting: ({ providerId, model: id }) => {
    (settings.post_process_models as Json)[providerId as string] = id;
    return null;
  },
  change_post_process_base_url_setting: ({ providerId, baseUrl }) => {
    const provider = (settings.post_process_providers as Json[]).find(
      (item) => item.id === providerId,
    );
    if (provider) provider.base_url = baseUrl;
    return null;
  },
  // Generic mirror for the simple assistant setters the new cards call, so
  // clicking a control in the preview behaves like the app.
  ...Object.fromEntries(
    (
      [
        [
          "set_assistant_ask_screen_access",
          "enabled",
          "assistant_ask_screen_access",
        ],
        [
          "set_assistant_call_screen_access",
          "enabled",
          "assistant_call_screen_access",
        ],
        [
          "set_assistant_vision_capture_timing",
          "timing",
          "assistant_vision_capture_timing",
        ],
        [
          "set_assistant_web_search_enabled",
          "enabled",
          "assistant_web_search_enabled",
        ],
        [
          "set_assistant_web_search_provider",
          "provider",
          "assistant_web_search_provider",
        ],
        ["set_assistant_search_depth", "depth", "assistant_search_depth"],
        ["set_assistant_memory_enabled", "enabled", "assistant_memory_enabled"],
        ["set_assistant_memory_detail", "detail", "assistant_memory_detail"],
        [
          "set_assistant_memory_incognito",
          "incognito",
          "assistant_memory_incognito",
        ],
        [
          "set_assistant_response_length",
          "length",
          "assistant_response_length",
        ],
        [
          "set_assistant_max_history_messages",
          "count",
          "assistant_max_history_messages",
        ],
        ["set_assistant_auto_summarize", "enabled", "assistant_auto_summarize"],
        ["set_assistant_font_size", "size", "assistant_font_size"],
        ["set_assistant_panel_opacity", "opacity", "assistant_panel_opacity"],
        ["set_assistant_ask_anchor", "anchor", "assistant_ask_anchor"],
        ["set_assistant_ask_display", "display", "assistant_ask_display"],
        [
          "set_assistant_active_character",
          "id",
          "assistant_active_character_id",
        ],
        ["change_ptt_setting", "enabled", "push_to_talk"],
      ] as const
    ).map(([command, arg, key]) => [
      command,
      (payload: Json) => {
        settings[key] = payload[arg];
        return null;
      },
    ]),
  ),
  update_custom_words: ({ words }) => {
    settings.custom_words = words;
    return null;
  },
  // The pill harness plays the window: Rust's sizing and mode, as DOM events.
  fit_meeting_pill: ({ height }) => {
    window.dispatchEvent(
      new CustomEvent("preview-pill", { detail: { height } }),
    );
    return null;
  },
  set_meeting_pill_expanded: ({ expanded }) => {
    window.dispatchEvent(
      new CustomEvent("preview-pill", { detail: { expanded } }),
    );
    return null;
  },
  set_meeting_paused: ({ paused }) => {
    params.set("paused", paused ? "1" : "0");
    void emit("meeting-state", liveState());
    return null;
  },
};

mockWindows("main");
mockConvertFileSrc("windows");
mockIPC(
  (cmd, payload) => {
    const handler = handlers[cmd];
    if (handler) return handler((payload ?? {}) as Json);
    // Everything else is a setter or a side effect the preview can ignore.
    return null;
  },
  { shouldMockEvents: true },
);

startLiveFeed();
if (params.get("pill") === "offer") {
  window.setTimeout(() => {
    void emit("meeting-call-detected", { active: true, app: "Zoom.exe" });
  }, 300);
}

export const previewParams = params;
