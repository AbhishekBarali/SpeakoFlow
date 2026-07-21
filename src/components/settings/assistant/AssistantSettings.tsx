import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  Check,
  Loader2,
  Volume2,
  ArrowUp,
  Globe,
  Keyboard,
  Sparkles,
  Monitor,
  PanelTop,
  Download,
  ChevronRight,
} from "lucide-react";
import {
  commands,
  type TtsVoice,
  type Result,
  type LocalLlmStatus,
  type AssistantResponseLength,
  type AssistantScreenAccessMode,
  type AssistantSearchDepth,
  type ModelUnloadTimeout,
  type VisionCaptureTiming,
} from "@/bindings";
import {
  Dropdown,
  SettingContainer,
  SettingsGroup,
  Slider,
  ToggleSwitch,
} from "@/components/ui";
import { Input } from "../../ui/Input";
import { ModelCombo } from "../../ui/ModelCombo";
import { Button } from "../../ui/Button";
import { TONE_TILE } from "../../ui/tones";
import { ProviderModeToggle } from "../PostProcessingSettingsApi/ProviderModeToggle";
import { ShortcutInput } from "../ShortcutInput";
import { PushToTalk } from "../PushToTalk";
import { useSettings } from "../../../hooks/useSettings";
import { useKokoroTts } from "../../../assistant/useKokoroTts";
import { FONT_SIZES } from "../../../assistant/appearance";
import "../../../assistant/AssistantPanel.css";
import { useModelStore } from "@/stores/modelStore";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { useLocalLlmEngineStatus } from "@/hooks/useLocalLlmEngineStatus";
import ScreenRecordingPermission from "@/components/ScreenRecordingPermission";

/** The built-in (local) llama.cpp provider id, mirrored from the backend. */
const BUILTIN_PROVIDER_ID = "builtin";

const KOKORO_DTYPES = [
  { value: "fp32", label: "fp32 (best quality, WebGPU)" },
  { value: "fp16", label: "fp16 (half precision)" },
  { value: "q8", label: "q8 (8-bit, fast on CPU)" },
  { value: "q4", label: "q4 (4-bit, fastest)" },
  { value: "q4f16", label: "q4f16 (4-bit mixed)" },
];

const KOKORO_VOICES = [
  { value: "af_heart", label: "Heart (US female)" },
  { value: "af_bella", label: "Bella (US female)" },
  { value: "af_nicole", label: "Nicole (US female, soft)" },
  { value: "af_sky", label: "Sky (US female)" },
  { value: "am_adam", label: "Adam (US male)" },
  { value: "am_michael", label: "Michael (US male)" },
  { value: "bf_emma", label: "Emma (UK female)" },
  { value: "bm_george", label: "George (UK male)" },
];

/** Quick-pick playback speeds for the TTS speed control. Users can also type
 *  an arbitrary value (clamped to 0.25–4 by the backend). */
const TTS_SPEED_PRESETS = [0.5, 1, 1.5, 2, 3];

/** Rotating set of playful lines spoken by the "Test voice" button. One is
 *  picked at random on each press instead of always saying the same thing.
 *  These are intentionally not translated — they're meme sample lines. */
const TEST_PHRASES = [
  "Wagwan brother.",
  "Hi! This is a test of SpeakoFlow's voice output.",
  "Ayo, is this thing on?",
  "Greetings, human. Your voice assistant has entered the chat.",
  "Testing, testing, one two... yeah we good.",
  "Beep boop, I am definitely not a robot.",
  "Loud and clear, captain.",
  "Yo, mic check. Sounding crispy.",
];

/** Pick a random sample line for the voice test. */
const randomTestPhrase = (): string =>
  TEST_PHRASES[Math.floor(Math.random() * TEST_PHRASES.length)];

/** Editable model/voice picker (input + datalist + refresh). Shared with the
 *  dictation-cleanup model field via `@/components/ui/ModelCombo` so the two
 *  never drift. Aliased here to keep the existing call sites unchanged. */
const LoadableSelect = ModelCombo;

/** Live preview of the assistant panel. Renders the REAL panel classes from
 *  AssistantPanel.css (dark-only, like the STT overlay), so the preview and
 *  the actual panel share one stylesheet and can never drift. */
const PanelPreview: React.FC<{
  fontSize: string;
  opacity: number;
}> = ({ fontSize, opacity }) => {
  const { t } = useTranslation();
  const fs = FONT_SIZES[fontSize] ?? FONT_SIZES.medium;

  return (
    <div
      className="assistant-scope assistant-preview"
      style={
        {
          "--as-msg-font": fs,
          "--as-alpha": String(Math.max(opacity, 0.5)),
        } as React.CSSProperties
      }
    >
      <div className="assistant-panel">
        <div className="assistant-header">
          <div className="assistant-title">
            <span className="assistant-status-dot" />
            {t("assistant.title")}
          </div>
          <div className="assistant-header-actions">
            <span className="assistant-icon-button">
              <Volume2 size={14} />
            </span>
          </div>
        </div>
        <div className="assistant-messages">
          <div className="assistant-message user">
            <div className="assistant-message-content">
              {t("settings.assistant.appearance.previewUser")}
            </div>
          </div>
          <div className="assistant-message assistant">
            <div className="assistant-message-content">
              {t("settings.assistant.appearance.previewAssistant")}
            </div>
          </div>
        </div>
        <div className="assistant-input-row">
          <div
            className="assistant-input"
            style={{
              display: "flex",
              alignItems: "center",
              color: "var(--as-faint)",
            }}
          >
            {t("assistant.inputPlaceholder")}
          </div>
          <span className="assistant-send-button">
            <ArrowUp size={15} strokeWidth={2.5} />
          </span>
        </div>
      </div>
    </div>
  );
};

interface AssistantSettingsProps {
  /** Open the on-device model catalog sub-page (owned by the parent section). */
  onOpenLlmCatalog?: () => void;
}

