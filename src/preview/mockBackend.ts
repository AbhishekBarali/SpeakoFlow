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
 *            &stt=device|cloud   &theme=light|dark   &fresh=1 (no history)
 *            &memory=on (assistant memory switched on)
 *            &voice=<engine id> (the voice engine in use; default elevenlabs)
 *            &update=1 (a newer version is available)   &feedback=1 (dialog open)
 */
import {
  mockConvertFileSrc,
  mockIPC,
  mockWindows,
} from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";

const params = new URLSearchParams(window.location.search);
const fresh = params.get("fresh") === "1";
const voiceEngine = params.get("voice") ?? "elevenlabs";
const onEleven = voiceEngine === "elevenlabs";

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

/** About six months of plausible, deterministic daily activity. */
const recentDays = (): Json[] => {
  const pad = (value: number) => String(value).padStart(2, "0");
  const today = new Date();
  const days: Json[] = [];
  const span = 190;
  for (let index = 0; index < span; index += 1) {
    // Rest days and a quiet stretch: the backend only sends days that had a
    // dictation.
    if (index % 6 === 2 || index % 11 === 4) continue;
    if (index > 40 && index < 58) continue;
    const date = new Date(
      today.getFullYear(),
      today.getMonth(),
      today.getDate() - (span - 1 - index),
    );
    const ramp = 0.35 + (0.65 * index) / span;
    const words = Math.round(
      ramp *
        (120 +
          560 * Math.abs(Math.sin(index * 1.7)) +
          (index % 7 === 5 ? 650 : 0)),
    );
    days.push({
      day: `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`,
      dictations: Math.max(1, Math.round(words / 45)),
      words,
      audio_seconds: Math.round(words / 2.6),
    });
  }
  return days;
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
];

const binding = (id: string, current: string): Json => ({
  id,
  name: id,
  description: "",
  default_binding: current,
  current_binding: current,
});

const settings: Json = {
  bindings: {
    transcribe: binding("transcribe", "ctrl_left+super_left"),
    transcribe_with_post_process: binding(
      "transcribe_with_post_process",
      "ctrl+shift+space",
    ),
    assistant: binding("assistant", "ctrl_left+alt_left"),
    assistant_call: binding("assistant_call", "ctrl+shift+c"),
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
      "The user is an entrepreneur managing multiple projects, building a desktop voice assistant in Rust and TypeScript. Prefers short, direct answers and dislikes having their name repeated.",
    notes: [
      ["Likes eating apples.", "high", "auto"],
      [
        "Enjoys playful praise, including being called the Apple God.",
        "high",
        "auto",
      ],
      ["Prefers not to have their name repeated often.", "high", "auto"],
      [
        "Likes being called “boss man” sparingly and only when it fits.",
        "high",
        "auto",
      ],
      ["Works mostly in Rust and TypeScript.", "high", "user"],
      ["Studies alone for exams in the evenings.", "medium", "auto"],
      ["Uses Groq and ElevenLabs as cloud providers.", "medium", "auto"],
      ["Lives in Nepal (UTC+5:45).", "high", "user"],
      ["Prefers metric units.", "medium", "auto"],
      ["Wants reminders phrased as short commands.", "low", "auto"],
      ["Drinks coffee before noon only.", "low", "auto"],
      ["Keeps a separate work and study calendar.", "medium", "auto"],
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
  : [
      "Dit is de lijn, and then the rest of the sentence came through in English.",
      "I see the problem is basically simple. I'm using small models, especially older ones, and not having access to the new models is probably the problem.",
      "Right now I'm going to use another DeepSeek model. That will give you one more example to see how good it really is.",
      "I don't think the outfit was anywhere good.",
      "Can you send the invoice to Sara before the call on Friday?",
      "Remind me to review the pull request after lunch.",
    ].map((text, index) => ({
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
    }));

const meetings = fresh
  ? []
  : [
      ["The meeting was an informal technical discussion", 4, 434],
      ["A brief technical check focused on verifying the build", 2, 61],
      ["No decisions were reached during this meeting", 7, 1003],
      ["Reviewing the performance of the new cleanup model", 2, 463],
    ].map(([title, _speakers, secs], index) => ({
      id: 40 - index,
      title,
      started_at: now - 86400 * (index + 1),
      ended_at: now - 86400 * (index + 1) + (secs as number),
      status: index === 3 ? "interrupted" : "complete",
      mic_file: null,
      system_file: null,
      my_notes: "",
      notes: index % 2 === 0 ? "Notes" : null,
      notes_template: null,
      language: null,
      diarized: true,
      segment_count: 12,
    }));

const readiness = (): Json => {
  const providerId = settings.post_process_provider_id as string;
  const modelsMap = settings.post_process_models as Record<string, string>;
  const providers = settings.post_process_providers as Json[];
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
  has_any_models_available: () => true,
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
    entries: assistantSessions,
    has_more: false,
  }),
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
      : {
          total_dictations: 1832,
          total_words: 28912,
          timed_words: 28400,
          total_audio_seconds: 10560,
          today_words: 412,
          today_dictations: 9,
          current_streak_days: 6,
          longest_streak_days: 21,
          active_days: 64,
          recent_days: recentDays(),
        },
  list_meetings: () => ({ meetings, has_more: false }),
  get_meeting_speakers: () => [
    { speaker_key: "mic", display_name: "Me", is_me: true },
    { speaker_key: "system", display_name: "Them", is_me: false },
  ],
  get_meeting_state: () => ({
    meeting_id: null,
    status: "complete",
    paused: false,
    system_audio: true,
    system_audio_error: null,
    elapsed_ms: 0,
  }),
  get_system_audio_status: () => ({ supported: true, help: null }),
  get_call_detection_status: () => ({ supported: true, enabled: true }),
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
  get_windows_microphone_permission_status: () => ({
    supported: false,
    overall_access: "allowed",
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
        ["set_assistant_panel_size", "size", "assistant_panel_size"],
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

export const previewParams = params;
