import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Cloud, ExternalLink, Globe, Languages, Loader2 } from "lucide-react";
import { toast } from "sonner";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
import { ProviderGrid } from "@/components/ui/ProviderGrid";
import { Input } from "@/components/ui/Input";
import { Button } from "@/components/ui/Button";
import { Alert } from "@/components/ui/Alert";
import { Dropdown } from "@/components/ui/Dropdown";
import { ModelCombo } from "@/components/ui/ModelCombo";
import { prettyModelName } from "@/lib/utils/prettyModelName";
import { ProviderModeToggle } from "../PostProcessingSettingsApi/ProviderModeToggle";
import { LanguageSelector } from "../LanguageSelector";
import { TranslateToEnglish } from "../TranslateToEnglish";
import { useSettings } from "@/hooks/useSettings";
import {
  commands,
  type CloudSttProvider,
  type CloudSttReadiness,
} from "@/bindings";
import {
  AMAZON_TRANSCRIBE_REGIONS,
  cloudSttFixedModel,
  cloudSttKeyOptional,
  cloudSttNeedsEndpoint,
  cloudSttStreamsOnly,
  composeAwsCredentials,
  isAmazonTranscribe,
} from "@/lib/cloudStt";

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
export const CloudTranscriptionGroup: React.FC<{
  /**
   * Render only the cloud provider rows, without the group card, the engine
   * switch, or the "model above is bypassed" notice. Used by the Models page,
   * which owns the device/cloud choice itself and does not show a local model
   * while cloud is selected — so there is nothing to explain away.
   */
  embedded?: boolean;
  /**
   * Which rows an embedded caller wants: `setup` (provider, key, model,
   * language — the same shape as the on-device card above it) or `options`
   * (streaming, custom words, fillers, the test). Omit for all of them.
   */
  section?: "setup" | "options";
}> = ({ embedded = false, section }) => {
  const showSetup = !section || section === "setup";
  const showOptions = !section || section === "options";
  const { t } = useTranslation();
  const { settings, getSetting, refreshSettings, isUpdating } = useSettings();

  const [providers, setProviders] = useState<CloudSttProvider[]>([]);
  const [keyStatus, setKeyStatus] = useState<Record<string, boolean>>({});
  const [keyDraft, setKeyDraft] = useState("");
  const [savingKey, setSavingKey] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const [loadingModels, setLoadingModels] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [busy, setBusy] = useState(false);
  // Amazon Transcribe takes its key in parts; they are joined into the one
  // stored secret only on Save (`composeAwsCredentials`).
  const [awsKeyId, setAwsKeyId] = useState("");
  const [awsSecret, setAwsSecret] = useState("");
  const [awsToken, setAwsToken] = useState("");
  const [readiness, setReadiness] = useState<CloudSttReadiness | null>(null);

  const isCloud = getSetting("stt_engine_mode") === "cloud";
  const providerId = getSetting("cloud_stt_provider_id") ?? "elevenlabs";
  const provider = useMemo(
    () => providers.find((entry) => entry.id === providerId),
    [providers, providerId],
  );
  const isAmazon = isAmazonTranscribe(provider);

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
    setAwsKeyId("");
    setAwsSecret("");
    setAwsToken("");
  }, [providerId]);

  const hasKey = keyStatus[providerId] ?? false;
  const selectedModel = settings?.cloud_stt_models?.[providerId] ?? "";
  const baseUrlOverride = settings?.cloud_stt_base_urls?.[providerId] ?? "";
  const keyOptional = cloudSttKeyOptional(provider);
  const needsEndpoint = cloudSttNeedsEndpoint(provider, baseUrlOverride);
  // Azure's endpoint is the user's own resource, not a server they host, so it
  // gets its own wording and an example of the URL the portal shows.
  const isAzure = provider?.kind === "azure_speech";
  // Amazon Transcribe's endpoint slot holds a region, shipped as us-east-1.
  const awsRegion = baseUrlOverride.trim() || provider?.base_url || "";
  const awsCredentials = composeAwsCredentials(awsKeyId, awsSecret, awsToken);
  const awsCredentialsInvalid =
    !!awsKeyId.trim() && !!awsSecret.trim() && awsCredentials === null;
  const selectedLanguage = getSetting("selected_language") ?? "auto";
  // The language picker updates the store before the backend has saved, so
  // the question waits for the save rather than asking about the old language.
  const languageSaving = isUpdating("selected_language");

  // Whether Amazon Transcribe can stream the chosen language in this region is
  // the backend's call (`stt_aws::streaming_language`), so it is asked rather
  // than mirrored here. Refreshed whenever an input to that answer changes.
  useEffect(() => {
    if (!isCloud || !isAmazon) {
      setReadiness(null);
      return;
    }
    if (languageSaving) return;
    let cancelled = false;
    commands
      .getCloudSttReadiness()
      .then((result) => {
        if (!cancelled) setReadiness(result);
      })
      .catch(() => {
        if (!cancelled) setReadiness(null);
      });
    return () => {
      cancelled = true;
    };
  }, [
    isCloud,
    isAmazon,
    providerId,
    selectedLanguage,
    languageSaving,
    awsRegion,
    hasKey,
  ]);
  const languageUnsupported =
    readiness?.state === "unavailable" &&
    readiness.reason === "unsupported_language";

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

  const saveAwsCredentials = async () => {
    if (!awsCredentials) return;
    setSavingKey(true);
    try {
      await commands.setCloudSttApiKey(providerId, awsCredentials);
      setAwsKeyId("");
      setAwsSecret("");
      setAwsToken("");
      await Promise.all([loadProviders(), refreshSettings()]);
      toast.success(t("settings.dictation.cloud.key.saved"));
    } finally {
      setSavingKey(false);
    }
  };

  const clearKey = async () => {
    await commands.setCloudSttApiKey(providerId, "");
    setKeyDraft("");
    setAwsKeyId("");
    setAwsSecret("");
    setAwsToken("");
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
    ready:
      (!!keyStatus[entry.id] || cloudSttKeyOptional(entry)) &&
      !cloudSttNeedsEndpoint(entry, settings?.cloud_stt_base_urls?.[entry.id]),
    // A fixed model says nothing under the tile; Amazon Transcribe's region
    // is what tells two setups apart.
    hint: isAmazonTranscribe(entry)
      ? settings?.cloud_stt_base_urls?.[entry.id]?.trim() || entry.base_url
      : prettyModelName(
          settings?.cloud_stt_models?.[entry.id]?.trim() || entry.default_model,
        ) || undefined,
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
  // Amazon Transcribe has no batch endpoint, so streaming is not a choice
  // there: with it off, a finished recording would be sent again at speaking
  // speed. The switch shows as on and locked, with that reason.
  const streamsOnly = cloudSttStreamsOnly(provider);
  const fixedModel = cloudSttFixedModel(provider);
  // Assumed until the provider list has loaded, so the row does not flash the
  // "cannot take a word list" wording on the way in.
  const honorsKeyterms = provider ? !!provider.honors_keyterms : true;
  const regionOptions = AMAZON_TRANSCRIBE_REGIONS.map((region) => ({
    value: region.code,
    label: `${region.name} · ${region.code}`,
  }));

  // Whether "Translate to English" has anywhere to go on this provider. Only the
  // OpenAI schema has a translation route, and its output language is fixed to
  // English. Where there is no route the row is not shown at all: a greyed-out
  // switch with a paragraph on why it cannot work was a row about something
  // the user cannot use.
  const supportsTranslation = provider?.supports_translation ?? false;

  const engineRow = (
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
  );

  const setupRows = isCloud && showSetup && (
    <>
      {!embedded && (
        <Alert variant="info" contained>
          {t("settings.dictation.cloud.bypassNotice")}
        </Alert>
      )}

      <SettingContainer
        title={t("settings.dictation.cloud.provider.title")}
        layout="stacked"
        grouped={true}
      >
        <ProviderGrid
          kind="stt"
          label={t("settings.dictation.cloud.provider.title")}
          options={providerOptions}
          value={providerId}
          onChange={(value) =>
            void commit(() => commands.setCloudSttProvider(value))
          }
          disabled={busy}
        />
      </SettingContainer>

      {isAmazon ? (
        <SettingContainer
          title={t("settings.dictation.cloud.amazon.credentials.title")}
          description={
            hasKey
              ? t("settings.dictation.cloud.key.stored")
              : t("settings.dictation.cloud.amazon.credentials.missing")
          }
          descriptionMode="caption"
          grouped={true}
          layout="stacked"
        >
          <div className="flex w-full flex-col gap-2">
            <div className="flex w-full flex-wrap items-center gap-2">
              <Input
                value={awsKeyId}
                onChange={(event) => setAwsKeyId(event.target.value)}
                placeholder={t(
                  "settings.dictation.cloud.amazon.credentials.keyId",
                )}
                aria-label={t(
                  "settings.dictation.cloud.amazon.credentials.keyId",
                )}
                autoComplete="off"
                spellCheck={false}
                variant="compact"
                className="min-w-[200px] flex-1"
              />
              <Input
                type="password"
                value={awsSecret}
                onChange={(event) => setAwsSecret(event.target.value)}
                placeholder={t(
                  "settings.dictation.cloud.amazon.credentials.secret",
                )}
                aria-label={t(
                  "settings.dictation.cloud.amazon.credentials.secret",
                )}
                variant="compact"
                className="min-w-[200px] flex-1"
              />
            </div>
            <div className="flex w-full flex-wrap items-center gap-2">
              <Input
                type="password"
                value={awsToken}
                onChange={(event) => setAwsToken(event.target.value)}
                placeholder={t(
                  "settings.dictation.cloud.amazon.credentials.sessionToken",
                )}
                aria-label={t(
                  "settings.dictation.cloud.amazon.credentials.sessionToken",
                )}
                variant="compact"
                className="min-w-[200px] flex-1"
              />
              <Button
                size="sm"
                onClick={() => void saveAwsCredentials()}
                disabled={!awsCredentials || savingKey}
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
                  {t("settings.dictation.cloud.amazon.credentials.permission")}
                  <ExternalLink size={11} />
                </button>
              )}
            </div>
            {awsCredentialsInvalid && (
              <span className="text-xs text-red-500">
                {t("settings.dictation.cloud.amazon.credentials.invalid")}
              </span>
            )}
          </div>
        </SettingContainer>
      ) : (
        <SettingContainer
          title={t("settings.dictation.cloud.key.title")}
          description={
            hasKey
              ? t("settings.dictation.cloud.key.stored")
              : t("settings.dictation.cloud.key.missing")
          }
          descriptionMode="caption"
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
              <Button size="sm" variant="ghost" onClick={() => void clearKey()}>
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
      )}

      {isAmazon && (
        <SettingContainer
          title={t("settings.dictation.cloud.amazon.region.title")}
          description={t("settings.dictation.cloud.amazon.region.description")}
          grouped={true}
        >
          <Dropdown
            className="min-w-[300px]"
            options={regionOptions}
            selectedValue={awsRegion}
            onSelect={(value) =>
              void commit(() => commands.setCloudSttBaseUrl(providerId, value))
            }
            disabled={busy}
          />
        </SettingContainer>
      )}

      {provider?.allow_base_url_edit && !isAmazon && (
        <SettingContainer
          title={t("settings.dictation.cloud.endpoint.title")}
          description={
            isAzure
              ? t("settings.dictation.cloud.endpoint.azureDescription")
              : t("settings.dictation.cloud.endpoint.description")
          }
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
              void commit(() => commands.setCloudSttBaseUrl(providerId, next));
            }}
            placeholder={
              isAzure
                ? t("settings.dictation.cloud.endpoint.azurePlaceholder")
                : provider.base_url
            }
            variant="compact"
            className="min-w-[280px]"
          />
        </SettingContainer>
      )}

      {/* A provider with one model has nothing to pick: Amazon Transcribe runs
              the one AWS serves. */}
      {!fixedModel && (
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
      )}

      {/* Language rows live here, not in the local model card above, because
              in cloud mode that model is unloaded and its capabilities describe
              nothing. Every provider wired up here accepts a spoken-language
              hint, so the picker is unconditional and offers the full list rather
              than a local model's subset — a cloud user on an English-only local
              model previously had no language control at all, because the card
              that owned it was hidden along with the model. */}
      <LanguageSelector
        descriptionMode="inline"
        grouped={true}
        icon={Globe}
        tone="emerald"
        description={
          isAmazon
            ? t("settings.dictation.cloud.amazon.language")
            : t("settings.dictation.cloud.language.description")
        }
      />

      {isAmazon && languageUnsupported && (
        <Alert variant="warning" contained>
          {t("settings.dictation.cloud.amazon.unsupportedLanguage", {
            region: awsRegion,
          })}
        </Alert>
      )}

      {supportsTranslation && (
        <TranslateToEnglish
          descriptionMode="inline"
          grouped={true}
          icon={Languages}
          tone="violet"
          description={t("settings.dictation.cloud.translate.supported", {
            provider: provider?.label ?? "",
          })}
        />
      )}
    </>
  );

  const optionRows = isCloud && showOptions && (
    <>
      {providerStreams && (
        <ToggleSwitch
          checked={streamsOnly || (getSetting("cloud_stt_streaming") ?? true)}
          onChange={(value) =>
            void commit(() => commands.setCloudSttStreaming(value))
          }
          disabled={streamsOnly}
          isUpdating={busy}
          label={t("settings.dictation.cloud.streaming.label")}
          description={
            streamsOnly
              ? t("settings.dictation.cloud.amazon.streaming")
              : modelStreams
                ? t("settings.dictation.cloud.streaming.description")
                : t("settings.dictation.cloud.streaming.needsRealtimeModel")
          }
          descriptionMode="inline"
          grouped={true}
        />
      )}

      {/* A provider that cannot take a word list (`honors_keyterms` false) gets
              the app's own correction pass instead, so the switch would claim
              something that does not happen there. */}
      <ToggleSwitch
        checked={
          honorsKeyterms && (getSetting("cloud_stt_send_custom_words") ?? true)
        }
        onChange={(value) =>
          void commit(() => commands.setCloudSttSendCustomWords(value))
        }
        disabled={!honorsKeyterms}
        isUpdating={busy}
        label={t("settings.dictation.cloud.keyterms.label")}
        description={
          honorsKeyterms
            ? t("settings.dictation.cloud.keyterms.description")
            : t("settings.dictation.cloud.keyterms.unsupported", {
                provider: provider?.label ?? "",
              })
        }
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
          disabled={testing || (!hasKey && !keyOptional) || needsEndpoint}
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
  );

  if (embedded) {
    return (
      <>
        {setupRows}
        {optionRows}
      </>
    );
  }

  return (
    <SettingsGroup
      title={t("settings.dictation.cloud.groupTitle")}
      icon={Cloud}
    >
      {engineRow}
      {setupRows}
      {optionRows}
    </SettingsGroup>
  );
};
