import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw, Volume2, ArrowUp, Globe } from "lucide-react";
import {
  commands,
  type TtsVoice,
  type LocalLlmStatus,
  type AssistantResponseLength,
  type AssistantSearchDepth,
} from "@/bindings";
import {
  Dropdown,
  SettingContainer,
  SettingsGroup,
  Slider,
  Textarea,
  ToggleSwitch,
} from "@/components/ui";
import { Input } from "../../ui/Input";
import { ShortcutInput } from "../ShortcutInput";
import { useSettings } from "../../../hooks/useSettings";
import { useKokoroTts } from "../../../assistant/useKokoroTts";
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

const ACCENTS: Record<string, [string, string]> = {
  violet: ["#6366f1", "#8b5cf6"],
  blue: ["#2563eb", "#06b6d4"],
  emerald: ["#059669", "#34d399"],
  rose: ["#e11d48", "#ec4899"],
  amber: ["#d97706", "#f59e0b"],
  mono: ["#52525b", "#71717a"],
};

const FONT_SIZES: Record<string, string> = {
  small: "12px",
  medium: "13px",
  large: "14.5px",
};

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

/** Explicit dark/light palettes for the live preview — mirror the real panel
 *  tokens in AssistantPanel.css so the preview is truthful for both themes
 *  regardless of the settings window's own appearance. */
const PREVIEW_THEMES: Record<
  "light" | "dark",
  {
    surface: string;
    surfaceStrong: string;
    ink: string;
    hairline: string;
    hairlineStrong: string;
    mutedSoft: string;
    inkPillBg: string;
    inkPillFg: string;
  }
> = {
  dark: {
    surface: "#1c1917",
    surfaceStrong: "#292524",
    ink: "#fafaf9",
    hairline: "#292524",
    hairlineStrong: "#44403c",
    mutedSoft: "#78716c",
    inkPillBg: "#fafaf9",
    inkPillFg: "#0c0a09",
  },
  light: {
    surface: "#fdfcfa",
    surfaceStrong: "#eeebe4",
    ink: "#2a2622",
    hairline: "#e9e5dd",
    hairlineStrong: "#dad4c8",
    mutedSoft: "#b7af9f",
    inkPillBg: "#3d362e",
    inkPillFg: "#fdfcfa",
  },
};

/** Resolve the panel theme preference to a concrete light/dark for the preview;
 *  "auto" follows the settings window's current (app) theme. */
const resolvePreviewTheme = (pref: string | undefined): "light" | "dark" => {
  const p = pref ?? "auto";
  if (p === "light" || p === "dark") return p;
  return typeof document !== "undefined" &&
    document.documentElement.dataset.theme === "dark"
    ? "dark"
    : "light";
};

/** Live preview of the assistant panel using the current appearance
 *  settings — mirrors the bubble/input styling of the real panel. */
