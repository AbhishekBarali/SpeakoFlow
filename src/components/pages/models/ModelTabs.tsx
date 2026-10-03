import React, { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useModelStore } from "@/stores/modelStore";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { getTranslatedModelName } from "@/lib/utils/modelTranslation";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { LogoSelect } from "@/components/ui/LogoSelect";
import { Button } from "@/components/ui/Button";
import { InfoTip } from "@/components/ui/InfoTip";
import { ProviderModeToggle } from "@/components/settings/PostProcessingSettingsApi/ProviderModeToggle";
import { CloudTranscriptionGroup } from "@/components/settings/dictation/CloudTranscriptionGroup";
import { LanguageSelector } from "@/components/settings/LanguageSelector";
import { TranslateToEnglish } from "@/components/settings/TranslateToEnglish";
import { ModelsSettings } from "@/components/settings/models/ModelsSettings";
import { LlmCatalog } from "@/components/settings/assistant/LlmCatalog";
import { PostProcessingSettingsApi } from "@/components/settings/post-processing/PostProcessingSettings";
import { PostProcessTimeout } from "@/components/settings/PostProcessTimeout";
import { PostProcessUnloadTimeout } from "@/components/settings/PostProcessUnloadTimeout";
import { AssistantSettings } from "@/components/settings/assistant/AssistantSettings";
import { ModelMark } from "@/components/shell/SlotVisuals";
import { useNavigation, type ModelSlot } from "@/components/shell/navigation";
import { useModelSlots } from "@/components/shell/useModelSlots";

/**
 * The content of each Models tab.
 *
 * Every tab has the same skeleton, whichever way the job runs: a settings card
 * whose first row is "Runs on" and whose second is "Model", then — for models
 * that run on this computer — the list to download more from. Switching a job
 * between this computer and the cloud changes the rows inside the card, not the
 * shape of the page, which is what used to make the switch feel like landing on
 * a different screen.
 */

const scrollIntoView = (element: HTMLElement | null) =>
  element?.scrollIntoView({ behavior: "smooth", block: "start" });

/** A plain line of context above a tab's first card, or nothing. */
const Note: React.FC<{
  children: React.ReactNode;
  action?: React.ReactNode;
}> = ({ children, action }) => (
  <div className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-2xl border border-hairline bg-surface-muted px-5 py-3">
    <p className="min-w-0 flex-1 text-sm text-body">{children}</p>
    {action}
  </div>
);

/** Heading for the download list on a tab. */
const CatalogHeading: React.FC<{ title: string; info?: string }> = ({
  title,
  info,
}) => (
  <div className="mb-3 flex items-center gap-1.5">
    <h3 className="font-display text-[1.375rem] text-ink">{title}</h3>
    {info && <InfoTip text={info} className="mt-0.5" />}
  </div>
);

/* ─────────────────────────── speech to text ─────────────────────────── */

const DeviceSttRows: React.FC<{ onBrowse: () => void }> = ({ onBrowse }) => {
  const { t } = useTranslation();
  const models = useModelStore((state) => state.models);
  const currentModel = useModelStore((state) => state.currentModel);
  const selectModel = useModelStore((state) => state.selectModel);
  const [busy, setBusy] = useState(false);
  const downloaded = models.filter(
    (model) => getModelCategory(model) === "stt" && model.is_downloaded,
  );
  const current = models.find((model) => model.id === currentModel);

  return (
    <>
      <SettingContainer
        title={t("settings.postProcessing.api.model.title")}
        grouped={true}
      >
        <LogoSelect
          options={downloaded.map((model) => ({
            value: model.id,
            label: getTranslatedModelName(model, t),
            icon: <ModelMark model={model} size="sm" />,
          }))}
          value={current?.is_downloaded ? currentModel : null}
          onChange={(id) => {
            setBusy(true);
            void selectModel(id).finally(() => setBusy(false));
          }}
          placeholder={t("pickers.choose")}
          disabled={busy}
          className="w-[20rem]"
          ariaLabel={t("pickers.label.stt")}
          footerAction={{ label: t("pickers.download"), onClick: onBrowse }}
        />
      </SettingContainer>
      {current?.supports_language_selection && (
        <LanguageSelector
          grouped={true}
          supportedLanguages={current.supported_languages}
        />
      )}
      {current?.supports_translation && <TranslateToEnglish grouped={true} />}
    </>
  );
};

