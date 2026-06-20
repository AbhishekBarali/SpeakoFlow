import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw, Volume2, ArrowUp } from "lucide-react";
import { commands, type AzureVoice, type LocalLlmStatus } from "@/bindings";
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

/** Live preview of the assistant panel using the current appearance
 *  settings — mirrors the bubble/input styling of the real panel. */
const PanelPreview: React.FC<{
  accent: string;
  fontSize: string;
  opacity: number;
}> = ({ accent, fontSize, opacity }) => {
  const { t } = useTranslation();
  const [from, to] = ACCENTS[accent] ?? ACCENTS.violet;
  const fs = FONT_SIZES[fontSize] ?? FONT_SIZES.medium;
  const accentGradient = `linear-gradient(135deg, ${from}, ${to})`;

  return (
    <div
      className="rounded-2xl border border-hairline bg-surface p-3 flex flex-col gap-2"
      style={{ opacity: Math.max(opacity, 0.5) }}
    >
      <div className="flex items-center gap-2 pb-1">
        <span
          className="w-1.5 h-1.5 rounded-full shrink-0"
          style={{ background: accentGradient }}
        />
        <span className="font-display text-[15px] leading-none text-ink">
          {t("assistant.title")}
        </span>
      </div>
      <div
        className="self-end max-w-[75%] rounded-2xl rounded-br-md px-3 py-1.5 bg-background-ui text-on-primary"
        style={{ fontSize: fs }}
      >
        {t("settings.assistant.appearance.previewUser")}
      </div>
      <div
        className="self-start max-w-[75%] rounded-2xl rounded-bl-md px-3 py-1.5 bg-surface-strong border border-hairline text-ink"
        style={{ fontSize: fs }}
      >
        {t("settings.assistant.appearance.previewAssistant")}
      </div>
      <div className="flex items-center gap-2 mt-1">
        <div
          className="flex-1 h-9 rounded-xl border border-hairline-strong bg-surface-strong px-3 flex items-center text-muted-soft"
          style={{ fontSize: fs }}
        >
          {t("assistant.inputPlaceholder")}
        </div>
        <div
          className="w-9 h-9 rounded-full flex items-center justify-center bg-background-ui text-on-primary shrink-0"
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

  // Only Kokoro (local) TTS is offered for now. Migrate any previously-selected
  // cloud engine to Kokoro so the UI and the backend stay in sync (otherwise a
  // stale "openai"/"azure" setting would still drive spoken summaries).
  useEffect(() => {
    if (settings && (settings.assistant_tts_engine ?? "kokoro") !== "kokoro") {
      void commands
        .setAssistantTtsEngine("kokoro")
        .then(() => refreshSettings());
    }
  }, [settings, refreshSettings]);

  const [model, setModel] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("");
  const [historyLimit, setHistoryLimit] = useState("12");
  const [ttsPrompt, setTtsPrompt] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [ttsBaseUrl, setTtsBaseUrl] = useState("");
  const [ttsApiKey, setTtsApiKey] = useState("");
  const [ttsModel, setTtsModel] = useState("");
  const [ttsRemoteVoice, setTtsRemoteVoice] = useState("");

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
  // Lazy (not preloaded) Kokoro instance used only by the Test button in this
  // settings window; force-speaks regardless of the enabled toggle.
  const kokoroTest = useKokoroTts(false, ttsVoice, ttsDtype);

  const handleTestTts = async () => {
    setTestState("testing");
    setTestError(null);
    const phrase = t("settings.assistant.tts.testPhrase");
    try {
      if (ttsEngine === "kokoro") {
        await kokoroTest.speak(phrase, true);
      } else {
        const res = await commands.assistantTestTts();
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

  useEffect(() => {
    setTtsPrompt(settings?.assistant_tts_prompt ?? "");
    setTtsBaseUrl(settings?.assistant_tts_base_url ?? "");
    setTtsApiKey(settings?.assistant_tts_api_key ?? "");
    setTtsModel(settings?.assistant_tts_model ?? "");
    setTtsRemoteVoice(settings?.assistant_tts_remote_voice ?? "");
  }, [
    settings?.assistant_tts_prompt,
    settings?.assistant_tts_base_url,
    settings?.assistant_tts_api_key,
    settings?.assistant_tts_model,
    settings?.assistant_tts_remote_voice,
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

  const handleTtsPromptBlur = async () => {
    await commands.changeAssistantTtsPromptSetting(ttsPrompt);
    await refreshSettings();
  };

  const setAndRefresh = async (promise: Promise<unknown>) => {
    await promise;
    await refreshSettings();
  };

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
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
            ]}
            selectedValue="kokoro"
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
                selectedValue={settings?.assistant_tts_kokoro_dtype ?? "fp32"}
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
              description={t("settings.assistant.tts.remoteVoiceDescription")}
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
              description={t("settings.assistant.tts.elevenVoiceDescription")}
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
              description={t("settings.assistant.tts.azureBaseUrlDescription")}
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
              description={t("settings.assistant.tts.azureVoiceDescription")}
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

        <SettingContainer
          title={t("settings.assistant.tts.promptLabel")}
          description={t("settings.assistant.tts.promptDescription")}
          descriptionMode="tooltip"
          layout="stacked"
          grouped={true}
        >
          <Textarea
            value={ttsPrompt}
            onChange={(e) => setTtsPrompt(e.target.value)}
            onBlur={handleTtsPromptBlur}
            className="w-full"
            rows={3}
            disabled={!settings?.assistant_tts_enabled}
          />
        </SettingContainer>
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
