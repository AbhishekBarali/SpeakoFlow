import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw, Volume2, ArrowUp, Globe } from "lucide-react";
import {
  commands,
  type AzureVoice,
  type LocalLlmStatus,
  type AssistantResponseLength,
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
          className="font-display text-[15px] leading-none"
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
  const [webSearchMaxResults, setWebSearchMaxResults] = useState("4");
  const [webSearchTest, setWebSearchTest] = useState<
    "idle" | "testing" | "ok" | "error"
  >("idle");
  const [webSearchTestMsg, setWebSearchTestMsg] = useState<string | null>(null);

  // TTS test button state (shared across engines).
  const [testState, setTestState] = useState<
    "idle" | "testing" | "ok" | "error"
  >("idle");
  const [testError, setTestError] = useState<string | null>(null);

  // Azure voice list state (loaded on demand from the endpoint + key).
  const [azureVoices, setAzureVoices] = useState<AzureVoice[]>([]);
  const [voicesLoading, setVoicesLoading] = useState(false);
  const [voicesError, setVoicesError] = useState<string | null>(null);

  const loadAzureVoices = async () => {
    setVoicesLoading(true);
    setVoicesError(null);
    try {
      const res = await commands.assistantListAzureVoices();
      if (res.status === "error") {
        setVoicesError(res.error);
        setAzureVoices([]);
        return;
      }
      setAzureVoices(res.data);
    } catch (e) {
      setVoicesError(String(e));
      setAzureVoices([]);
    } finally {
      setVoicesLoading(false);
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

  const providerOptions = providers
    .filter((p) => p.id !== "apple_intelligence")
    .map((p) => ({ value: p.id, label: p.label }));

  const handleProviderSelect = async (providerId: string) => {
    await commands.setAssistantProvider(providerId);
    await refreshSettings();
  };

  const handleModelBlur = async () => {
    await commands.changeAssistantModelSetting(selectedProviderId, model);
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
    settings?.assistant_web_search_provider ?? "duckduckgo";
  const webSearchEnabled = settings?.assistant_web_search_enabled ?? false;
  const webSearchNeedsKey =
    webSearchProvider === "firecrawl" || webSearchProvider === "brave";

  // Sync the API-key field to the selected provider's stored key.
  useEffect(() => {
    setWebSearchApiKey(
      settings?.web_search_api_keys?.[webSearchProvider] ?? "",
    );
  }, [settings, webSearchProvider]);

  useEffect(() => {
    setWebSearchMaxResults(
      String(settings?.assistant_web_search_max_results ?? 5),
    );
  }, [settings?.assistant_web_search_max_results]);

  const handleWebSearchApiKeyBlur = async () => {
    await commands.setAssistantWebSearchApiKey(
      webSearchProvider,
      webSearchApiKey,
    );
    await refreshSettings();
  };

  const handleWebSearchMaxResultsBlur = async () => {
    const parsed = Math.max(
      1,
      Math.min(10, parseInt(webSearchMaxResults, 10) || 5),
    );
    setWebSearchMaxResults(String(parsed));
    await commands.setAssistantWebSearchMaxResults(parsed);
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
        <ShortcutInput shortcutId="assistant_vision" grouped={true} />
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
            <Input
              type="text"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              onBlur={handleModelBlur}
              placeholder={t("settings.assistant.provider.modelPlaceholder")}
              className="min-w-[320px]"
            />
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
                    value: "duckduckgo",
                    label: t(
                      "settings.assistant.webSearch.providers.duckduckgo",
                    ),
                  },
                  {
                    value: "firecrawl",
                    label: t(
                      "settings.assistant.webSearch.providers.firecrawl",
                    ),
                  },
                  {
                    value: "brave",
                    label: t("settings.assistant.webSearch.providers.brave"),
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

            <ToggleSwitch
              checked={settings?.assistant_web_search_fetch_content ?? true}
              onChange={(checked) =>
                setAndRefresh(
                  commands.setAssistantWebSearchFetchContent(checked),
                )
              }
              label={t("settings.assistant.webSearch.fetchContentLabel")}
              description={t(
                "settings.assistant.webSearch.fetchContentDescription",
              )}
              grouped={true}
              disabled={!webSearchEnabled}
            />

            <SettingContainer
              title={t("settings.assistant.webSearch.maxResultsLabel")}
              description={t(
                "settings.assistant.webSearch.maxResultsDescription",
              )}
              descriptionMode="tooltip"
              layout="horizontal"
              grouped={true}
            >
              <Input
                type="number"
                min={1}
                max={10}
                value={webSearchMaxResults}
                onChange={(e) => setWebSearchMaxResults(e.target.value)}
                onBlur={handleWebSearchMaxResultsBlur}
                className="w-[120px]"
                disabled={!webSearchEnabled}
              />
            </SettingContainer>

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
                  <Input
                    type="text"
                    value={ttsModel}
                    onChange={(e) => setTtsModel(e.target.value)}
                    onBlur={() =>
                      setAndRefresh(commands.setAssistantTtsModel(ttsModel))
                    }
                    placeholder="gpt-4o-mini-tts"
                    className="min-w-[240px]"
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
                  <Input
                    type="text"
                    value={ttsRemoteVoice}
                    onChange={(e) => setTtsRemoteVoice(e.target.value)}
                    onBlur={() =>
                      setAndRefresh(
                        commands.setAssistantTtsRemoteVoice(ttsRemoteVoice),
                      )
                    }
                    placeholder="alloy"
                    className="min-w-[240px]"
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
                  <Input
                    type="text"
                    value={ttsRemoteVoice}
                    onChange={(e) => setTtsRemoteVoice(e.target.value)}
                    onBlur={() =>
                      setAndRefresh(
                        commands.setAssistantTtsRemoteVoice(ttsRemoteVoice),
                      )
                    }
                    placeholder="JBFqnCBsd6RMkjVDRZzb"
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
                  <Input
                    type="text"
                    value={ttsModel}
                    onChange={(e) => setTtsModel(e.target.value)}
                    onBlur={() =>
                      setAndRefresh(commands.setAssistantTtsModel(ttsModel))
                    }
                    placeholder="eleven_flash_v2_5"
                    className="min-w-[240px]"
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
                  <div className="flex flex-col items-end gap-2">
                    <div className="flex items-center gap-2">
                      <Input
                        type="text"
                        value={ttsRemoteVoice}
                        onChange={(e) => setTtsRemoteVoice(e.target.value)}
                        onBlur={() =>
                          setAndRefresh(
                            commands.setAssistantTtsRemoteVoice(ttsRemoteVoice),
                          )
                        }
                        placeholder="en-US-JennyNeural"
                        className="min-w-[220px]"
                      />
                      <button
                        type="button"
                        onClick={loadAzureVoices}
                        disabled={voicesLoading}
                        className="flex items-center gap-2 px-3 py-1.5 rounded-lg border border-mid-gray/30 hover:bg-mid-gray/10 disabled:opacity-50 disabled:cursor-not-allowed text-sm whitespace-nowrap"
                      >
                        <RefreshCw
                          size={14}
                          className={voicesLoading ? "animate-spin" : ""}
                        />
                        {voicesLoading
                          ? t("settings.assistant.tts.voicesLoading")
                          : t("settings.assistant.tts.loadVoices")}
                      </button>
                    </div>
                    {azureVoices.length > 0 && (
                      <Dropdown
                        options={azureVoices.map((v) => ({
                          value: v.short_name,
                          label: `${v.short_name} · ${v.locale} ${v.gender}`,
                        }))}
                        selectedValue={ttsRemoteVoice}
                        onSelect={(v) => {
                          setTtsRemoteVoice(v);
                          setAndRefresh(commands.setAssistantTtsRemoteVoice(v));
                        }}
                        placeholder={t("settings.assistant.tts.voicesPick", {
                          count: azureVoices.length,
                        })}
                        className="min-w-[300px]"
                      />
                    )}
                    {voicesError && (
                      <span className="text-xs text-red-500 max-w-[360px] text-right break-words">
                        {voicesError}
                      </span>
                    )}
                  </div>
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