export const SpeechTab: React.FC = () => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();
  const [busy, setBusy] = useState(false);
  const catalogRef = useRef<HTMLElement>(null);
  const isCloud = settings?.stt_engine_mode === "cloud";

  const setMode = async (mode: "device" | "cloud") => {
    setBusy(true);
    try {
      await commands.setSttEngineMode(mode === "cloud" ? "cloud" : "local");
      await refreshSettings();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-9">
      <SettingsGroup>
        <SettingContainer
          title={t("models.runsOn")}
          description={t("modelsHub.slots.stt.whereHint")}
          grouped={true}
        >
          <ProviderModeToggle
            mode={isCloud ? "cloud" : "device"}
            onChange={(mode) => void setMode(mode)}
            disabled={busy}
          />
        </SettingContainer>
        {isCloud ? (
          <CloudTranscriptionGroup embedded section="setup" />
        ) : (
          <DeviceSttRows onBrowse={() => scrollIntoView(catalogRef.current)} />
        )}
      </SettingsGroup>

      {isCloud ? (
        <SettingsGroup title={t("models.options")}>
          <CloudTranscriptionGroup embedded section="options" />
        </SettingsGroup>
      ) : (
        <section ref={catalogRef} className="scroll-mt-6">
          <CatalogHeading
            title={t("models.downloadTitle")}
            info={t("modelsHub.slots.stt.catalogDescription")}
          />
          <ModelsSettings lockedCategory="stt" />
        </section>
      )}
    </div>
  );
};

/* ────────────────────────────── AI cleanup ────────────────────────────── */

export const CleanupTab: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const summary = useModelSlots().cleanup;
  const catalogRef = useRef<HTMLElement>(null);
  const enabled = getSetting("post_process_enabled") ?? false;
  const isDevice = (getSetting("post_process_provider_id") ?? "") === "builtin";

  return (
    <div className="space-y-9">
      {!enabled && (
        <Note
          action={
            <Button
              size="sm"
              onClick={() => void updateSetting("post_process_enabled", true)}
              disabled={isUpdating("post_process_enabled")}
            >
              {t("modelsHub.slots.cleanup.turnOn")}
            </Button>
          }
        >
          {t("models.cleanupOff")}
        </Note>
      )}
      {summary.borrowsAssistant && (
        <Note>
          {t("modelsHub.slots.cleanup.borrowsNotice", {
            model: summary.modelLabel ?? "",
          })}
        </Note>
      )}

      <SettingsGroup>
        <PostProcessingSettingsApi
          onBrowseModels={() => scrollIntoView(catalogRef.current)}
        />
      </SettingsGroup>

      {isDevice && (
        <section ref={catalogRef} className="scroll-mt-6">
          <CatalogHeading title={t("models.downloadTitle")} />
          <LlmCatalog role="cleanup" />
        </section>
      )}

      <SettingsGroup title={t("modelsHub.performance")}>
        <PostProcessTimeout grouped={true} />
        {isDevice && <PostProcessUnloadTimeout grouped={true} />}
      </SettingsGroup>
    </div>
  );
};

/* ────────────────────────────── assistant ────────────────────────────── */

export const AssistantTab: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const { navigate } = useNavigation();
  const catalogRef = useRef<HTMLElement>(null);
  const assistantEnabled = getSetting("assistant_enabled") ?? true;
  const isDevice = (getSetting("assistant_provider_id") ?? "") === "builtin";

  return (
    <div className="space-y-9">
      {!assistantEnabled && (
        <Note
          action={
            <Button
              size="sm"
              variant="secondary"
              onClick={() => navigate("assistant")}
            >
              {t("modelsHub.slots.assistant.openAssistant")}
            </Button>
          }
        >
          {t("models.assistantOff")}
        </Note>
      )}

      <AssistantSettings
        sections={["brain"]}
        onOpenLlmCatalog={() => scrollIntoView(catalogRef.current)}
      />

      {isDevice && (
        <section ref={catalogRef} className="scroll-mt-6">
          <CatalogHeading
            title={t("models.downloadTitle")}
            info={t("modelsHub.slots.assistant.fallbackNote")}
          />
          <LlmCatalog role="assistant" />
        </section>
      )}
    </div>
  );
};

/* ──────────────────────────────── voice ──────────────────────────────── */

export const VoiceTab: React.FC = () => (
  <AssistantSettings sections={["voice"]} />
);

export const MODEL_TABS: Record<ModelSlot, React.ComponentType> = {
  stt: SpeechTab,
  cleanup: CleanupTab,
  assistant: AssistantTab,
  voice: VoiceTab,
};
