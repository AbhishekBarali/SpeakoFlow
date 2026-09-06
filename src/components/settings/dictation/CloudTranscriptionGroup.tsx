import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Cloud, ExternalLink, Loader2 } from "lucide-react";
import { toast } from "sonner";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
import { Dropdown } from "@/components/ui/Dropdown";
import { Input } from "@/components/ui/Input";
import { Button } from "@/components/ui/Button";
import { Alert } from "@/components/ui/Alert";
import { ModelCombo } from "@/components/ui/ModelCombo";
import { ProviderModeToggle } from "../PostProcessingSettingsApi/ProviderModeToggle";
import { useSettings } from "@/hooks/useSettings";
import { commands, type CloudSttProvider } from "@/bindings";

/**
 * "Where transcription runs" — the on-device engine, or a hosted one.
 *
 * The switch is first and everything else is behind it, because choosing cloud
 * is the decision that makes the rest of the group meaningful: a provider, a
 * key, and a model are all consequences of it. On the local side this group is a
 * single row, which is the point — the default install should not have to read
 * about an API it is not using.
 *
 * Every field here writes through a dedicated command rather than
 * `updateSetting`. That is not a style choice: `updateSetting` dispatches
 * through an allowlist in the settings store and silently does nothing (a
 * `console.warn`) for a key it has no entry for, so an optimistic-only write
 * would look like it worked and evaporate on the next refresh.
 *
 * Two things are surfaced deliberately rather than buried:
 *
 * - **That the model above is bypassed.** A user who switches to cloud is still
 *   looking at a local model card at the top of the page, and leaving that
 *   contradiction unexplained is how "it ignored my setting" bug reports happen.
 * - **A test button.** A wrong key, an exhausted quota, or a model id this
 *   endpoint does not know all fail identically at dictation time — mid-sentence,
 *   with the transcript gone. One second of audio here turns all three into an
 *   error message with a name on it, before it costs the user a thought.
 */
