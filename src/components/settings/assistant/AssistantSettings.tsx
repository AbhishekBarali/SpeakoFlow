import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { RefreshCw, Volume2, ArrowUp, Globe } from "lucide-react";
import {
  commands,
  type TtsVoice,
  type LocalLlmStatus,
  type AssistantResponseLength,
  type AssistantSearchDepth,
  type ModelUnloadTimeout,
  type VisionCaptureTiming,
} from "@/bindings";
import {
  Dropdown,
  MoreOptions,
  SettingContainer,
  SettingsGroup,
  Slider,
  ToggleSwitch,
} from "@/components/ui";
import { Input } from "../../ui/Input";
import { ShortcutInput } from "../ShortcutInput";
import { TapToLock } from "../TapToLock";
import { useSettings } from "../../../hooks/useSettings";
import { useKokoroTts } from "../../../assistant/useKokoroTts";
import { FONT_SIZES } from "../../../assistant/appearance";
import "../../../assistant/AssistantPanel.css";
import { useModelStore } from "@/stores/modelStore";
import { getModelCategory } from "@/lib/utils/modelCategory";

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

/** An editable text field with type-to-search suggestions (native `<datalist>`)
 *  paired with a "Load" (refresh) button and an inline error line. Used for the
 *  remote-TTS voice/model pickers and the assistant model picker.
 *
 *  A plain editable input (not a react-select value chip) is deliberate: the
 *  value stays fully editable — cursor at the end, edit any character — which
 *  is what you want for tweaking a model name like `gpt-5.1-mini`, while the
 *  datalist still filters suggestions as you type. Module-scope so it isn't
 *  recreated on every parent render. */
const LoadableSelect: React.FC<{
  value: string;
  options: { value: string; label: string }[];
  onCommit: (value: string) => void;
  onLoad: () => void;
  loading: boolean;
  error: string | null;
  placeholder: string;
  loadLabel: string;
  disabled?: boolean;
  /** Unused; kept for call-site compatibility. */
  formatCreateLabel?: (input: string) => string;
}> = ({
  value,
  options,
  onCommit,
  onLoad,
  loading,
  error,
  placeholder,
  loadLabel,
  disabled,
}) => {
  const listId = React.useId();
  const [local, setLocal] = React.useState(value);
  React.useEffect(() => setLocal(value), [value]);

  const commit = (next: string) => {
    const trimmed = next.trim();
    if (trimmed && trimmed !== value.trim()) onCommit(trimmed);
  };

  return (
    <div className="flex flex-col items-end gap-1">
      <div className="flex items-center gap-2">
        <Input
          type="text"
          list={listId}
          value={local}
          onChange={(e) => {
            const next = e.target.value;
            setLocal(next);
            // Picking a suggestion from the datalist matches an option exactly —
            // commit immediately so a click doesn't require an extra blur.
            if (options.some((o) => o.value === next)) commit(next);
          }}
          onBlur={() => commit(local)}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit(local);
          }}
          placeholder={placeholder}
          disabled={disabled}
          className="min-w-[300px]"
        />
        <datalist id={listId}>
          {options.map((o) => (
            <option
              key={o.value}
              value={o.value}
              label={o.label !== o.value ? o.label : undefined}
            />
          ))}
        </datalist>
        <button
          type="button"
          onClick={onLoad}
          disabled={loading || disabled}
          aria-label={loadLabel}
          title={loadLabel}
          className="flex h-10 w-10 items-center justify-center rounded-lg border border-mid-gray/30 hover:bg-mid-gray/10 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <RefreshCw size={14} className={loading ? "animate-spin" : ""} />
        </button>
      </div>
      {error && (
        <span className="text-xs text-red-500 max-w-[360px] text-right break-words">
          {error}
        </span>
      )}
    </div>
  );
};

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

