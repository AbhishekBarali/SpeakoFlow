import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ArrowUpRight } from "lucide-react";
import {
  commands,
  type AppSettings,
  type ModelInfo,
  type Result,
} from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useModelStore } from "@/stores/modelStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { getModelCategory } from "@/lib/utils/modelCategory";
import {
  prettyModelName,
  splitLocalModelName,
} from "@/lib/utils/prettyModelName";
import { getTranslatedModelName } from "@/lib/utils/modelTranslation";
import { ProviderTile } from "@/components/icons/ProviderLogos";
import { LogoSelect, type LogoSelectOption } from "@/components/ui/LogoSelect";
import { Button } from "@/components/ui/Button";
import { ModelMark } from "./SlotVisuals";
import { useNavigation, type ModelSlot } from "./navigation";
import { useModelSlots } from "./useModelSlots";
import { useSlotDataStore } from "./slotData";
import {
  LLM_REQUIRES_KEY,
  LlmProviderSetup,
  VoiceEngineSetup,
  ttsNeedsSetup,
} from "./ProviderSetup";
import { TTS_ENGINE_IDS } from "@/lib/ttsEngines";

/**
 * "Which model does this job" as one control, usable on any page.
 *
 * Every model on this computer that can do the job, every cloud provider set
 * up for it, and — under "Not set up yet" — every other provider, marked with
 * what it still needs. Choosing one of those opens its setup right here (key,
 * model) instead of sending you to the Models page to find the right form.
 * The footer still leads to Models, for downloads and the fine print.
 *
 * Options are encoded `local:<model id>` / `cloud:<provider id>` /
 * `setup:<provider id>` so one list can hold them all.
 */

const KEYLESS_LLM = new Set([
  "builtin",
  "local",
  "custom",
  "apple_intelligence",
]);
const BUILTIN = "builtin";
const APPLE = "apple_intelligence";

/** A catalog model's label and its quant tag, for a picker option. */
const localLabel = (
  model: ModelInfo,
  t: ReturnType<typeof useTranslation>["t"],
) => splitLocalModelName(getTranslatedModelName(model, t), model.filename);

const hasLlmKey = (settings: AppSettings | null, providerId: string) =>
  KEYLESS_LLM.has(providerId) ||
  !!settings?.post_process_api_keys?.[providerId]?.trim();

/** Commands answer with a Result; a failure must not look like success. */
const ok = async (result: Promise<Result<unknown, string>>) => {
  const settled = await result;
  if (settled.status !== "ok") throw new Error(String(settled.error));
};

const useBrowseAction = (slot: ModelSlot) => {
  const { t } = useTranslation();
  const { openModelSlot } = useNavigation();
  return {
    label: t("pickers.browse"),
    icon: <ArrowUpRight className="h-4 w-4 text-muted" aria-hidden="true" />,
    onClick: () => openModelSlot(slot),
  };
};

/* ─────────────────────────── language models ─────────────────────────── */