export const CloudTranscriptionGroup: React.FC = () => {
  const { t } = useTranslation();
  const { settings, getSetting, refreshSettings } = useSettings();

  const [providers, setProviders] = useState<CloudSttProvider[]>([]);
  const [keyStatus, setKeyStatus] = useState<Record<string, boolean>>({});
  const [keyDraft, setKeyDraft] = useState("");
  const [savingKey, setSavingKey] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const [loadingModels, setLoadingModels] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [busy, setBusy] = useState(false);

  const isCloud = getSetting("stt_engine_mode") === "cloud";
  const providerId = getSetting("cloud_stt_provider_id") ?? "elevenlabs";
  const provider = useMemo(
    () => providers.find((entry) => entry.id === providerId),
    [providers, providerId],
  );

  const loadProviders = useCallback(async () => {
    const [list, keys] = await Promise.all([
      commands.getCloudSttProviders(),
      commands.getCloudSttKeyStatus(),
    ]);
    setProviders(list);
    setKeyStatus(Object.fromEntries(keys));
  }, []);

  useEffect(() => {
    void loadProviders();
  }, [loadProviders]);

  // Suggestions come from the registry until the user asks for a live listing.
  // Showing the shipped names immediately means the field is never empty, and an
  // unnecessary authenticated request is never made just to render a picker.
  //
  // Keyed on the provider *id*, not the provider object: `loadProviders()` hands
  // back fresh objects, so depending on identity would throw away a model list
  // the user had just loaded every time a key was saved.
  const registryModels = provider?.models;
  useEffect(() => {
    setModels(registryModels ?? []);
    setModelsError(null);
    setKeyDraft("");
  }, [providerId]);

  const hasKey = keyStatus[providerId] ?? false;
  const selectedModel = settings?.cloud_stt_models?.[providerId] ?? "";
  const baseUrlOverride = settings?.cloud_stt_base_urls?.[providerId] ?? "";

  /** Run a settings write and pull the authoritative state back. */
  const commit = async (write: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await write();
      await refreshSettings();
    } finally {
      setBusy(false);
    }
  };

  const saveKey = async () => {
    const trimmed = keyDraft.trim();
    if (!trimmed) return;
    setSavingKey(true);
    try {
      await commands.setCloudSttApiKey(providerId, trimmed);
      setKeyDraft("");
      await Promise.all([loadProviders(), refreshSettings()]);
      toast.success(t("settings.dictation.cloud.key.saved"));
    } finally {
      setSavingKey(false);
    }
  };

  const clearKey = async () => {
    await commands.setCloudSttApiKey(providerId, "");
    setKeyDraft("");
    await Promise.all([loadProviders(), refreshSettings()]);
  };

  const loadModels = async () => {
    setLoadingModels(true);
    setModelsError(null);
    try {
      const result = await commands.listCloudSttModels();
      if (result.status === "ok") {
        setModels(result.data);
      } else {
        setModelsError(result.error);
      }
    } catch (error) {
      setModelsError(String(error));
    } finally {
      // In a `finally` so an IPC rejection cannot leave the button spinning
      // forever with no way back.
      setLoadingModels(false);
    }
  };

  const runTest = async () => {
    setTesting(true);
    try {
      const result = await commands.testCloudStt();
      if (result.status === "ok") {
        toast.success(result.data);
      } else {
        toast.error(result.error);
      }
    } catch (error) {
      toast.error(String(error));
    } finally {
      setTesting(false);
    }
  };

  const providerOptions = providers.map((entry) => ({
    value: entry.id,
    label: entry.label,
  }));

  const modelOptions = models.map((id) => ({ value: id, label: id }));

  // Streaming is only offered where it exists. ElevenLabs splits its realtime
  // and batch models by name, so "this provider streams" is not enough — the
  // selected model has to be a realtime one, and saying so is more useful than
  // a toggle that silently does nothing.
  const providerStreams = provider?.supports_streaming ?? false;
  const modelStreams =
    provider?.kind === "eleven_labs"
      ? selectedModel.includes("realtime")
      : providerStreams;

  return (
    <SettingsGroup
      title={t("settings.dictation.cloud.groupTitle")}
      icon={Cloud}
    >
      <SettingContainer
        title={t("settings.dictation.cloud.engine.title")}
        description={t("settings.dictation.cloud.engine.description")}
        info={t("settings.dictation.cloud.engine.info")}
        grouped={true}
      >
        <ProviderModeToggle
          mode={isCloud ? "cloud" : "device"}
          disabled={busy}
          onChange={(mode) =>
            void commit(() =>
              commands.setSttEngineMode(mode === "cloud" ? "cloud" : "local"),
            )
          }
        />
      </SettingContainer>

      {isCloud && (
        <>
          <Alert variant="info" contained>
            {t("settings.dictation.cloud.bypassNotice")}
          </Alert>

          <SettingContainer
            title={t("settings.dictation.cloud.provider.title")}
            grouped={true}
          >
            <Dropdown
              options={providerOptions}
              selectedValue={providerId}
              onSelect={(value) =>
                void commit(() => commands.setCloudSttProvider(value))
              }
              disabled={busy}
              className="min-w-[200px]"
            />
          </SettingContainer>

          <SettingContainer
            title={t("settings.dictation.cloud.key.title")}
            description={
              hasKey
                ? t("settings.dictation.cloud.key.stored")
                : t("settings.dictation.cloud.key.missing")
            }
            descriptionMode="inline"
            grouped={true}
            layout="stacked"
          >
            <div className="flex w-full flex-wrap items-center gap-2">
              <Input
                type="password"
                value={keyDraft}
                onChange={(event) => setKeyDraft(event.target.value)}
                placeholder={
                  hasKey
                    ? t("settings.dictation.cloud.key.replacePlaceholder")
                    : t("settings.dictation.cloud.key.placeholder")
                }
                variant="compact"
                className="min-w-[260px] flex-1"
              />
              <Button
                size="sm"
                onClick={() => void saveKey()}
                disabled={!keyDraft.trim() || savingKey}
              >
                {t("settings.dictation.cloud.key.save")}
              </Button>
              {hasKey && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => void clearKey()}
                >
                  {t("settings.dictation.cloud.key.clear")}
                </Button>
              )}
              {provider?.api_key_url && (
                <button
                  type="button"
                  onClick={() => void openUrl(provider.api_key_url as string)}
                  className="inline-flex items-center gap-1 text-xs text-muted underline decoration-dotted underline-offset-2 transition-colors hover:text-accent"
                >
                  {t("settings.dictation.cloud.key.getOne")}
                  <ExternalLink size={11} />
                </button>
              )}
            </div>
          </SettingContainer>

          {provider?.allow_base_url_edit && (
            <SettingContainer
              title={t("settings.dictation.cloud.endpoint.title")}
              description={t("settings.dictation.cloud.endpoint.description")}
              grouped={true}
            >
              {/* Committed on blur, not per keystroke: each write is a full
                  settings save, which also re-syncs every stored key with the OS
                  keychain. One save per edit, not one per character. */}
              <Input
                defaultValue={baseUrlOverride}
                key={`${providerId}-${baseUrlOverride}`}
                onBlur={(event) => {
                  const next = event.target.value.trim();
                  if (next === baseUrlOverride) return;
                  void commit(() =>
                    commands.setCloudSttBaseUrl(providerId, next),
                  );
                }}
                placeholder={provider.base_url}
                variant="compact"
                className="min-w-[280px]"
              />
            </SettingContainer>
          )}

          <SettingContainer
            title={t("settings.dictation.cloud.model.title")}
            info={t("settings.dictation.cloud.model.info")}
            grouped={true}
          >
            <ModelCombo
              value={selectedModel}
              options={modelOptions}
              onCommit={(value) =>
                void commit(() => commands.setCloudSttModel(providerId, value))
              }
              onLoad={() => void loadModels()}
              loading={loadingModels}
              error={modelsError}
              placeholder={provider?.default_model}
              loadLabel={t("settings.dictation.cloud.model.load")}
              disabled={busy}
            />
          </SettingContainer>

          {providerStreams && (
            <ToggleSwitch
              checked={getSetting("cloud_stt_streaming") ?? true}
              onChange={(value) =>
                void commit(() => commands.setCloudSttStreaming(value))
              }
              isUpdating={busy}
              label={t("settings.dictation.cloud.streaming.label")}
              description={
                modelStreams
                  ? t("settings.dictation.cloud.streaming.description")
                  : t("settings.dictation.cloud.streaming.needsRealtimeModel")
              }
              descriptionMode="inline"
              grouped={true}
            />
          )}

          <ToggleSwitch
            checked={getSetting("cloud_stt_send_custom_words") ?? true}
            onChange={(value) =>
              void commit(() => commands.setCloudSttSendCustomWords(value))
            }
            isUpdating={busy}
            label={t("settings.dictation.cloud.keyterms.label")}
            description={t("settings.dictation.cloud.keyterms.description")}
            grouped={true}
          />

          <ToggleSwitch
            checked={getSetting("cloud_stt_no_verbatim") ?? false}
            onChange={(value) =>
              void commit(() => commands.setCloudSttNoVerbatim(value))
            }
            isUpdating={busy}
            label={t("settings.dictation.cloud.noVerbatim.label")}
            description={t("settings.dictation.cloud.noVerbatim.description")}
            grouped={true}
          />

          <SettingContainer
            title={t("settings.dictation.cloud.test.title")}
            description={t("settings.dictation.cloud.test.description")}
            grouped={true}
          >
            <Button
              size="sm"
              variant="secondary"
              onClick={() => void runTest()}
              disabled={testing || (!hasKey && !provider?.allow_base_url_edit)}
            >
              {testing ? (
                <span className="inline-flex items-center gap-1.5">
                  <Loader2 size={13} className="animate-spin" />
                  {t("settings.dictation.cloud.test.running")}
                </span>
              ) : (
                t("settings.dictation.cloud.test.action")
              )}
            </Button>
          </SettingContainer>
        </>
      )}
    </SettingsGroup>
  );
};