export const AssistantSettings: React.FC = () => {
  const { t } = useTranslation();
  const { settings, refreshSettings, updatePostProcessApiKey } = useSettings();

  const providers = settings?.post_process_providers || [];
  const selectedProviderId = settings?.assistant_provider_id || "custom";
  const selectedProvider = providers.find((p) => p.id === selectedProviderId);

  // Built-in (local) provider: model is chosen from downloaded LLM models and
  // there is no API key. The engine is the bundled llama.cpp sidecar.
  const isBuiltin = selectedProviderId === "builtin";
  const { models } = useModelStore();
  const llmModels = useMemo(
    () =>
      models.filter((m) => getModelCategory(m) === "llm" && m.is_downloaded),
    [models],
  );
  const [localLlmStatus, setLocalLlmStatus] = useState<LocalLlmStatus | null>(
    null,
  );
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

  // Load selectable voices / models for the current remote TTS engine
  // (OpenAI-compatible or ElevenLabs; Azure has its own loader above).
  const handleLoadTtsVoices = async () => {
    setTtsVoicesLoading(true);
    setTtsVoicesError(null);
    try {
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

  const ttsEngine = settings?.assistant_tts_engine ?? "kokoro";
  const ttsEnabled = settings?.assistant_tts_enabled ?? false;
  const ttsVoice = settings?.assistant_tts_voice ?? "af_heart";
  const ttsDtype = settings?.assistant_tts_kokoro_dtype ?? "fp32";
  const ttsSpeed = settings?.assistant_tts_speed ?? 1;
  // Preload the local Kokoro voice as soon as it's the selected engine (and TTS
  // is on), so its ~80 MB of weights are downloaded and cached here — before
  // the user ever opens the assistant — instead of stalling the first spoken
  // reply. The same instance still backs the Test button, which force-speaks
  // regardless of `enabled`, so a test reuses the already-loaded model.
  const preloadKokoro = ttsEnabled && ttsEngine === "kokoro";
  const kokoroTest = useKokoroTts(preloadKokoro, ttsVoice, ttsDtype, ttsSpeed);

  // Small "downloading voice… → ready" toast so the background download isn't
  // invisible. A cache hit resolves well under the delay below, so no toast
  // flashes on every visit once the model is already downloaded.
  const {
    status: kokoroStatus,
    progress: kokoroProgress,
    error: kokoroError,
  } = kokoroTest;
  const kokoroToastIdRef = useRef<string | number | null>(null);
  const kokoroToastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  useEffect(() => {
    const TOAST_ID = "kokoro-voice-download";

    if (kokoroStatus === "loading") {
      if (kokoroToastIdRef.current != null) {
        // Already showing — keep the percentage fresh as it climbs.
        toast.loading(
          t("settings.assistant.tts.downloadProgress", {
            progress: kokoroProgress,
          }),
          { id: TOAST_ID },
        );
      } else if (kokoroToastTimerRef.current == null) {
        // Defer surfacing the toast so a fast cache hit never flashes one.
        kokoroToastTimerRef.current = setTimeout(() => {
          kokoroToastTimerRef.current = null;
          kokoroToastIdRef.current = toast.loading(
            t("settings.assistant.tts.downloadStart"),
            { id: TOAST_ID },
          );
        }, 600);
      }
      return;
    }

    // Left the loading state: cancel a still-pending (fast download) timer.
    if (kokoroToastTimerRef.current != null) {
      clearTimeout(kokoroToastTimerRef.current);
      kokoroToastTimerRef.current = null;
    }
    // Only report an outcome if we actually surfaced a download in progress.
    if (kokoroToastIdRef.current != null) {
      if (kokoroStatus === "error" || kokoroError?.reason === "load") {
        toast.error(t("settings.assistant.tts.downloadError"), {
          id: TOAST_ID,
        });
      } else {
        toast.success(t("settings.assistant.tts.downloadReady"), {
          id: TOAST_ID,
        });
      }
      kokoroToastIdRef.current = null;
    }
  }, [kokoroStatus, kokoroProgress, kokoroError, t]);

  // Tidy up a pending timer / lingering toast if the user leaves this page
  // mid-download.
  useEffect(
    () => () => {
      if (kokoroToastTimerRef.current != null) {
        clearTimeout(kokoroToastTimerRef.current);
      }
      if (kokoroToastIdRef.current != null) {
        toast.dismiss(kokoroToastIdRef.current);
      }
    },
    [],
  );

  const handleTestTts = async () => {
    setTestState("testing");
    setTestError(null);
    const phrase = randomTestPhrase();
    try {
      if (ttsEngine === "kokoro") {
        await kokoroTest.speak(phrase, true);
      } else {
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

  const providerOptions = providers
    .filter((p) => p.id !== "apple_intelligence")
    .map((p) => ({ value: p.id, label: p.label }))
    // Keep the built-in local model pinned to the top — it's the zero-setup,
    // no-API-key option most users should reach for first.
    .sort((a, b) =>
      a.value === "builtin" ? -1 : b.value === "builtin" ? 1 : 0,
    );

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

  const handleProviderSelect = async (providerId: string) => {
    await commands.setAssistantProvider(providerId);
    await refreshSettings();
  };

  const handleApiKeyBlur = async () => {
    await updatePostProcessApiKey(selectedProviderId, apiKey);
  };

  const handleBaseUrlBlur = async () => {
    await commands.changePostProcessBaseUrlSetting(selectedProviderId, baseUrl);
    await refreshSettings();
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
      void commitTtsSpeed(parsed);
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

  return (
    <div className="max-w-2xl w-full mx-auto space-y-8">
      <SettingsGroup title={t("settings.assistant.shortcuts.title")}>
        <ShortcutInput shortcutId="assistant" grouped={true} />
        <ShortcutInput shortcutId="assistant_panel_toggle" grouped={true} />
        <TapToLock
          grouped={true}
          settingKey="assistant_tap_to_lock_key"
          fallback="shift"
          labelKey="settings.assistant.tapToLock.label"
          infoKey="settings.assistant.tapToLock.info"
          offKey="settings.assistant.tapToLock.off"
          clearKey="settings.assistant.tapToLock.clear"
        />
      </SettingsGroup>

      <SettingsGroup title={t("settings.assistant.provider.title")}>
        <SettingContainer
          title={t("settings.assistant.provider.providerLabel")}
          layout="horizontal"
          grouped={true}
        >
          <Dropdown
            options={providerOptions}
            selectedValue={selectedProviderId}
            onSelect={handleProviderSelect}
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
              className="min-w-[380px]"
            />
          </SettingContainer>
        )}

        {!isBuiltin && (
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
              className="min-w-[320px]"
            />
          </SettingContainer>
        )}

        {isBuiltin ? (
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
                  className="min-w-[320px]"
                />
              ) : (
                <span className="text-xs text-muted-soft max-w-[360px] text-right">
                  {t("settings.assistant.provider.builtinNoModels")}
                </span>
              )}
              {localLlmStatus && !localLlmStatus.engine_present ? (
                <span className="text-xs text-amber-500 max-w-[360px] text-right">
                  {t("settings.assistant.provider.builtinEngineMissing")}
                </span>
              ) : (
                <span className="text-xs text-muted-soft max-w-[360px] text-right">
                  {t("settings.assistant.provider.builtinReady")}
                </span>
              )}
            </div>
          </SettingContainer>
        ) : (
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
        )}

        {isBuiltin && (
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
        )}

        {isBuiltin && (
          <SettingContainer
            title={t("settings.assistant.provider.unloadTimeoutLabel")}
            info={t("settings.assistant.provider.unloadTimeoutDescription")}
            layout="horizontal"
            grouped={true}
          >
            <Dropdown
              options={llmUnloadOptions}
              selectedValue={
                settings?.local_llm_unload_timeout ??
                ("min5" as ModelUnloadTimeout)
              }
              onSelect={(value) =>
                setAndRefresh(
                  commands.setLocalLlmUnloadTimeout(
                    value as ModelUnloadTimeout,
                  ),
                )
              }
              className="min-w-[200px]"
            />
          </SettingContainer>
        )}
      </SettingsGroup>

      <SettingsGroup title={t("settings.assistant.vision.title")}>
        <ToggleSwitch
          checked={settings?.assistant_screenshot_enabled ?? true}
          onChange={(checked) =>
            setAndRefresh(commands.setAssistantScreenshotEnabled(checked))
          }
          label={t("settings.assistant.vision.enableLabel")}
          info={t("settings.assistant.vision.enableDescription")}
          grouped={true}
        />
        {(settings?.assistant_screenshot_enabled ?? true) && (
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
                  label: t("settings.assistant.vision.timing.options.immediate"),
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
      </SettingsGroup>

      <SettingsGroup title={t("settings.assistant.webSearch.title")}>
        <ToggleSwitch
          checked={webSearchEnabled}
          onChange={(checked) =>
            setAndRefresh(commands.setAssistantWebSearchEnabled(checked))
          }
          label={t("settings.assistant.webSearch.enableLabel")}
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

            <MoreOptions>
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
                      label: t(
                        "settings.assistant.webSearch.depthOptions.high",
                      ),
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

              {settings?.assistant_provider_id === "builtin" && (
                <ToggleSwitch
                  checked={settings?.assistant_local_search_smart ?? false}
                  onChange={(checked) =>
                    setAndRefresh(
                      commands.setAssistantLocalSearchSmart(checked),
                    )
                  }
                  label={t("settings.assistant.webSearch.localSmartLabel")}
                  info={t("settings.assistant.webSearch.localSmartDescription")}
                  grouped={true}
                  disabled={!webSearchEnabled}
                />
              )}
            </MoreOptions>
          </>
        )}
      </SettingsGroup>

      <SettingsGroup title={t("settings.assistant.tts.title")}>
        <ToggleSwitch
          checked={settings?.assistant_tts_enabled ?? false}
          onChange={(checked) =>
            setAndRefresh(commands.setAssistantTtsEnabled(checked))
          }
          label={t("settings.assistant.tts.enableLabel")}
          grouped={true}
        />
        {settings?.assistant_tts_enabled && (
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
                    value: "elevenlabs",
                    label: t("settings.assistant.tts.engines.elevenlabs"),
                  },
                  {
                    value: "azure",
                    label: t("settings.assistant.tts.engines.azure"),
                  },
                ]}
                selectedValue={settings?.assistant_tts_engine ?? "kokoro"}
                onSelect={(engine) =>
                  setAndRefresh(commands.setAssistantTtsEngine(engine))
                }
                disabled={!settings?.assistant_tts_enabled}
                className="min-w-[260px]"
              />
            </SettingContainer>

            {(settings?.assistant_tts_engine ?? "kokoro") === "kokoro" && (
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
                />
              </SettingContainer>
            )}

            {settings?.assistant_tts_engine === "openai" && (
              <>
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
                    onBlur={() =>
                      setAndRefresh(commands.setAssistantTtsBaseUrl(ttsBaseUrl))
                    }
                    placeholder="https://my-resource.openai.azure.com/openai/v1/audio/speech?api-version=2025-03-01-preview"
                    className="min-w-[360px]"
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
                    onBlur={() =>
                      setAndRefresh(commands.setAssistantTtsApiKey(ttsApiKey))
                    }
                    className="min-w-[300px]"
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
                      void setAndRefresh(commands.setAssistantTtsModel(v));
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
                      void setAndRefresh(
                        commands.setAssistantTtsRemoteVoice(v),
                      );
                    }}
                    onLoad={handleLoadTtsVoices}
                    loading={ttsVoicesLoading}
                    error={ttsVoicesError}
                    placeholder="alloy"
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
                    onBlur={() =>
                      setAndRefresh(commands.setAssistantTtsApiKey(ttsApiKey))
                    }
                    className="min-w-[300px]"
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
                      void setAndRefresh(
                        commands.setAssistantTtsRemoteVoice(v),
                      );
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
                      void setAndRefresh(commands.setAssistantTtsModel(v));
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
                    onBlur={() =>
                      setAndRefresh(commands.setAssistantTtsBaseUrl(ttsBaseUrl))
                    }
                    placeholder="https://eastus2.tts.speech.microsoft.com"
                    className="min-w-[360px]"
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
                    onBlur={() =>
                      setAndRefresh(commands.setAssistantTtsApiKey(ttsApiKey))
                    }
                    className="min-w-[300px]"
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
                      void setAndRefresh(
                        commands.setAssistantTtsRemoteVoice(v),
                      );
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
                      onClick={() => commitTtsSpeed(preset)}
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

            <MoreOptions>
              <ToggleSwitch
                checked={settings?.assistant_tts_stop_on_dictation ?? false}
                onChange={(checked) =>
                  setAndRefresh(
                    commands.setAssistantTtsStopOnDictation(checked),
                  )
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
                    selectedValue={
                      settings?.assistant_tts_kokoro_dtype ?? "fp32"
                    }
                    onSelect={(dtype) =>
                      setAndRefresh(commands.setAssistantTtsKokoroDtype(dtype))
                    }
                    disabled={!settings?.assistant_tts_enabled}
                  />
                </SettingContainer>
              )}
            </MoreOptions>
          </>
        )}
      </SettingsGroup>

      <SettingsGroup title={t("settings.assistant.appearance.title")}>
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
      </SettingsGroup>
    </div>
  );
};