export const LlmModelPicker: React.FC<{
  role: "assistant" | "cleanup";
  className?: string;
  /** Limit the list to on-device models or cloud providers. */
  scope?: "all" | "device" | "cloud";
  /** Replaces the default "Browse models" link (which opens Models). */
  onBrowse?: () => void;
}> = ({ role, className = "w-[20rem]", scope = "all", onBrowse }) => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();
  const setPostProcessProvider = useSettingsStore(
    (state) => state.setPostProcessProvider,
  );
  const models = useModelStore((state) => state.models);
  const summary = useModelSlots()[role];
  const defaultBrowse = useBrowseAction(role);
  const browse = onBrowse
    ? { ...defaultBrowse, label: t("pickers.download"), onClick: onBrowse }
    : defaultBrowse;
  const [busy, setBusy] = useState(false);
  const [setupId, setSetupId] = useState<string | null>(null);

  const isCleanup = role === "cleanup";
  const providerId =
    (isCleanup
      ? settings?.post_process_provider_id
      : settings?.assistant_provider_id) ?? "";
  const modelMap =
    (isCleanup ? settings?.post_process_models : settings?.assistant_models) ??
    {};

  const options = useMemo<LogoSelectOption[]>(() => {
    // Headings only help when both kinds are listed.
    const deviceGroup =
      scope === "all" ? t("pickers.groups.device") : undefined;
    const cloudGroup = scope === "all" ? t("pickers.groups.cloud") : undefined;
    const local = models
      .filter(
        (model) =>
          getModelCategory(model) === "llm" &&
          model.is_downloaded &&
          // A cleanup fine-tune cannot hold a conversation.
          (isCleanup || !model.is_cleanup_specialist),
      )
      .sort((a, b) => {
        // Our cleanup model leads the cleanup list; the rest are alphabetical.
        if (isCleanup && a.is_cleanup_specialist !== b.is_cleanup_specialist) {
          return a.is_cleanup_specialist ? -1 : 1;
        }
        return localLabel(a, t).name.localeCompare(localLabel(b, t).name);
      })
      .map<LogoSelectOption>((model) => {
        const { name, quant } = localLabel(model, t);
        return {
          value: `local:${model.id}`,
          label: name,
          hint: quant ?? undefined,
          title: model.local_path ?? model.filename,
          icon: <ModelMark model={model} size="sm" />,
          group: deviceGroup,
        };
      });

    // Apple Intelligence runs on the Mac and needs nothing; the assistant
    // cannot use it at all.
    const providers = (settings?.post_process_providers ?? []).filter(
      (provider) =>
        provider.id !== BUILTIN && (isCleanup || provider.id !== APPLE),
    );
    const isReady = (providerId: string) =>
      providerId === APPLE ||
      (!!modelMap[providerId]?.trim() && hasLlmKey(settings, providerId));

    const cloud = providers
      .filter((provider) => isReady(provider.id))
      .map<LogoSelectOption>((provider) => {
        const model = modelMap[provider.id]?.trim() ?? "";
        return {
          value: `cloud:${provider.id}`,
          label: prettyModelName(model) || provider.label,
          // Under a model name, say who runs it; a provider shown by its own
          // name (Apple Intelligence has no model) needs no second line.
          hint: model ? provider.label : undefined,
          // The trigger shows name + logo; the exact id and provider are one
          // hover away, as on Home.
          title: model ? `${model} · ${provider.label}` : provider.label,
          icon: <ProviderTile id={provider.id} kind="llm" size="sm" />,
          group: cloudGroup,
        };
      });

    // Everything else, with what it still needs. Choosing one sets it up.
    const setup = providers
      .filter((provider) => !isReady(provider.id))
      .map<LogoSelectOption>((provider) => ({
        value: `setup:${provider.id}`,
        label: provider.label,
        hint:
          LLM_REQUIRES_KEY.has(provider.id) &&
          !settings?.post_process_api_keys?.[provider.id]?.trim()
            ? t("pickers.needsKey")
            : t("pickers.needsSetup"),
        icon: <ProviderTile id={provider.id} kind="llm" size="sm" />,
        group: t("pickers.groups.setup"),
      }));

    return [
      ...(scope === "cloud" ? [] : local),
      ...(scope === "device" ? [] : cloud),
      ...(scope === "device" ? [] : setup),
    ];
  }, [models, settings, modelMap, isCleanup, scope, t]);

  const value =
    providerId === BUILTIN
      ? modelMap[BUILTIN]
        ? `local:${modelMap[BUILTIN]}`
        : null
      : providerId
        ? `cloud:${providerId}`
        : null;

  const choose = async (next: string) => {
    const split = next.indexOf(":");
    const kind = next.slice(0, split);
    const id = next.slice(split + 1);
    if (kind === "setup") {
      setSetupId(id);
      return;
    }
    setBusy(true);
    try {
      if (isCleanup) {
        if (kind === "local") {
          await ok(commands.setCleanupLocalModel(id));
          await refreshSettings();
        } else {
          const switched = await setPostProcessProvider(id);
          if (!switched) throw new Error(`could not select ${id}`);
        }
      } else if (kind === "local") {
        await ok(commands.changeAssistantModelSetting(BUILTIN, id));
        if (providerId !== BUILTIN) {
          await ok(commands.setAssistantProvider(BUILTIN));
        }
        await refreshSettings();
      } else {
        await ok(commands.setAssistantProvider(id));
        await refreshSettings();
      }
    } catch (error) {
      console.error(`Failed to switch the ${role} model:`, error);
      toast.error(t("pickers.switchFailed"));
      await refreshSettings();
    } finally {
      setBusy(false);
    }
  };

  // Cleanup with no model of its own quietly uses the assistant's; say so in
  // the closed control instead of showing an empty "Select…".
  const placeholder =
    isCleanup && summary.borrowsAssistant && summary.modelLabel
      ? t("pickers.borrowsAssistant", { model: summary.modelLabel })
      : t("pickers.choose");

  return (
    <>
      <LogoSelect
        options={options}
        value={value}
        onChange={(next) => void choose(next)}
        placeholder={placeholder}
        disabled={busy}
        className={className}
        ariaLabel={t(`pickers.label.${role}`)}
        footerAction={browse}
      />
      <LlmProviderSetup
        role={role}
        providerId={setupId}
        onClose={() => setSetupId(null)}
      />
    </>
  );
};

/* ─────────────────────────── speech to text ─────────────────────────── */