const PanelPreview: React.FC<{
  accent: string;
  fontSize: string;
  opacity: number;
  theme: "light" | "dark";
}> = ({ accent, fontSize, opacity, theme }) => {
  const { t } = useTranslation();
  const [from, to] = ACCENTS[accent] ?? ACCENTS.violet;
  const fs = FONT_SIZES[fontSize] ?? FONT_SIZES.medium;
  const accentGradient = `linear-gradient(135deg, ${from}, ${to})`;
  const c = PREVIEW_THEMES[theme];

  return (
    <div
      className="rounded-2xl p-3 flex flex-col gap-2"
      style={{
        opacity: Math.max(opacity, 0.5),
        background: c.surface,
        border: `1px solid ${c.hairline}`,
      }}
    >
      <div className="flex items-center gap-2 pb-1">
        <span
          className="w-1.5 h-1.5 rounded-full shrink-0"
          style={{ background: accentGradient }}
        />
        <span
          className="font-display font-medium text-[15px] leading-none"
          style={{ color: c.ink }}
        >
          {t("assistant.title")}
        </span>
      </div>
      <div
        className="self-end max-w-[75%] rounded-2xl rounded-br-md px-3 py-1.5"
        style={{ fontSize: fs, background: c.inkPillBg, color: c.inkPillFg }}
      >
        {t("settings.assistant.appearance.previewUser")}
      </div>
      <div
        className="self-start max-w-[75%] rounded-2xl rounded-bl-md px-3 py-1.5"
        style={{
          fontSize: fs,
          background: c.surfaceStrong,
          border: `1px solid ${c.hairline}`,
          color: c.ink,
        }}
      >
        {t("settings.assistant.appearance.previewAssistant")}
      </div>
      <div className="flex items-center gap-2 mt-1">
        <div
          className="flex-1 h-9 rounded-xl px-3 flex items-center"
          style={{
            fontSize: fs,
            background: c.surfaceStrong,
            border: `1px solid ${c.hairlineStrong}`,
            color: c.mutedSoft,
          }}
        >
          {t("assistant.inputPlaceholder")}
        </div>
        <div
          className="w-9 h-9 rounded-full flex items-center justify-center shrink-0"
          style={{ background: c.inkPillBg, color: c.inkPillFg }}
        >
          <ArrowUp size={15} strokeWidth={2.5} />
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
  const [systemPrompt, setSystemPrompt] = useState("");
  const [historyLimit, setHistoryLimit] = useState("12");
  const [contextSize, setContextSize] = useState("4096");
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
  const ttsVoice = settings?.assistant_tts_voice ?? "af_heart";
  const ttsDtype = settings?.assistant_tts_kokoro_dtype ?? "fp32";
  const ttsSpeed = settings?.assistant_tts_speed ?? 1;
  // Lazy (not preloaded) Kokoro instance used only by the Test button in this
  // settings window; force-speaks regardless of the enabled toggle.
  const kokoroTest = useKokoroTts(false, ttsVoice, ttsDtype, ttsSpeed);

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
    setSystemPrompt(settings?.assistant_system_prompt ?? "");
  }, [settings?.assistant_system_prompt]);

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
    setContextSize(String(settings?.local_llm_context_size ?? 4096));
  }, [settings?.local_llm_context_size]);

  const handleContextSizeBlur = async () => {
    const parsed = Math.max(
      512,
      Math.min(32768, parseInt(contextSize, 10) || 4096),
    );
    setContextSize(String(parsed));
    await commands.setLocalLlmContextSize(parsed);
    await refreshSettings();
  };

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

  const handlePromptBlur = async () => {
    await commands.changeAssistantSystemPromptSetting(systemPrompt);
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
  const webSearchProvider =
    settings?.assistant_web_search_provider ?? "serper";
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
    <div className="max-w-3xl w-full mx-auto space-y-8">
      <SettingsGroup title={t("settings.assistant.shortcuts.title")}>
        <ShortcutInput shortcutId="assistant" grouped={true} />
        <ShortcutInput shortcutId="assistant_panel_toggle" grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settings.assistant.provider.title")}>
        <SettingContainer
          title={t("settings.assistant.provider.providerLabel")}
          description={t("settings.assistant.provider.providerDescription")}
          descriptionMode="tooltip"
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
            description={t("settings.assistant.provider.baseUrlDescription")}
            descriptionMode="tooltip"
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
            description={t("settings.assistant.provider.apiKeyDescription")}
            descriptionMode="tooltip"
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
            description={t(
              "settings.assistant.provider.builtinModelDescription",
            )}
            descriptionMode="tooltip"
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
            description={t("settings.assistant.provider.modelDescription")}
            descriptionMode="tooltip"
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
            description={t(
              "settings.assistant.provider.contextSizeDescription",
            )}
            descriptionMode="tooltip"
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
      </SettingsGroup>

      <SettingsGroup title={t("settings.assistant.vision.title")}>
        <ToggleSwitch
          checked={settings?.assistant_screenshot_enabled ?? true}
          onChange={(checked) =>
            setAndRefresh(commands.setAssistantScreenshotEnabled(checked))
          }
          label={t("settings.assistant.vision.enableLabel")}
          description={t("settings.assistant.vision.enableDescription")}
          grouped={true}
        />
      </SettingsGroup>

      <SettingsGroup title={t("settings.assistant.webSearch.title")}>
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
            <SettingContainer
              title={t("settings.assistant.webSearch.providerLabel")}
              description={t(
                "settings.assistant.webSearch.providerDescription",
              )}
              descriptionMode="tooltip"
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
                description={t(
                  "settings.assistant.webSearch.apiKeyDescription",
                )}
                descriptionMode="tooltip"
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
              title={t("settings.assistant.webSearch.depthLabel")}
              description={t("settings.assistant.webSearch.depthDescription")}
              descriptionMode="tooltip"
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

            {settings?.assistant_provider_id === "builtin" && (
              <ToggleSwitch
                checked={settings?.assistant_local_search_smart ?? false}
                onChange={(checked) =>
                  setAndRefresh(
                    commands.setAssistantLocalSearchSmart(checked),
                  )
                }
                label={t("settings.assistant.webSearch.localSmartLabel")}
                description={t(
                  "settings.assistant.webSearch.localSmartDescription",
                )}
                grouped={true}
                disabled={!webSearchEnabled}
              />
            )}

            <SettingContainer
              title={t("settings.assistant.webSearch.testLabel")}
              description={t("settings.assistant.webSearch.testDescription")}
              descriptionMode="tooltip"
              layout="horizontal"
              grouped={true}
            >
              <div className="flex flex-col items-end gap-1">
                <button
                  type="button"
                  onClick={handleTestWebSearch}
                  disabled={!webSearchEnabled || webSearchTest === "testing"}
                  className="flex items-center gap-2 px-3 py-1.5 rounded-lg border border-mid-gray/30 hover:bg-mid-gray/10 disabled:opacity-50 disabled:cursor-not-allowed text-sm"
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
                        ? "text-red-500"
                        : "text-muted-soft"
                    }`}
                  >
                    {webSearchTestMsg}
                  </span>
                )}
              </div>
            </SettingContainer>
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
          description={t("settings.assistant.tts.enableDescription")}
          grouped={true}
        />
        {settings?.assistant_tts_enabled && (
          <>
            <SettingContainer
              title={t("settings.assistant.tts.engineLabel")}
              description={t("settings.assistant.tts.engineDescription")}
              descriptionMode="tooltip"
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
              <>
                <SettingContainer
                  title={t("settings.assistant.tts.voiceLabel")}
                  description={t("settings.assistant.tts.voiceDescription")}
                  descriptionMode="tooltip"
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
                <SettingContainer
                  title={t("settings.assistant.tts.dtypeLabel")}
                  description={t("settings.assistant.tts.dtypeDescription")}
                  descriptionMode="tooltip"
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
              </>
            )}

            {settings?.assistant_tts_engine === "openai" && (
              <>
                <SettingContainer
                  title={t("settings.assistant.tts.baseUrlLabel")}
                  description={t("settings.assistant.tts.baseUrlDescription")}
                  descriptionMode="tooltip"
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
                  description={t("settings.assistant.tts.apiKeyDescription")}
                  descriptionMode="tooltip"
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
                    placeholder="gpt-4o-mini-tts"
                    loadLabel={t("settings.assistant.tts.loadModels")}
                    formatCreateLabel={(input) =>
                      t("settings.assistant.tts.modelsUse", { model: input })
                    }
                  />
                </SettingContainer>
                <SettingContainer
                  title={t("settings.assistant.tts.remoteVoiceLabel")}
                  description={t(
                    "settings.assistant.tts.remoteVoiceDescription",
                  )}
                  descriptionMode="tooltip"
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
                  description={t("settings.assistant.tts.apiKeyDescription")}
                  descriptionMode="tooltip"
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
                  description={t(
                    "settings.assistant.tts.elevenVoiceDescription",
                  )}
                  descriptionMode="tooltip"
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
                  description={t(
                    "settings.assistant.tts.azureBaseUrlDescription",
                  )}
                  descriptionMode="tooltip"
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
                  description={t("settings.assistant.tts.apiKeyDescription")}
                  descriptionMode="tooltip"
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
                  description={t(
                    "settings.assistant.tts.azureVoiceDescription",
                  )}
                  descriptionMode="tooltip"
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
              description={t("settings.assistant.tts.speedDescription")}
              descriptionMode="tooltip"
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
                      className={`px-2.5 py-1 text-sm font-medium rounded-full transition-colors disabled:opacity-50 disabled:cursor-not-allowed ${
                        active
                          ? "bg-ink text-on-primary"
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
              description={t("settings.assistant.tts.testDescription")}
              descriptionMode="tooltip"
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
                  className="flex items-center gap-2 px-3 py-1.5 rounded-lg border border-mid-gray/30 hover:bg-mid-gray/10 disabled:opacity-50 disabled:cursor-not-allowed text-sm"
                >
                  <Volume2 size={14} />
                  {testState === "testing"
                    ? t("settings.assistant.tts.testing")
                    : testState === "ok"
                      ? t("settings.assistant.tts.testOk")
                      : t("settings.assistant.tts.testButton")}
                </button>
                {testState === "error" && testError && (
                  <span className="text-xs text-red-500 max-w-[360px] text-right break-words">
                    {testError}
                  </span>
                )}
              </div>
            </SettingContainer>
          </>
        )}
      </SettingsGroup>

      <SettingsGroup title={t("settings.assistant.appearance.title")}>
        <SettingContainer
          title={t("settings.assistant.appearance.previewLabel")}
          description={t("settings.assistant.appearance.previewDescription")}
          descriptionMode="tooltip"
          layout="stacked"
          grouped={true}
        >
          <PanelPreview
            accent={settings?.assistant_accent ?? "violet"}
            fontSize={settings?.assistant_font_size ?? "medium"}
            opacity={settings?.assistant_panel_opacity ?? 1}
            theme={resolvePreviewTheme(settings?.assistant_panel_theme)}
          />
        </SettingContainer>
        <SettingContainer
          title={t("settings.assistant.appearance.themeLabel")}
          description={t("settings.assistant.appearance.themeDescription")}
          descriptionMode="tooltip"
          layout="horizontal"
          grouped={true}
        >
          <Dropdown
            options={[
              {
                value: "auto",
                label: t("settings.assistant.appearance.themes.auto"),
              },
              {
                value: "light",
                label: t("settings.assistant.appearance.themes.light"),
              },
              {
                value: "dark",
                label: t("settings.assistant.appearance.themes.dark"),
              },
            ]}
            selectedValue={settings?.assistant_panel_theme ?? "auto"}
            onSelect={(theme) =>
              setAndRefresh(commands.setAssistantPanelTheme(theme))
            }
          />
        </SettingContainer>
        <SettingContainer
          title={t("settings.assistant.appearance.sizeLabel")}
          description={t("settings.assistant.appearance.sizeDescription")}
          descriptionMode="tooltip"
          layout="horizontal"
          grouped={true}
        >
          <Dropdown
            options={[
              {
                value: "compact",
                label: t("settings.assistant.appearance.sizes.compact"),
              },
              {
                value: "standard",
                label: t("settings.assistant.appearance.sizes.standard"),
              },
              {
                value: "large",
                label: t("settings.assistant.appearance.sizes.large"),
              },
            ]}
            selectedValue={settings?.assistant_panel_size ?? "standard"}
            onSelect={(size) =>
              setAndRefresh(commands.setAssistantPanelSize(size))
            }
          />
        </SettingContainer>
        <SettingContainer
          title={t("settings.assistant.appearance.accentLabel")}
          description={t("settings.assistant.appearance.accentDescription")}
          descriptionMode="tooltip"
          layout="horizontal"
          grouped={true}
        >
          <Dropdown
            options={[
              {
                value: "violet",
                label: t("settings.assistant.appearance.accents.violet"),
              },
              {
                value: "blue",
                label: t("settings.assistant.appearance.accents.blue"),
              },
              {
                value: "emerald",
                label: t("settings.assistant.appearance.accents.emerald"),
              },
              {
                value: "rose",
                label: t("settings.assistant.appearance.accents.rose"),
              },
              {
                value: "amber",
                label: t("settings.assistant.appearance.accents.amber"),
              },
              {
                value: "mono",
                label: t("settings.assistant.appearance.accents.mono"),
              },
            ]}
            selectedValue={settings?.assistant_accent ?? "violet"}
            onSelect={(accent) =>
              setAndRefresh(commands.setAssistantAccent(accent))
            }
          />
        </SettingContainer>
        <SettingContainer
          title={t("settings.assistant.appearance.fontSizeLabel")}
          description={t("settings.assistant.appearance.fontSizeDescription")}
          descriptionMode="tooltip"
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
        <Slider
          value={settings?.assistant_panel_opacity ?? 1}
          onChange={(value) =>
            setAndRefresh(commands.setAssistantPanelOpacity(value))
          }
          min={0.5}
          max={1}
          step={0.05}
          label={t("settings.assistant.appearance.opacityLabel")}
          description={t("settings.assistant.appearance.opacityDescription")}
          grouped={true}
          formatValue={(v) => `${Math.round(v * 100)}%`}
        />
      </SettingsGroup>

      <SettingsGroup title={t("settings.assistant.systemPrompt.title")}>
        <SettingContainer
          title={t("settings.assistant.systemPrompt.label")}
          description={t("settings.assistant.systemPrompt.description")}
          descriptionMode="tooltip"
          layout="stacked"
          grouped={true}
        >
          <Textarea
            value={systemPrompt}
            onChange={(e) => setSystemPrompt(e.target.value)}
            onBlur={handlePromptBlur}
            className="w-full"
            rows={5}
          />
        </SettingContainer>
        <SettingContainer
          title={t("settings.assistant.responseLength.label")}
          description={t("settings.assistant.responseLength.description")}
          descriptionMode="tooltip"
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
          descriptionMode="tooltip"
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