export const AssistantSettings: React.FC<AssistantSettingsProps> = ({
  onOpenLlmCatalog,
}) => {
  const { t } = useTranslation();
  const {
    settings,
    refreshSettings,
    updatePostProcessApiKey,
    updatePostProcessBaseUrl,
  } = useSettings();

  const providers = settings?.post_process_providers || [];
  const selectedProviderId = settings?.assistant_provider_id || "custom";
  const selectedProvider = providers.find((p) => p.id === selectedProviderId);

  // Built-in (local) provider: model is chosen from downloaded LLM models and
  // there is no API key. The engine is the bundled llama.cpp sidecar.
  const isBuiltin = selectedProviderId === BUILTIN_PROVIDER_ID;
  const { models } = useModelStore();
  const llmModels = useMemo(
    () =>
      models.filter((m) => getModelCategory(m) === "llm" && m.is_downloaded),
    [models],
  );
  const [localLlmStatus, setLocalLlmStatus] = useState<LocalLlmStatus | null>(
    null,
  );
  // Live built-in engine (llama.cpp binary) setup progress, shared with the
  // assistant panel via the same backend events, so this form can show the
  // one-time first-run engine download instead of leaving it invisible.
  const engineStatus = useLocalLlmEngineStatus();
  useEffect(() => {
    if (!isBuiltin) return;
    let active = true;
    void commands.getLocalLlmStatus().then((res) => {
      if (active && res.status === "ok") setLocalLlmStatus(res.data);
    });
    return () => {
      active = false;
    };
  }, [isBuiltin]);

  const [model, setModel] = useState("");
  const [historyLimit, setHistoryLimit] = useState("12");
  const [contextSize, setContextSize] = useState("8192");
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [ttsBaseUrl, setTtsBaseUrl] = useState("");
  const [ttsApiKey, setTtsApiKey] = useState("");
  const [ttsModel, setTtsModel] = useState("");
  const [ttsRemoteVoice, setTtsRemoteVoice] = useState("");
  // Manual playback-speed entry (string while editing; committed on blur).
  const [ttsSpeedInput, setTtsSpeedInput] = useState("1");

  // Remember the last cloud (non-built-in) provider so flipping the brain
  // picker back to "Cloud provider" restores the user's choice instead of
  // resetting to a default.
  const [lastCloudProviderId, setLastCloudProviderId] = useState<string>(
    selectedProvider &&
      selectedProvider.id !== BUILTIN_PROVIDER_ID &&
      selectedProvider.id !== "apple_intelligence"
      ? selectedProvider.id
      : "custom",
  );
  const providerSwitchSequence = useRef(0);
  const providerSwitchQueue = useRef<Promise<void>>(Promise.resolve());
  const [isProviderSwitching, setIsProviderSwitching] = useState(false);

  // Web search section state.
  const [webSearchApiKey, setWebSearchApiKey] = useState("");
  const [webSearchTest, setWebSearchTest] = useState<
    "idle" | "testing" | "ok" | "error"
  >("idle");
  const [webSearchTestMsg, setWebSearchTestMsg] = useState<string | null>(null);

  // TTS test button state (shared across engines).
  const [testState, setTestState] = useState<
    "idle" | "testing" | "ok" | "error"
  >("idle");
  const [testError, setTestError] = useState<string | null>(null);

  // Assistant model list, fetched per-provider from its /models endpoint via
  // the same command post-processing uses (providers + keys are shared). Kept
  // local to this component so it doesn't couple to the post-processing tab.
  const [loadedModels, setLoadedModels] = useState<Record<string, string[]>>(
    {},
  );
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);

  // Remote TTS voice / model lists, loaded on demand for the OpenAI-compatible,
  // ElevenLabs, and Azure engines (searchable pickers instead of raw text
  // fields).
  const [ttsVoiceList, setTtsVoiceList] = useState<TtsVoice[]>([]);
  const [ttsVoicesLoading, setTtsVoicesLoading] = useState(false);
  const [ttsVoicesError, setTtsVoicesError] = useState<string | null>(null);
  const [ttsModelList, setTtsModelList] = useState<string[]>([]);
  const [ttsModelsLoading, setTtsModelsLoading] = useState(false);
  const [ttsModelsError, setTtsModelsError] = useState<string | null>(null);

  const ttsEngine = settings?.assistant_tts_engine ?? "kokoro";
  const ttsEnabled = settings?.assistant_tts_enabled ?? false;
  const ttsVoice = settings?.assistant_tts_voice ?? "af_heart";
  const ttsDtype = settings?.assistant_tts_kokoro_dtype ?? "fp32";
  const ttsSpeed = settings?.assistant_tts_speed ?? 1;

  // TTS fields save on blur. Serialize those saves with actions such as Load
  // and Test so a click can never overtake the blur that contains a newly typed
  // API key/model/voice (the previous race sent OpenRouter an unauthenticated
  // request even while the password field visibly contained a key).
  const ttsTaskQueue = useRef<Promise<void>>(Promise.resolve());
  const queueTtsTask = (task: () => Promise<void>): Promise<void> => {
    const next = ttsTaskQueue.current.catch(() => undefined).then(task);
    ttsTaskQueue.current = next;
    return next;
  };

  const runTtsCommand = async (command: Promise<Result<null, string>>) => {
    const result = await command;
    if (result.status === "error") throw new Error(result.error);
  };

  /** Persist every visible remote-TTS draft before an action consumes it. */
  const persistRemoteTtsDraft = async () => {
    let changed = false;
    if (
      (ttsEngine === "openai" || ttsEngine === "azure") &&
      ttsBaseUrl !== (settings?.assistant_tts_base_url ?? "")
    ) {
      await runTtsCommand(commands.setAssistantTtsBaseUrl(ttsBaseUrl));
      changed = true;
    }
    if (
      ttsEngine !== "kokoro" &&
      ttsApiKey !== (settings?.assistant_tts_api_key ?? "")
    ) {
      await runTtsCommand(commands.setAssistantTtsApiKey(ttsApiKey));
      changed = true;
    }
    if (
      (ttsEngine === "openai" ||
        ttsEngine === "openrouter" ||
        ttsEngine === "elevenlabs") &&
      ttsModel.trim() !== (settings?.assistant_tts_model ?? "").trim()
    ) {
      await runTtsCommand(commands.setAssistantTtsModel(ttsModel.trim()));
      changed = true;
    }
    if (
      ttsEngine !== "kokoro" &&
      ttsRemoteVoice.trim() !==
        (settings?.assistant_tts_remote_voice ?? "").trim()
    ) {
      await runTtsCommand(
        commands.setAssistantTtsRemoteVoice(ttsRemoteVoice.trim()),
      );
      changed = true;
    }

    const parsedSpeed = Number.parseFloat(ttsSpeedInput);
    if (Number.isFinite(parsedSpeed)) {
      const clampedSpeed = Math.min(4, Math.max(0.25, parsedSpeed));
      if (clampedSpeed !== ttsSpeed) {
        await runTtsCommand(commands.setAssistantTtsSpeed(clampedSpeed));
        changed = true;
      }
    }
    if (changed) await refreshSettings();
  };

  // Fetch the model list for the selected assistant provider from its /models
  // endpoint (shares the post-processing command since providers + keys are
  // shared). Errors surface inline; the field stays a free-text search so a
  // provider without a model list is never a dead end.
  const handleLoadAssistantModels = async () => {
    setModelsLoading(true);
    setModelsError(null);
    try {
      const res = await commands.fetchPostProcessModels(selectedProviderId);
      if (res.status === "error") {
        setModelsError(res.error);
        return;
      }
      setLoadedModels((prev) => ({ ...prev, [selectedProviderId]: res.data }));
      if (res.data.length === 0) {
        setModelsError(t("settings.assistant.provider.noModelsFound"));
      }
    } catch (e) {
      setModelsError(String(e));
    } finally {
      setModelsLoading(false);
    }
  };

  const handleAssistantModelChange = async (value: string) => {
    const trimmed = value.trim();
    if (!trimmed) return;
    setModel(trimmed);
    await commands.changeAssistantModelSetting(selectedProviderId, trimmed);
    await refreshSettings();
  };

  // Load selectable voices / models for the current remote TTS engine.
  // Await pending field saves first so discovery uses the value on screen.
  const handleLoadTtsVoices = async () => {
    setTtsVoicesLoading(true);
    setTtsVoicesError(null);
    try {
      await queueTtsTask(persistRemoteTtsDraft);
      const res = await commands.assistantListTtsVoices();
      if (res.status === "error") {
        setTtsVoicesError(res.error);
        setTtsVoiceList([]);
        return;
      }
      setTtsVoiceList(res.data);
    } catch (e) {
      setTtsVoicesError(String(e));
      setTtsVoiceList([]);
    } finally {
      setTtsVoicesLoading(false);
    }
  };

  const handleLoadTtsModels = async () => {
    setTtsModelsLoading(true);
    setTtsModelsError(null);
    try {
      await queueTtsTask(persistRemoteTtsDraft);
      const res = await commands.assistantListTtsModels();
      if (res.status === "error") {
        setTtsModelsError(res.error);
        setTtsModelList([]);
        return;
      }
      setTtsModelList(res.data);
    } catch (e) {
      setTtsModelsError(String(e));
      setTtsModelList([]);
    } finally {
      setTtsModelsLoading(false);
    }
  };
  const kokoroEnabled = ttsEnabled && ttsEngine === "kokoro";
  // Settings must never download Kokoro just because this page mounted. The
  // live assistant still loads on an actual spoken reply; here, only Setup or
  // Test voice may call prepare/speak.
  const kokoroTest = useKokoroTts(
    kokoroEnabled,
    ttsVoice,
    ttsDtype,
    ttsSpeed,
    false,
  );
  const {
    status: kokoroStatus,
    progress: kokoroProgress,
    error: kokoroError,
  } = kokoroTest;
  const kokoroReadyKey = `speakoflow.kokoro.ready.${ttsDtype}`;
  const [kokoroPrepared, setKokoroPrepared] = useState(false);

  // Reflect the ACTUAL browser cache, not just a local flag. kokoro-js
  // (transformers.js) caches model weights in the "transformers-cache" Cache
  // Storage; if this precision's weights are already present, the model is
  // ready and we must NOT prompt a re-download. This fixes "have to click
  // Download every time you switch": the old flag was keyed per-precision and
  // only set from this page, so it desynced from reality (e.g. after the live
  // panel had already downloaded the model). Falls back to the local flag if
  // the Cache API is unavailable, so there's no regression.
  useEffect(() => {
    let cancelled = false;
    const dtypeSuffix: Record<string, string> = {
      fp32: "",
      fp16: "_fp16",
      q8: "_quantized",
      int8: "_int8",
      uint8: "_uint8",
      q4: "_q4",
      q4f16: "_q4f16",
      bnb4: "_bnb4",
    };
    const refresh = async () => {
      let cached = false;
      try {
        if (typeof caches !== "undefined") {
          const cache = await caches.open("transformers-cache");
          const urls = (await cache.keys())
            .map((r) => r.url)
            .filter((u) => u.includes("Kokoro-82M"));
          const file = `model${dtypeSuffix[ttsDtype] ?? ""}.onnx`;
          cached =
            urls.some((u) => u.includes(file)) ||
            urls.some((u) => u.endsWith(".onnx"));
        }
      } catch {
        cached = false;
      }
      const flagged = window.localStorage.getItem(kokoroReadyKey) === "true";
      if (!cancelled) setKokoroPrepared(cached || flagged);
    };
    void refresh();
    return () => {
      cancelled = true;
    };
  }, [kokoroReadyKey, ttsDtype]);

  const rememberKokoroReady = () => {
    window.localStorage.setItem(kokoroReadyKey, "true");
    setKokoroPrepared(true);
  };

  const handlePrepareKokoro = async () => {
    try {
      await kokoroTest.prepare();
      rememberKokoroReady();
    } catch {
      // The hook exposes a precise error state in the setup row.
    }
  };

  const handleTestTts = async () => {
    setTestState("testing");
    setTestError(null);
    const phrase = randomTestPhrase();
    try {
      if (ttsEngine === "kokoro") {
        await kokoroTest.prepare();
        rememberKokoroReady();
        await kokoroTest.speak(phrase, true);
      } else {
        await queueTtsTask(persistRemoteTtsDraft);
        const res = await commands.assistantTestTts(phrase);
        if (res.status === "error") {
          setTestState("error");
          setTestError(res.error);
          return;
        }
      }
      setTestState("ok");
      setTimeout(() => setTestState("idle"), 2000);
    } catch (e) {
      setTestState("error");
      setTestError(String(e));
    }
  };

  useEffect(() => {
    setModel(settings?.assistant_models?.[selectedProviderId] ?? "");
    setApiKey(settings?.post_process_api_keys?.[selectedProviderId] ?? "");
    setBaseUrl(selectedProvider?.base_url ?? "");
  }, [settings, selectedProviderId, selectedProvider]);

  useEffect(() => {
    setHistoryLimit(String(settings?.assistant_max_history_messages ?? 12));
  }, [settings?.assistant_max_history_messages]);

  const handleHistoryLimitBlur = async () => {
    const parsed = Math.max(0, Math.min(200, parseInt(historyLimit, 10) || 0));
    setHistoryLimit(String(parsed));
    await commands.setAssistantMaxHistoryMessages(parsed);
    await refreshSettings();
  };

  // Built-in local engine context window. Mirrors the history-limit pattern:
  // local input state, clamped + persisted on blur. Only shown for the
  // built-in provider (external providers manage their own context).
  useEffect(() => {
    setContextSize(String(settings?.local_llm_context_size ?? 8192));
  }, [settings?.local_llm_context_size]);

  const handleContextSizeBlur = async () => {
    const parsed = Math.max(
      512,
      Math.min(32768, parseInt(contextSize, 10) || 8192),
    );
    setContextSize(String(parsed));
    await commands.setLocalLlmContextSize(parsed);
    await refreshSettings();
  };

  // Idle-unload timeout for the built-in local LLM engine: after this long with
  // no use, the model is unloaded from RAM/VRAM (it reloads on next use). Mirrors
  // the STT model-unload control; the extra 15s option only shows in debug mode.
  const llmUnloadOptions = useMemo(() => {
    // NOTE: the values are the serde snake_case forms the backend actually
    // accepts ("min2", not the "min_2" specta emits for the TS type), matching
    // ModelUnloadTimeout.tsx — hence the casts. Sending the specta form would
    // fail to deserialize and silently not save.
    const base: { value: ModelUnloadTimeout; label: string }[] = [
      {
        value: "never" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.never"),
      },
      {
        value: "immediately" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.immediately"),
      },
      {
        value: "min2" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min2"),
      },
      {
        value: "min5" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min5"),
      },
      {
        value: "min10" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min10"),
      },
      {
        value: "min15" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min15"),
      },
      {
        value: "hour1" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.hour1"),
      },
    ];
    if (settings?.debug_mode) {
      base.push({
        value: "sec15" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.sec15"),
      });
    }
    return base;
  }, [t, settings?.debug_mode]);

  useEffect(() => {
    setTtsBaseUrl(settings?.assistant_tts_base_url ?? "");
    setTtsApiKey(settings?.assistant_tts_api_key ?? "");
    setTtsModel(settings?.assistant_tts_model ?? "");
    setTtsRemoteVoice(settings?.assistant_tts_remote_voice ?? "");
    setTtsSpeedInput(String(settings?.assistant_tts_speed ?? 1));
  }, [
    settings?.assistant_tts_base_url,
    settings?.assistant_tts_api_key,
    settings?.assistant_tts_model,
    settings?.assistant_tts_remote_voice,
    settings?.assistant_tts_speed,
  ]);

  // Clear any loaded voice/model lists when the TTS engine changes so a stale
  // list (e.g. OpenAI voices) never shows under a different engine.
  useEffect(() => {
    setTtsVoiceList([]);
    setTtsVoicesError(null);
    setTtsModelList([]);
    setTtsModelsError(null);
  }, [settings?.assistant_tts_engine]);

  const providerOptions = useMemo(
    () =>
      providers
        .filter((provider) => provider.id !== "apple_intelligence")
        .map((provider) => ({ value: provider.id, label: provider.label }))
        // Keep the built-in local model pinned to the top — it's the zero-setup,
        // no-API-key option most users should reach for first.
        .sort((a, b) =>
          a.value === BUILTIN_PROVIDER_ID
            ? -1
            : b.value === BUILTIN_PROVIDER_ID
              ? 1
              : 0,
        ),
    [providers],
  );

  // Cloud (non-built-in) providers only, for the "Cloud provider" brain mode.
  const cloudProviderOptions = useMemo(
    () => providerOptions.filter((p) => p.value !== BUILTIN_PROVIDER_ID),
    [providerOptions],
  );

  useEffect(() => {
    if (
      cloudProviderOptions.some((option) => option.value === selectedProviderId)
    ) {
      setLastCloudProviderId(selectedProviderId);
    }
  }, [cloudProviderOptions, selectedProviderId]);

  // Options for the searchable assistant-model picker: loaded models for the
  // current provider plus the currently-set model (so a hand-typed value still
  // shows as selected). The Select is creatable, so users can type any model.
  const assistantModelOptions = useMemo(() => {
    const seen = new Set<string>();
    const opts: { value: string; label: string }[] = [];
    const add = (v?: string | null) => {
      const trimmed = v?.trim();
      if (!trimmed || seen.has(trimmed)) return;
      seen.add(trimmed);
      opts.push({ value: trimmed, label: trimmed });
    };
    for (const m of loadedModels[selectedProviderId] || []) add(m);
    add(model);
    return opts;
  }, [loadedModels, selectedProviderId, model]);

  const ttsVoiceOptions = useMemo(() => {
    const seen = new Set<string>();
    const opts: { value: string; label: string }[] = [];
    const add = (value: string, label: string) => {
      const v = value.trim();
      if (!v || seen.has(v)) return;
      seen.add(v);
      opts.push({ value: v, label: label || v });
    };
    for (const v of ttsVoiceList) add(v.id, v.label);
    if (ttsRemoteVoice.trim()) add(ttsRemoteVoice, ttsRemoteVoice);
    return opts;
  }, [ttsVoiceList, ttsRemoteVoice]);

  const ttsModelOptions = useMemo(() => {
    const seen = new Set<string>();
    const opts: { value: string; label: string }[] = [];
    const add = (v?: string | null) => {
      const trimmed = v?.trim();
      if (!trimmed || seen.has(trimmed)) return;
      seen.add(trimmed);
      opts.push({ value: trimmed, label: trimmed });
    };
    for (const m of ttsModelList) add(m);
    if (ttsModel.trim()) add(ttsModel);
    return opts;
  }, [ttsModelList, ttsModel]);

  const showProviderSwitchError = () => {
    toast.error(
      t("settings.assistant.provider.saveFailed", {
        defaultValue: "Couldn’t switch the Assistant provider.",
      }),
    );
  };

  const handleProviderSelect = (providerId: string) => {
    const isValidTarget =
      providerId === BUILTIN_PROVIDER_ID ||
      cloudProviderOptions.some((option) => option.value === providerId);
    if (!isValidTarget) {
      showProviderSwitchError();
      return;
    }

    const sequence = ++providerSwitchSequence.current;
    setIsProviderSwitching(true);
    const run = providerSwitchQueue.current
      .catch(() => undefined)
      .then(async () => {
        // Skip a queued choice that was superseded before its write began.
        if (sequence !== providerSwitchSequence.current) return;
        try {
          const result = await commands.setAssistantProvider(providerId);
          if (sequence !== providerSwitchSequence.current) return;
          if (result.status !== "ok") {
            showProviderSwitchError();
            return;
          }
          await refreshSettings();
        } catch (error) {
          console.error("Failed to switch Assistant provider:", error);
          if (sequence === providerSwitchSequence.current) {
            showProviderSwitchError();
          }
        }
      });
    providerSwitchQueue.current = run.finally(() => {
      if (sequence === providerSwitchSequence.current) {
        setIsProviderSwitching(false);
      }
    });
  };

  // Segmented brain picker: "On my device" is the built-in provider; "Cloud
  // provider" is any other supported provider. Switching restores the user's
  // last valid cloud choice rather than a hidden/stale provider.
  const brainMode: "device" | "cloud" = isBuiltin ? "device" : "cloud";
  const handleBrainModeChange = (mode: "device" | "cloud") => {
    if (mode === "device") {
      if (!isBuiltin) handleProviderSelect(BUILTIN_PROVIDER_ID);
      return;
    }
    if (!isBuiltin) return;

    const target = cloudProviderOptions.some(
      (option) => option.value === lastCloudProviderId,
    )
      ? lastCloudProviderId
      : cloudProviderOptions[0]?.value;
    if (target) handleProviderSelect(target);
  };

  const handleApiKeyBlur = async () => {
    await updatePostProcessApiKey(selectedProviderId, apiKey);
  };

  const handleBaseUrlBlur = async () => {
    // Base URLs are shared with AI cleanup, so use the hardened store action:
    // it checks Result.status, clears the now-invalid model, refreshes
    // readiness, and reports failures — instead of a silent bypass write.
    await updatePostProcessBaseUrl(selectedProviderId, baseUrl);
  };

  const setAndRefresh = async (promise: Promise<unknown>) => {
    await promise;
    await refreshSettings();
  };

  const currentTtsSpeed = settings?.assistant_tts_speed ?? 1;

  // Persist a playback speed (preset or typed). The backend clamps to 0.25–4.
  const commitTtsSpeed = async (value: number) => {
    const clamped = Math.min(4, Math.max(0.25, value));
    setTtsSpeedInput(String(clamped));
    await setAndRefresh(commands.setAssistantTtsSpeed(clamped));
  };

  const handleTtsSpeedBlur = () => {
    const parsed = parseFloat(ttsSpeedInput);
    if (Number.isFinite(parsed)) {
      void queueTtsTask(() => commitTtsSpeed(parsed));
    } else {
      // Revert an unparseable entry to the persisted value.
      setTtsSpeedInput(String(currentTtsSpeed));
    }
  };

  // --- Web search ---------------------------------------------------------
  const webSearchProvider = settings?.assistant_web_search_provider ?? "serper";
  const webSearchEnabled = settings?.assistant_web_search_enabled ?? false;
  const webSearchNeedsKey =
    webSearchProvider === "serper" ||
    webSearchProvider === "brave" ||
    webSearchProvider === "tavily" ||
    webSearchProvider === "exa" ||
    webSearchProvider === "serpapi";

  // Sync the API-key field to the selected provider's stored key.
  useEffect(() => {
    setWebSearchApiKey(
      settings?.web_search_api_keys?.[webSearchProvider] ?? "",
    );
  }, [settings, webSearchProvider]);

  const handleWebSearchApiKeyBlur = async () => {
    await commands.setAssistantWebSearchApiKey(
      webSearchProvider,
      webSearchApiKey,
    );
    await refreshSettings();
  };

  const handleTestWebSearch = async () => {
    setWebSearchTest("testing");
    setWebSearchTestMsg(null);
    try {
      const res = await commands.assistantTestWebSearch(
        "who is the prime minister of canada",
      );
      if (res.status === "error") {
        setWebSearchTest("error");
        setWebSearchTestMsg(res.error);
        return;
      }
      setWebSearchTest("ok");
      setWebSearchTestMsg(
        t("settings.assistant.webSearch.testResult", {
          count: res.data.length,
        }),
      );
      setTimeout(() => setWebSearchTest("idle"), 4000);
    } catch (e) {
      setWebSearchTest("error");
      setWebSearchTestMsg(String(e));
    }
  };

  const screenAccessMode = settings?.assistant_screen_access_mode ?? "manual";
  const manualScreenAccess = screenAccessMode === "manual";
  const screenAccessDescription = t(
    `settings.assistant.vision.modes.descriptions.${screenAccessMode}`,
  );

  // Cloud provider form (Provider → Base URL where needed → API key → Model),
  // per the §4.0 consistency contract. Shared shape with Dictation's AI-cleanup.
  const cloudProviderForm = (
    <>
      <SettingContainer
        title={t("settings.assistant.provider.providerLabel")}
        layout="horizontal"
        grouped={true}
      >
        <Dropdown
          options={cloudProviderOptions}
          selectedValue={selectedProviderId}
          onSelect={handleProviderSelect}
          disabled={isProviderSwitching}
        />
      </SettingContainer>

      {selectedProvider?.allow_base_url_edit && (
        <SettingContainer
          title={t("settings.assistant.provider.baseUrlLabel")}
          info={t("settings.assistant.provider.baseUrlDescription")}
          layout="horizontal"
          grouped={true}
        >
          <Input
            type="text"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            onBlur={handleBaseUrlBlur}
            placeholder="https://my-resource.openai.azure.com/openai/v1"
            className="w-[340px]"
          />
        </SettingContainer>
      )}

      <SettingContainer
        title={t("settings.assistant.provider.apiKeyLabel")}
        info={t("settings.assistant.provider.apiKeyDescription")}
        layout="horizontal"
        grouped={true}
      >
        <Input
          type="password"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          onBlur={handleApiKeyBlur}
          placeholder={t("settings.assistant.provider.apiKeyPlaceholder")}
          className="w-[340px]"
        />
      </SettingContainer>

      <SettingContainer
        title={t("settings.assistant.provider.modelLabel")}
        layout="horizontal"
        grouped={true}
      >
        <div className="flex flex-col items-end gap-1">
          <LoadableSelect
            value={model}
            options={assistantModelOptions}
            onCommit={(v) => void handleAssistantModelChange(v)}
            onLoad={handleLoadAssistantModels}
            loading={modelsLoading}
            error={modelsError}
            placeholder={t(
              "settings.assistant.provider.modelSearchPlaceholder",
            )}
            loadLabel={t("settings.assistant.provider.loadModels")}
          />
        </div>
      </SettingContainer>
    </>
  );

  // On-device (built-in local engine) brain form: pick a downloaded model or
  // open the catalog to download one; context window + unload timeout fold.
  const deviceProviderForm = (
    <>
      <SettingContainer
        title={t("settings.assistant.provider.modelLabel")}
        layout="horizontal"
        grouped={true}
      >
        <div className="flex flex-col items-end gap-1">
          {llmModels.length > 0 ? (
            <Dropdown
              options={llmModels.map((m) => ({
                value: m.id,
                label: m.name,
              }))}
              selectedValue={model}
              onSelect={(value) => {
                setModel(value);
                void setAndRefresh(
                  commands.changeAssistantModelSetting(
                    selectedProviderId,
                    value,
                  ),
                );
              }}
              placeholder={t(
                "settings.assistant.provider.builtinModelPlaceholder",
              )}
              className="min-w-[200px]"
            />
          ) : (
            <span className="text-xs text-muted-soft max-w-[360px] text-right">
              {t("settings.assistant.provider.builtinNoModels")}
            </span>
          )}
          {engineStatus.active ? (
            <div className="flex w-full max-w-[360px] flex-col items-end gap-1">
              <span className="inline-flex items-center gap-1.5 text-xs text-muted">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {engineStatus.phase === "extracting"
                  ? t("settings.assistant.provider.builtinEngineExtracting")
                  : engineStatus.total > 0
                    ? t(
                        "settings.assistant.provider.builtinEngineDownloading",
                        {
                          percent: engineStatus.pct,
                        },
                      )
                    : t("settings.assistant.provider.builtinEnginePreparing")}
              </span>
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-hairline-strong">
                <div
                  className={`h-full rounded-full bg-accent ${
                    engineStatus.total > 0
                      ? "transition-[width] duration-200"
                      : "w-full animate-pulse"
                  }`}
                  style={
                    engineStatus.total > 0
                      ? { width: `${engineStatus.pct}%` }
                      : undefined
                  }
                />
              </div>
            </div>
          ) : (
            localLlmStatus &&
            !localLlmStatus.engine_present && (
              <span className="text-xs text-amber-500 max-w-[360px] text-right">
                {t("settings.assistant.provider.builtinEngineMissing")}
              </span>
            )
          )}
        </div>
      </SettingContainer>

      <div className="px-4 py-3">
        <button
          type="button"
          onClick={onOpenLlmCatalog}
          className={`flex w-full items-center gap-3 rounded-xl border px-3.5 py-3 text-start transition-colors cursor-pointer ${
            llmModels.length === 0
              ? "border-accent/35 bg-accent/8 hover:bg-accent/12"
              : "border-hairline bg-surface-strong/55 hover:border-hairline-strong hover:bg-surface-strong"
          }`}
        >
          <span
            className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-xl ${TONE_TILE.teal}`}
          >
            <Download size={17} />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-[13px] font-medium text-ink">
              {t("settings.assistant.brain.downloadModel")}
            </span>
            <span className="mt-0.5 block text-xs text-muted">
              {t("settings.assistant.brain.downloadModelDescription")}
            </span>
          </span>
          <span className="flex shrink-0 items-center gap-1 text-xs font-medium text-accent">
            {t("settings.assistant.brain.downloadModelAction")}
            <ChevronRight width={15} height={15} />
          </span>
        </button>
      </div>

      <SettingContainer
        title={t("settings.assistant.provider.contextSizeLabel")}
        info={t("settings.assistant.provider.contextSizeDescription")}
        layout="horizontal"
        grouped={true}
      >
        <Input
          type="number"
          min={512}
          max={32768}
          step={512}
          value={contextSize}
          onChange={(e) => setContextSize(e.target.value)}
          onBlur={handleContextSizeBlur}
          className="w-[120px]"
        />
      </SettingContainer>

      <SettingContainer
        title={t("settings.assistant.provider.unloadTimeoutLabel")}
        info={t("settings.assistant.provider.unloadTimeoutDescription")}
        layout="horizontal"
        grouped={true}
      >
        <Dropdown
          options={llmUnloadOptions}
          selectedValue={
            settings?.local_llm_unload_timeout ?? ("min5" as ModelUnloadTimeout)
          }
          onSelect={(value) =>
            setAndRefresh(
              commands.setLocalLlmUnloadTimeout(value as ModelUnloadTimeout),
            )
          }
          className="min-w-[200px]"
        />
      </SettingContainer>
    </>
  );

  return (
    <div className="max-w-3xl w-full mx-auto space-y-8">
      {/* Hotkeys ---------------------------------------------------------- */}
      <SettingsGroup
        title={t("settings.assistant.shortcuts.title")}
        icon={Keyboard}
      >
        <ShortcutInput
          shortcutId="assistant"
          grouped={true}
          icon={Sparkles}
          tone="teal"
        />
        <ShortcutInput
          shortcutId="assistant_panel_toggle"
          grouped={true}
          icon={PanelTop}
          tone="violet"
        />
        <PushToTalk grouped={true} />
      </SettingsGroup>

      {/* Brain picker ----------------------------------------------------- */}
      <SettingsGroup
        title={t("settings.assistant.brain.title")}
        description={t("settings.assistant.brain.description")}
        icon={Sparkles}
      >
        <SettingContainer
          title={t("settings.assistant.brain.whereLabel")}
          layout="stacked"
          grouped={true}
        >
          <ProviderModeToggle
            mode={brainMode}
            onChange={handleBrainModeChange}
            disabled={isProviderSwitching}
          />
        </SettingContainer>
        {brainMode === "device" ? deviceProviderForm : cloudProviderForm}
      </SettingsGroup>

      {/* Voice output ----------------------------------------------------- */}
      <SettingsGroup title={t("settings.assistant.tts.title")} icon={Volume2}>
        <ToggleSwitch
          checked={settings?.assistant_tts_enabled ?? false}
          onChange={(checked) =>
            setAndRefresh(commands.setAssistantTtsEnabled(checked))
          }
          label={t("settings.assistant.tts.enableLabel")}
          description={t("settings.assistant.tts.enableDescription")}
          grouped={true}
        />
        {ttsEnabled && (
          <>
            <SettingContainer
              title={t("settings.assistant.tts.engineLabel")}
              info={t("settings.assistant.tts.engineDescription")}
              layout="horizontal"
              grouped={true}
            >
              <Dropdown
                options={[
                  {
                    value: "kokoro",
                    label: t("settings.assistant.tts.engines.kokoro"),
                  },
                  {
                    value: "openai",
                    label: t("settings.assistant.tts.engines.openai"),
                  },
                  {
                    value: "openrouter",
                    label: t("settings.assistant.tts.engines.openrouter"),
                  },
                  {
                    value: "elevenlabs",
                    label: t("settings.assistant.tts.engines.elevenlabs"),
                  },
                  {
                    value: "azure",
                    label: t("settings.assistant.tts.engines.azure"),
                  },
                ]}
                selectedValue={settings?.assistant_tts_engine ?? "kokoro"}
                onSelect={(engine) => {
                  void queueTtsTask(async () => {
                    await setAndRefresh(commands.setAssistantTtsEngine(engine));
                  });
                }}
                disabled={!settings?.assistant_tts_enabled}
                className="min-w-[340px]"
              />
            </SettingContainer>

            {(settings?.assistant_tts_engine ?? "kokoro") === "kokoro" && (
              <>
                <SettingContainer
                  title={t("settings.assistant.tts.kokoroSetupLabel")}
                  description={t(
                    "settings.assistant.tts.kokoroSetupDescription",
                  )}
                  descriptionMode="inline"
                  layout="horizontal"
                  grouped={true}
                >
                  <div className="flex min-w-[340px] justify-end">
                    {kokoroStatus === "loading" ? (
                      <div className="w-full max-w-[260px] space-y-1.5">
                        <div className="flex items-center justify-between gap-3 text-xs text-muted">
                          <span className="inline-flex items-center gap-1.5">
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            {t("settings.assistant.tts.kokoroDownloading")}
                          </span>
                          <span className="tabular-nums">
                            {kokoroProgress}%
                          </span>
                        </div>
                        <div className="h-1.5 overflow-hidden rounded-full bg-hairline-strong">
                          <div
                            className="h-full rounded-full bg-accent transition-[width] duration-200"
                            style={{ width: `${kokoroProgress}%` }}
                          />
                        </div>
                      </div>
                    ) : kokoroPrepared ||
                      kokoroStatus === "ready" ||
                      kokoroStatus === "speaking" ? (
                      <span className="inline-flex items-center gap-1.5 text-[13px] font-medium text-accent">
                        <Check className="h-4 w-4" />
                        {t("settings.assistant.tts.kokoroReady")}
                      </span>
                    ) : (
                      <div className="flex flex-col items-end gap-1.5">
                        <Button
                          variant={kokoroError ? "secondary" : "primary-soft"}
                          size="sm"
                          onClick={() => void handlePrepareKokoro()}
                        >
                          <Download className="h-3.5 w-3.5" />
                          {kokoroError
                            ? t("settings.assistant.tts.kokoroRetry")
                            : t("settings.assistant.tts.kokoroDownload")}
                        </Button>
                        {kokoroError && (
                          <span className="text-xs text-error">
                            {t("settings.assistant.tts.downloadError")}
                          </span>
                        )}
                      </div>
                    )}
                  </div>
                </SettingContainer>

                <SettingContainer
                  title={t("settings.assistant.tts.voiceLabel")}
                  layout="horizontal"
                  grouped={true}
                >
                  <Dropdown
                    options={KOKORO_VOICES}
                    selectedValue={settings?.assistant_tts_voice ?? "af_heart"}
                    onSelect={(voice) =>
                      setAndRefresh(commands.setAssistantTtsVoice(voice))
                    }
                    disabled={!settings?.assistant_tts_enabled}
                    className="min-w-[340px]"
                  />
                </SettingContainer>
              </>
            )}

            {(settings?.assistant_tts_engine === "openai" ||
              settings?.assistant_tts_engine === "openrouter") && (
              <>
                {settings?.assistant_tts_engine === "openai" && (
                  <SettingContainer
                    title={t("settings.assistant.tts.baseUrlLabel")}
                    info={t("settings.assistant.tts.baseUrlDescription")}
                    layout="horizontal"
                    grouped={true}
                  >
                    <Input
                      type="text"
                      value={ttsBaseUrl}
                      onChange={(e) => setTtsBaseUrl(e.target.value)}
                      onBlur={() => {
                        void queueTtsTask(async () => {
                          await setAndRefresh(
                            commands.setAssistantTtsBaseUrl(ttsBaseUrl),
                          );
                        });
                      }}
                      placeholder="https://my-resource.openai.azure.com/openai/v1/audio/speech?api-version=2025-03-01-preview"
                      className="w-[340px]"
                    />
                  </SettingContainer>
                )}
                <SettingContainer
                  title={t("settings.assistant.tts.apiKeyLabel")}
                  info={t("settings.assistant.tts.apiKeyDescription")}
                  layout="horizontal"
                  grouped={true}
                >
                  <Input
                    type="password"
                    value={ttsApiKey}
                    onChange={(e) => setTtsApiKey(e.target.value)}
                    onBlur={() => {
                      void queueTtsTask(async () => {
                        await setAndRefresh(
                          commands.setAssistantTtsApiKey(ttsApiKey),
                        );
                      });
                    }}
                    className="w-[340px]"
                  />
                </SettingContainer>
                <SettingContainer
                  title={t("settings.assistant.tts.modelLabel")}
                  layout="horizontal"
                  grouped={true}
                >
                  <LoadableSelect
                    value={ttsModel}
                    options={ttsModelOptions}
                    onCommit={(v) => {
                      setTtsModel(v);
                      void queueTtsTask(async () => {
                        await setAndRefresh(commands.setAssistantTtsModel(v));
                      });
                    }}
                    onLoad={handleLoadTtsModels}
                    loading={ttsModelsLoading}
                    error={ttsModelsError}
                    placeholder="gpt-4o-mini-tts"
                    loadLabel={t("settings.assistant.tts.loadModels")}
                    formatCreateLabel={(input) =>
                      t("settings.assistant.tts.modelsUse", { model: input })
                    }
                  />
                </SettingContainer>
                <SettingContainer
                  title={t("settings.assistant.tts.remoteVoiceLabel")}
                  info={t("settings.assistant.tts.remoteVoiceDescription")}
                  layout="horizontal"
                  grouped={true}
                >
                  <LoadableSelect
                    value={ttsRemoteVoice}
                    options={ttsVoiceOptions}
                    onCommit={(v) => {
                      setTtsRemoteVoice(v);
                      void queueTtsTask(async () => {
                        await setAndRefresh(
                          commands.setAssistantTtsRemoteVoice(v),
                        );
                      });
                    }}
                    onLoad={handleLoadTtsVoices}
                    loading={ttsVoicesLoading}
                    error={ttsVoicesError}
                    placeholder={
                      ttsModel.toLowerCase().includes("gemini") &&
                      ttsModel.toLowerCase().includes("tts")
                        ? "Puck"
                        : "alloy"
                    }
                    loadLabel={t("settings.assistant.tts.loadVoices")}
                    formatCreateLabel={(input) =>
                      t("settings.assistant.tts.voicesUse", { voice: input })
                    }
                  />
                </SettingContainer>
              </>
            )}

            {settings?.assistant_tts_engine === "elevenlabs" && (
              <>
                <SettingContainer
                  title={t("settings.assistant.tts.apiKeyLabel")}
                  info={t("settings.assistant.tts.apiKeyDescription")}
                  layout="horizontal"
                  grouped={true}
                >
                  <Input
                    type="password"
                    value={ttsApiKey}
                    onChange={(e) => setTtsApiKey(e.target.value)}
                    onBlur={() => {
                      void queueTtsTask(async () => {
                        await setAndRefresh(
                          commands.setAssistantTtsApiKey(ttsApiKey),
                        );
                      });
                    }}
                    className="w-[340px]"
                  />
                </SettingContainer>
                <SettingContainer
                  title={t("settings.assistant.tts.elevenVoiceLabel")}
                  info={t("settings.assistant.tts.elevenVoiceDescription")}
                  layout="horizontal"
                  grouped={true}
                >
                  <LoadableSelect
                    value={ttsRemoteVoice}
                    options={ttsVoiceOptions}
                    onCommit={(v) => {
                      setTtsRemoteVoice(v);
                      void queueTtsTask(async () => {
                        await setAndRefresh(
                          commands.setAssistantTtsRemoteVoice(v),
                        );
                      });
                    }}
                    onLoad={handleLoadTtsVoices}
                    loading={ttsVoicesLoading}
                    error={ttsVoicesError}
                    placeholder="JBFqnCBsd6RMkjVDRZzb"
                    loadLabel={t("settings.assistant.tts.loadVoices")}
                    formatCreateLabel={(input) =>
                      t("settings.assistant.tts.voicesUse", { voice: input })
                    }
                  />
                </SettingContainer>
                <SettingContainer
                  title={t("settings.assistant.tts.modelLabel")}
                  description={t("settings.assistant.tts.modelDescription")}
                  descriptionMode="tooltip"
                  layout="horizontal"
                  grouped={true}
                >
                  <LoadableSelect
                    value={ttsModel}
                    options={ttsModelOptions}
                    onCommit={(v) => {
                      setTtsModel(v);
                      void queueTtsTask(async () => {
                        await setAndRefresh(commands.setAssistantTtsModel(v));
                      });
                    }}
                    onLoad={handleLoadTtsModels}
                    loading={ttsModelsLoading}
                    error={ttsModelsError}
                    placeholder="eleven_flash_v2_5"
                    loadLabel={t("settings.assistant.tts.loadModels")}
                    formatCreateLabel={(input) =>
                      t("settings.assistant.tts.modelsUse", { model: input })
                    }
                  />
                </SettingContainer>
              </>
            )}

            {settings?.assistant_tts_engine === "azure" && (
              <>
                <SettingContainer
                  title={t("settings.assistant.tts.azureBaseUrlLabel")}
                  info={t("settings.assistant.tts.azureBaseUrlDescription")}
                  layout="horizontal"
                  grouped={true}
                >
                  <Input
                    type="text"
                    value={ttsBaseUrl}
                    onChange={(e) => setTtsBaseUrl(e.target.value)}
                    onBlur={() => {
                      void queueTtsTask(async () => {
                        await setAndRefresh(
                          commands.setAssistantTtsBaseUrl(ttsBaseUrl),
                        );
                      });
                    }}
                    placeholder="https://eastus2.tts.speech.microsoft.com"
                    className="w-[340px]"
                  />
                </SettingContainer>
                <SettingContainer
                  title={t("settings.assistant.tts.apiKeyLabel")}
                  info={t("settings.assistant.tts.apiKeyDescription")}
                  layout="horizontal"
                  grouped={true}
                >
                  <Input
                    type="password"
                    value={ttsApiKey}
                    onChange={(e) => setTtsApiKey(e.target.value)}
                    onBlur={() => {
                      void queueTtsTask(async () => {
                        await setAndRefresh(
                          commands.setAssistantTtsApiKey(ttsApiKey),
                        );
                      });
                    }}
                    className="w-[340px]"
                  />
                </SettingContainer>
                <SettingContainer
                  title={t("settings.assistant.tts.azureVoiceLabel")}
                  info={t("settings.assistant.tts.azureVoiceDescription")}
                  layout="horizontal"
                  grouped={true}
                >
                  <LoadableSelect
                    value={ttsRemoteVoice}
                    options={ttsVoiceOptions}
                    onCommit={(v) => {
                      setTtsRemoteVoice(v);
                      void queueTtsTask(async () => {
                        await setAndRefresh(
                          commands.setAssistantTtsRemoteVoice(v),
                        );
                      });
                    }}
                    onLoad={handleLoadTtsVoices}
                    loading={ttsVoicesLoading}
                    error={ttsVoicesError}
                    placeholder="en-US-JennyNeural"
                    loadLabel={t("settings.assistant.tts.loadVoices")}
                    formatCreateLabel={(input) =>
                      t("settings.assistant.tts.voicesUse", { voice: input })
                    }
                  />
                </SettingContainer>
              </>
            )}

            <SettingContainer
              title={t("settings.assistant.tts.speedLabel")}
              info={t("settings.assistant.tts.speedDescription")}
              layout="horizontal"
              grouped={true}
            >
              <div className="flex items-center gap-1.5">
                {TTS_SPEED_PRESETS.map((preset) => {
                  const active = Math.abs(currentTtsSpeed - preset) < 0.001;
                  return (
                    <button
                      key={preset}
                      type="button"
                      onClick={() => {
                        void queueTtsTask(() => commitTtsSpeed(preset));
                      }}
                      disabled={!settings?.assistant_tts_enabled}
                      className={`px-2.5 py-1 text-[13px] font-medium rounded-md transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed ${
                        active
                          ? "bg-accent/12 text-accent"
                          : "bg-surface-strong text-muted hover:text-ink"
                      }`}
                    >
                      {t("settings.assistant.tts.speedValue", {
                        value: preset,
                      })}
                    </button>
                  );
                })}
                <Input
                  type="number"
                  value={ttsSpeedInput}
                  onChange={(e) => setTtsSpeedInput(e.target.value)}
                  onBlur={handleTtsSpeedBlur}
                  min="0.25"
                  max="4"
                  step="0.1"
                  disabled={!settings?.assistant_tts_enabled}
                  aria-label={t("settings.assistant.tts.speedCustomLabel")}
                  className="w-20"
                />
              </div>
            </SettingContainer>

            <SettingContainer
              title={t("settings.assistant.tts.testLabel")}
              layout="horizontal"
              grouped={true}
            >
              <div className="flex flex-col items-end gap-1">
                <button
                  type="button"
                  onClick={handleTestTts}
                  disabled={
                    !settings?.assistant_tts_enabled || testState === "testing"
                  }
                  className="flex items-center gap-2 px-3 py-1.5 rounded-lg border border-hairline-strong bg-surface hover:bg-surface-strong disabled:opacity-50 disabled:cursor-not-allowed text-[13px] font-medium cursor-pointer transition-colors"
                >
                  <Volume2 size={14} />
                  {testState === "testing"
                    ? t("settings.assistant.tts.testing")
                    : testState === "ok"
                      ? t("settings.assistant.tts.testOk")
                      : t("settings.assistant.tts.testButton")}
                </button>
                {testState === "error" && testError && (
                  <span className="text-xs text-error max-w-[360px] text-right break-words">
                    {testError}
                  </span>
                )}
              </div>
            </SettingContainer>

            <ToggleSwitch
              checked={settings?.assistant_tts_stop_on_dictation ?? false}
              onChange={(checked) =>
                setAndRefresh(commands.setAssistantTtsStopOnDictation(checked))
              }
              label={t("settings.assistant.tts.stopOnDictationLabel")}
              grouped={true}
            />
            {(settings?.assistant_tts_engine ?? "kokoro") === "kokoro" && (
              <SettingContainer
                title={t("settings.assistant.tts.dtypeLabel")}
                info={t("settings.assistant.tts.dtypeDescription")}
                layout="horizontal"
                grouped={true}
              >
                <Dropdown
                  options={KOKORO_DTYPES}
                  selectedValue={settings?.assistant_tts_kokoro_dtype ?? "fp32"}
                  onSelect={(dtype) =>
                    setAndRefresh(commands.setAssistantTtsKokoroDtype(dtype))
                  }
                  disabled={!settings?.assistant_tts_enabled}
                />
              </SettingContainer>
            )}
          </>
        )}
      </SettingsGroup>

      {/* Screen vision ---------------------------------------------------- */}
      <SettingsGroup
        title={t("settings.assistant.vision.title")}
        icon={Monitor}
      >
        <SettingContainer
          title={t("settings.assistant.vision.modeLabel")}
          info={screenAccessDescription}
          layout="horizontal"
          grouped={true}
        >
          <Dropdown
            options={[
              {
                value: "off",
                label: t("settings.assistant.vision.modes.off"),
              },
              {
                value: "manual",
                label: t("settings.assistant.vision.modes.manual"),
              },
              {
                value: "agent_decides",
                label: t("settings.assistant.vision.modes.agentDecides"),
              },
            ]}
            selectedValue={screenAccessMode}
            onSelect={(mode) =>
              setAndRefresh(
                commands.setAssistantScreenAccessMode(
                  mode as AssistantScreenAccessMode,
                ),
              )
            }
          />
        </SettingContainer>
        {manualScreenAccess && (
          <SettingContainer
            title={t("settings.assistant.vision.timing.label")}
            info={t("settings.assistant.vision.timing.description")}
            layout="horizontal"
            grouped={true}
          >
            <Dropdown
              options={[
                {
                  value: "immediate",
                  label: t(
                    "settings.assistant.vision.timing.options.immediate",
                  ),
                },
                {
                  value: "on_send",
                  label: t("settings.assistant.vision.timing.options.on_send"),
                },
              ]}
              selectedValue={
                settings?.assistant_vision_capture_timing ?? "immediate"
              }
              onSelect={(value) =>
                setAndRefresh(
                  commands.setAssistantVisionCaptureTiming(
                    value as VisionCaptureTiming,
                  ),
                )
              }
            />
          </SettingContainer>
        )}
        {screenAccessMode !== "off" && <ScreenRecordingPermission />}
      </SettingsGroup>

      {/* Web search ------------------------------------------------------- */}
      <SettingsGroup
        title={t("settings.assistant.webSearch.title")}
        icon={Globe}
      >
        <ToggleSwitch
          checked={webSearchEnabled}
          onChange={(checked) =>
            setAndRefresh(commands.setAssistantWebSearchEnabled(checked))
          }
          label={t("settings.assistant.webSearch.enableLabel")}
          description={t("settings.assistant.webSearch.enableDescription")}
          grouped={true}
        />
        {webSearchEnabled && (
          <>
            {selectedProviderId === "openrouter" && (
              <ToggleSwitch
                checked={settings?.assistant_prefer_provider_web_search ?? true}
                onChange={(checked) =>
                  setAndRefresh(
                    commands.setAssistantPreferProviderWebSearch(checked),
                  )
                }
                label={t("settings.assistant.webSearch.openRouterNativeLabel")}
                info={t(
                  "settings.assistant.webSearch.openRouterNativeDescription",
                )}
                grouped={true}
              />
            )}
            <SettingContainer
              title={t("settings.assistant.webSearch.providerLabel")}
              info={t("settings.assistant.webSearch.providerDescription")}
              layout="horizontal"
              grouped={true}
            >
              <Dropdown
                options={[
                  {
                    value: "serper",
                    label: t("settings.assistant.webSearch.providers.serper"),
                  },
                  {
                    value: "brave",
                    label: t("settings.assistant.webSearch.providers.brave"),
                  },
                  {
                    value: "tavily",
                    label: t("settings.assistant.webSearch.providers.tavily"),
                  },
                  {
                    value: "exa",
                    label: t("settings.assistant.webSearch.providers.exa"),
                  },
                  {
                    value: "serpapi",
                    label: t("settings.assistant.webSearch.providers.serpapi"),
                  },
                ]}
                selectedValue={webSearchProvider}
                onSelect={(provider) =>
                  setAndRefresh(
                    commands.setAssistantWebSearchProvider(provider),
                  )
                }
                disabled={!webSearchEnabled}
              />
            </SettingContainer>

            {webSearchNeedsKey && (
              <SettingContainer
                title={t("settings.assistant.webSearch.apiKeyLabel")}
                layout="horizontal"
                grouped={true}
              >
                <Input
                  type="password"
                  value={webSearchApiKey}
                  onChange={(e) => setWebSearchApiKey(e.target.value)}
                  onBlur={handleWebSearchApiKeyBlur}
                  placeholder={t(
                    "settings.assistant.webSearch.apiKeyPlaceholder",
                  )}
                  className="min-w-[320px]"
                  disabled={!webSearchEnabled}
                />
              </SettingContainer>
            )}

            <SettingContainer
              title={t("settings.assistant.webSearch.testLabel")}
              layout="horizontal"
              grouped={true}
            >
              <div className="flex flex-col items-end gap-1">
                <button
                  type="button"
                  onClick={handleTestWebSearch}
                  disabled={!webSearchEnabled || webSearchTest === "testing"}
                  className="flex items-center gap-2 px-3 py-1.5 rounded-lg border border-hairline-strong bg-surface hover:bg-surface-strong disabled:opacity-50 disabled:cursor-not-allowed text-[13px] font-medium cursor-pointer transition-colors"
                >
                  <Globe size={14} />
                  {webSearchTest === "testing"
                    ? t("settings.assistant.webSearch.testing")
                    : t("settings.assistant.webSearch.testButton")}
                </button>
                {webSearchTestMsg && (
                  <span
                    className={`text-xs max-w-[360px] text-right break-words ${
                      webSearchTest === "error"
                        ? "text-error"
                        : "text-muted-soft"
                    }`}
                  >
                    {webSearchTestMsg}
                  </span>
                )}
              </div>
            </SettingContainer>

            <SettingContainer
              title={t("settings.assistant.webSearch.depthLabel")}
              info={t("settings.assistant.webSearch.depthDescription")}
              layout="horizontal"
              grouped={true}
            >
              <Dropdown
                options={[
                  {
                    value: "low",
                    label: t("settings.assistant.webSearch.depthOptions.low"),
                  },
                  {
                    value: "medium",
                    label: t(
                      "settings.assistant.webSearch.depthOptions.medium",
                    ),
                  },
                  {
                    value: "high",
                    label: t("settings.assistant.webSearch.depthOptions.high"),
                  },
                ]}
                selectedValue={settings?.assistant_search_depth ?? "medium"}
                onSelect={(depth) =>
                  setAndRefresh(
                    commands.setAssistantSearchDepth(
                      depth as AssistantSearchDepth,
                    ),
                  )
                }
                disabled={!webSearchEnabled}
              />
            </SettingContainer>

            {selectedProviderId === "builtin" && (
              <ToggleSwitch
                checked={settings?.assistant_local_search_smart ?? false}
                onChange={(checked) =>
                  setAndRefresh(commands.setAssistantLocalSearchSmart(checked))
                }
                label={t("settings.assistant.webSearch.localSmartLabel")}
                info={t("settings.assistant.webSearch.localSmartDescription")}
                grouped={true}
                disabled={!webSearchEnabled}
              />
            )}
          </>
        )}
      </SettingsGroup>

      {/* Panel appearance -------------------------------------------------- */}
      <SettingsGroup
        title={t("settings.assistant.appearance.title")}
        icon={PanelTop}
      >
        <SettingContainer
          title={t("settings.assistant.appearance.previewLabel")}
          layout="stacked"
          grouped={true}
        >
          <PanelPreview
            fontSize={settings?.assistant_font_size ?? "medium"}
            opacity={settings?.assistant_panel_opacity ?? 1}
          />
        </SettingContainer>
        <SettingContainer
          title={t("settings.assistant.appearance.fontSizeLabel")}
          layout="horizontal"
          grouped={true}
        >
          <Dropdown
            options={[
              {
                value: "small",
                label: t("settings.assistant.appearance.fontSizes.small"),
              },
              {
                value: "medium",
                label: t("settings.assistant.appearance.fontSizes.medium"),
              },
              {
                value: "large",
                label: t("settings.assistant.appearance.fontSizes.large"),
              },
            ]}
            selectedValue={settings?.assistant_font_size ?? "medium"}
            onSelect={(size) =>
              setAndRefresh(commands.setAssistantFontSize(size))
            }
          />
        </SettingContainer>
        <SettingContainer
          title={t("settings.assistant.appearance.panelSizeLabel")}
          info={t("settings.assistant.appearance.panelSizeDescription")}
          layout="horizontal"
          grouped={true}
        >
          <Dropdown
            options={[
              {
                value: "mini",
                label: t("settings.assistant.appearance.panelSizes.mini"),
              },
              {
                value: "compact",
                label: t("settings.assistant.appearance.panelSizes.compact"),
              },
              {
                value: "standard",
                label: t("settings.assistant.appearance.panelSizes.standard"),
              },
              {
                value: "large",
                label: t("settings.assistant.appearance.panelSizes.large"),
              },
            ]}
            selectedValue={settings?.assistant_panel_size ?? "standard"}
            onSelect={(size) =>
              setAndRefresh(commands.setAssistantPanelSize(size))
            }
          />
        </SettingContainer>
        <Slider
          value={settings?.assistant_panel_opacity ?? 1}
          onChange={(value) =>
            setAndRefresh(commands.setAssistantPanelOpacity(value))
          }
          min={0.5}
          max={1}
          step={0.05}
          label={t("settings.assistant.appearance.opacityLabel")}
          info={t("settings.assistant.appearance.opacityDescription")}
          grouped={true}
          controlClassName="w-[200px]"
          formatValue={(v) => `${Math.round(v * 100)}%`}
        />
      </SettingsGroup>

      {/* Reply behavior ---------------------------------------------------- */}
      <SettingsGroup title={t("settings.assistant.behavior.title")}>
        <SettingContainer
          title={t("settings.assistant.responseLength.label")}
          info={t("settings.assistant.responseLength.description")}
          layout="horizontal"
          grouped={true}
        >
          <Dropdown
            options={[
              {
                value: "default",
                label: t("settings.assistant.responseLength.options.default"),
              },
              {
                value: "short",
                label: t("settings.assistant.responseLength.options.short"),
              },
              {
                value: "medium",
                label: t("settings.assistant.responseLength.options.medium"),
              },
              {
                value: "long",
                label: t("settings.assistant.responseLength.options.long"),
              },
            ]}
            selectedValue={settings?.assistant_response_length ?? "default"}
            onSelect={(value) =>
              setAndRefresh(
                commands.setAssistantResponseLength(
                  value as AssistantResponseLength,
                ),
              )
            }
          />
        </SettingContainer>
        <SettingContainer
          title={t("settings.assistant.memory.label")}
          description={t("settings.assistant.memory.description")}
          layout="horizontal"
          grouped={true}
        >
          <Input
            type="number"
            min={0}
            max={200}
            value={historyLimit}
            onChange={(e) => setHistoryLimit(e.target.value)}
            onBlur={handleHistoryLimitBlur}
            className="w-[120px]"
          />
        </SettingContainer>
        <ToggleSwitch
          checked={settings?.assistant_auto_summarize ?? true}
          onChange={(value) =>
            setAndRefresh(commands.setAssistantAutoSummarize(value))
          }
          label={t("settings.assistant.autoSummarize.label")}
          description={t("settings.assistant.autoSummarize.description")}
          grouped={true}
        />
      </SettingsGroup>
    </div>
  );
};