export const SttModelPicker: React.FC<{ className?: string }> = ({
  className = "w-[20rem]",
}) => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();
  const models = useModelStore((state) => state.models);
  const currentModel = useModelStore((state) => state.currentModel);
  const selectModel = useModelStore((state) => state.selectModel);
  const cloudProviders = useSlotDataStore((state) => state.cloudProviders);
  const cloudKeys = useSlotDataStore((state) => state.cloudKeys);
  const refreshCloud = useSlotDataStore((state) => state.refreshCloud);
  const browse = useBrowseAction("stt");
  const [busy, setBusy] = useState(false);

  const isCloud = settings?.stt_engine_mode === "cloud";

  const options = useMemo<LogoSelectOption[]>(() => {
    const local = models
      .filter(
        (model) => getModelCategory(model) === "stt" && model.is_downloaded,
      )
      .map<LogoSelectOption>((model) => ({
        value: `local:${model.id}`,
        label: localLabel(model, t).name,
        title: model.local_path ?? model.filename,
        icon: <ModelMark model={model} size="sm" />,
        group: t("pickers.groups.device"),
      }));
    const cloud = cloudProviders
      .filter(
        (provider) => cloudKeys[provider.id] || provider.allow_base_url_edit,
      )
      .map<LogoSelectOption>((provider) => {
        const model =
          settings?.cloud_stt_models?.[provider.id]?.trim() ||
          provider.default_model;
        return {
          value: `cloud:${provider.id}`,
          label: prettyModelName(model) || provider.label,
          hint: provider.label,
          // The trigger shows name + logo; the exact id and provider are one
          // hover away, as on Home.
          title: `${model} · ${provider.label}`,
          icon: <ProviderTile id={provider.id} kind="stt" size="sm" />,
          group: t("pickers.groups.cloud"),
        };
      });
    return [...local, ...cloud];
  }, [models, cloudProviders, cloudKeys, settings?.cloud_stt_models, t]);

  const value = isCloud
    ? `cloud:${settings?.cloud_stt_provider_id ?? ""}`
    : currentModel
      ? `local:${currentModel}`
      : null;

  const choose = async (next: string) => {
    const split = next.indexOf(":");
    const kind = next.slice(0, split);
    const id = next.slice(split + 1);
    setBusy(true);
    try {
      if (kind === "local") {
        if (isCloud) await commands.setSttEngineMode("local");
        await selectModel(id);
      } else {
        await commands.setCloudSttProvider(id);
        if (!isCloud) await commands.setSttEngineMode("cloud");
      }
      await refreshSettings();
      void refreshCloud();
    } catch (error) {
      console.error("Failed to switch the speech model:", error);
      toast.error(t("pickers.switchFailed"));
      await refreshSettings();
    } finally {
      setBusy(false);
    }
  };

  return (
    <LogoSelect
      options={options}
      value={value}
      onChange={(next) => void choose(next)}
      placeholder={t("pickers.choose")}
      disabled={busy}
      className={className}
      ariaLabel={t("pickers.label.stt")}
      footerAction={browse}
    />
  );
};

/* ──────────────────────────────── voice ──────────────────────────────── */

/**
 * Which voice engine reads replies aloud. An engine that still needs a key or
 * a voice is marked, and choosing it opens its setup in place; cancelling
 * puts back the engine that was in use. When the engine in use is the one
 * missing something, a "Finish setup" button sits beside the picker.
 */
export const VoicePicker: React.FC<{ className?: string }> = ({
  className = "w-[20rem]",
}) => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();
  const browse = useBrowseAction("voice");
  const [busy, setBusy] = useState(false);
  const [setup, setSetup] = useState<{
    engine: string;
    previous: string;
  } | null>(null);
  const current = settings?.assistant_tts_engine ?? "kokoro";

  const options = TTS_ENGINE_IDS.map<LogoSelectOption>((engine) => ({
    value: engine,
    label: t(`settings.assistant.tts.engines.${engine}`),
    hint:
      engine === "kokoro"
        ? t("modelsHub.where.device")
        : ttsNeedsSetup(settings, engine)
          ? t("pickers.needsSetup")
          : undefined,
    icon: <ProviderTile id={engine} kind="tts" size="sm" />,
  }));

  const choose = async (engine: string) => {
    if (ttsNeedsSetup(settings, engine)) {
      setSetup({ engine, previous: current });
      return;
    }
    setBusy(true);
    try {
      await ok(commands.setAssistantTtsEngine(engine));
      await refreshSettings();
    } catch (error) {
      console.error("Failed to switch the voice engine:", error);
      toast.error(t("pickers.switchFailed"));
    } finally {
      setBusy(false);
    }
  };

  // Setting up switches to the engine early (its voice list comes from the
  // engine in use), so backing out has to switch back.
  const cancelSetup = async () => {
    const opened = setup;
    setSetup(null);
    if (!opened || current === opened.previous) return;
    try {
      await ok(commands.setAssistantTtsEngine(opened.previous));
    } catch (error) {
      console.error("Failed to restore the voice engine:", error);
    }
    await refreshSettings();
  };

  return (
    <>
      <LogoSelect
        options={options}
        value={current}
        onChange={(engine) => void choose(engine)}
        disabled={busy}
        className={className}
        ariaLabel={t("pickers.label.voice")}
        footerAction={browse}
      />
      {ttsNeedsSetup(settings, current) && (
        <Button
          variant="secondary"
          className="h-10"
          onClick={() => setSetup({ engine: current, previous: current })}
        >
          {t("pickers.finishSetup")}
        </Button>
      )}
      <VoiceEngineSetup
        engine={setup?.engine ?? null}
        onSaved={() => setSetup(null)}
        onCancel={() => void cancelSetup()}
      />
    </>
  );
};
