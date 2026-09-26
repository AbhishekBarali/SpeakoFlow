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
import { ModelMark } from "./SlotVisuals";
import { useNavigation, type ModelSlot } from "./navigation";
import { useModelSlots } from "./useModelSlots";
import { useSlotDataStore } from "./slotData";

/**
 * "Which model does this job" as one control, usable on any page.
 *
 * The old pages showed the model as a read-only card with a Change link that
 * sent you to a different page to pick one — and the Back button from there
 * went somewhere else again. Every job now has a real picker in place: every
 * model on this computer that can do the job, every cloud provider already set
 * up for it, and a link to the Models page for anything else (downloading,
 * API keys, fine print).
 *
 * Options are encoded `local:<model id>` / `cloud:<provider id>` so one list can
 * hold both.
 */

const KEYLESS_LLM = new Set([
  "builtin",
  "local",
  "custom",
  "apple_intelligence",
]);
const BUILTIN = "builtin";

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

    const cloud = (settings?.post_process_providers ?? [])
      .filter((provider) => provider.id !== BUILTIN)
      .filter((provider) => {
        const model = modelMap[provider.id]?.trim();
        return !!model && hasLlmKey(settings, provider.id);
      })
      .map<LogoSelectOption>((provider) => {
        const model = modelMap[provider.id]?.trim() ?? "";
        return {
          value: `cloud:${provider.id}`,
          label: prettyModelName(model) || provider.label,
          hint: provider.label,
          // The trigger shows name + logo; the exact id and provider are one
          // hover away, as on Home.
          title: `${model} · ${provider.label}`,
          icon: <ProviderTile id={provider.id} kind="llm" size="sm" />,
          group: cloudGroup,
        };
      });

    return [
      ...(scope === "cloud" ? [] : local),
      ...(scope === "device" ? [] : cloud),
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

const TTS_ENGINES = ["kokoro", "openai", "openrouter", "elevenlabs", "azure"];

export const VoicePicker: React.FC<{ className?: string }> = ({
  className = "w-[20rem]",
}) => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();
  const browse = useBrowseAction("voice");
  const [busy, setBusy] = useState(false);

  const options = TTS_ENGINES.map<LogoSelectOption>((engine) => {
    const needsKey =
      engine !== "kokoro" &&
      !settings?.assistant_tts_api_keys?.[engine]?.trim();
    return {
      value: engine,
      label: t(`settings.assistant.tts.engines.${engine}`),
      hint:
        engine === "kokoro"
          ? t("modelsHub.where.device")
          : needsKey
            ? t("pickers.needsKey")
            : undefined,
      icon: <ProviderTile id={engine} kind="tts" size="sm" />,
    };
  });

  const choose = async (engine: string) => {
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

  return (
    <LogoSelect
      options={options}
      value={settings?.assistant_tts_engine ?? "kokoro"}
      onChange={(engine) => void choose(engine)}
      disabled={busy}
      className={className}
      ariaLabel={t("pickers.label.voice")}
      footerAction={browse}
    />
  );
};
