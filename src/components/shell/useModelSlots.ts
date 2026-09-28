import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  type AppSettings,
  type CloudSttProvider,
  type ModelInfo,
  type PostProcessReadiness,
} from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useModelStore } from "@/stores/modelStore";
import { getTranslatedModelName } from "@/lib/utils/modelTranslation";
import { ttsEngineSpec, ttsNeedsSetup, ttsValues } from "@/lib/ttsEngines";
import { KITTEN_MODEL_ID } from "@/assistant/localVoice";
import {
  prettyModelName,
  splitLocalModelName,
} from "@/lib/utils/prettyModelName";
import type { ProviderKind } from "@/components/icons/ProviderLogos";
import type { ModelSlot } from "./navigation";
import { useSlotDataStore } from "./slotData";

/**
 * One answer to "what is doing this job right now?", per job.
 *
 * The old layout made the user reconstruct this from four pages: the dictation
 * model card, a cloud switch below it that silently overrode the card, a
 * cleanup model two groups further down, and the assistant's brain on another
 * page — which meeting notes also used, without saying so anywhere. Every
 * summary surface (Home, Models, the AI cleanup and Assistant pages, Meetings)
 * reads this one derivation so they cannot disagree.
 */

export type SlotIssue = "no_model" | "not_downloaded" | "no_key" | null;

export interface SlotSummary {
  slot: ModelSlot;
  /** The feature behind the slot is switched on. */
  active: boolean;
  /** Something is configured that can actually run. */
  ready: boolean;
  where: "device" | "cloud";
  /** Provider or engine id, for its logo. Null for a local catalog model. */
  providerId: string | null;
  providerKind: ProviderKind;
  providerLabel: string | null;
  /** What the user would call the model. */
  modelLabel: string | null;
  /** The exact id or file the model is configured as, for a tooltip. */
  modelId: string | null;
  /** The on-device catalog entry, when there is one (for its own brand). */
  localModel: ModelInfo | null;
  /** Cleanup has no model of its own and uses the assistant's. */
  borrowsAssistant: boolean;
  issue: SlotIssue;
}

/** Providers that authenticate without an API key. */
const KEYLESS_LLM_PROVIDERS = new Set([
  "builtin",
  "local",
  "custom",
  "apple_intelligence",
]);

const findLocal = (models: ModelInfo[], id: string | null | undefined) =>
  id ? (models.find((model) => model.id === id) ?? null) : null;

const llmProviderLabel = (settings: AppSettings | null, id: string) =>
  settings?.post_process_providers?.find((provider) => provider.id === id)
    ?.label ?? id;

/** A catalog model's name without its packaging ("(Vision)", "Q8_0"). */
export const localModelLabel = (model: ModelInfo, t: TFunction): string =>
  splitLocalModelName(getTranslatedModelName(model, t), model.filename).name;

export const summarizeStt = (
  settings: AppSettings | null,
  models: ModelInfo[],
  currentModel: string,
  cloudProviders: CloudSttProvider[],
  cloudKeys: Record<string, boolean>,
  t: TFunction,
): SlotSummary => {
  if (settings?.stt_engine_mode === "cloud") {
    const id = settings.cloud_stt_provider_id ?? "elevenlabs";
    const provider = cloudProviders.find((entry) => entry.id === id);
    const model =
      settings.cloud_stt_models?.[id]?.trim() || provider?.default_model || "";
    const hasKey = cloudKeys[id] ?? false;
    const keyless = provider?.allow_base_url_edit ?? false;
    return {
      slot: "stt",
      active: true,
      ready: !!model && (hasKey || keyless),
      where: "cloud",
      providerId: id,
      providerKind: "stt",
      providerLabel: provider?.label ?? id,
      modelLabel: prettyModelName(model) || null,
      modelId: model || null,
      localModel: null,
      borrowsAssistant: false,
      issue: !hasKey && !keyless ? "no_key" : !model ? "no_model" : null,
    };
  }

  const local = findLocal(models, currentModel);
  return {
    slot: "stt",
    active: true,
    ready: !!local?.is_downloaded,
    where: "device",
    providerId: null,
    providerKind: "stt",
    providerLabel: null,
    modelLabel: local ? localModelLabel(local, t) : null,
    modelId: local?.id ?? (currentModel || null),
    localModel: local,
    borrowsAssistant: false,
    issue: !local ? "no_model" : !local.is_downloaded ? "not_downloaded" : null,
  };
};

const summarizeLlm = (
  slot: "assistant" | "cleanup",
  settings: AppSettings | null,
  models: ModelInfo[],
  providerId: string,
  modelId: string,
  active: boolean,
  t: TFunction,
  borrowsAssistant = false,
): SlotSummary => {
  const isDevice = providerId === "builtin";
  const local = isDevice ? findLocal(models, modelId) : null;
  const hasKey =
    KEYLESS_LLM_PROVIDERS.has(providerId) ||
    !!settings?.post_process_api_keys?.[providerId]?.trim();
  const ready = isDevice ? !!local?.is_downloaded : !!modelId.trim() && hasKey;
  return {
    slot,
    active,
    ready,
    where: isDevice ? "device" : "cloud",
    providerId: isDevice ? "builtin" : providerId,
    providerKind: "llm",
    providerLabel: isDevice ? null : llmProviderLabel(settings, providerId),
    modelLabel: local
      ? localModelLabel(local, t)
      : prettyModelName(modelId) || null,
    modelId: modelId.trim() || null,
    localModel: local,
    borrowsAssistant,
    issue: isDevice
      ? !modelId
        ? "no_model"
        : !local?.is_downloaded
          ? "not_downloaded"
          : null
      : !modelId.trim()
        ? "no_model"
        : !hasKey
          ? "no_key"
          : null,
  };
};

export const summarizeAssistant = (
  settings: AppSettings | null,
  models: ModelInfo[],
  t: TFunction,
): SlotSummary => {
  const providerId = settings?.assistant_provider_id ?? "custom";
  return summarizeLlm(
    "assistant",
    settings,
    models,
    providerId,
    settings?.assistant_models?.[providerId] ?? "",
    settings?.assistant_enabled ?? true,
    t,
  );
};

export const summarizeCleanup = (
  settings: AppSettings | null,
  models: ModelInfo[],
  readiness: PostProcessReadiness | null,
  t: TFunction,
): SlotSummary => {
  const active = settings?.post_process_enabled ?? false;
  // The backend's resolver is the authority on which model will run: cleanup
  // falls back to the assistant's selection when its own is incomplete, so the
  // stored selection alone can name a model that will never be called.
  if (readiness?.state === "ready") {
    return summarizeLlm(
      "cleanup",
      settings,
      models,
      readiness.provider_id,
      readiness.model,
      active,
      t,
      readiness.source === "assistant_fallback",
    );
  }
  const providerId = settings?.post_process_provider_id ?? "builtin";
  const summary = summarizeLlm(
    "cleanup",
    settings,
    models,
    providerId,
    settings?.post_process_models?.[providerId] ?? "",
    active,
    t,
  );
  if (readiness?.state === "unavailable") {
    return {
      ...summary,
      ready: false,
      issue: readiness.reason === "missing_api_key" ? "no_key" : "no_model",
    };
  }
  return summary;
};

export const summarizeVoice = (
  settings: AppSettings | null,
  t: TFunction,
  models: ModelInfo[] = [],
): SlotSummary => {
  const engine = settings?.assistant_tts_engine ?? "kokoro";
  const spec = ttsEngineSpec(engine);
  const isDevice = !!spec?.local;
  const values = ttsValues(settings, engine);
  // Kitten needs nothing configured, only its voice pack on disk.
  const kittenPack =
    engine === "kitten" ? findLocal(models, KITTEN_MODEL_ID) : null;
  const ready =
    engine === "kitten"
      ? !!kittenPack?.is_downloaded
      : !ttsNeedsSetup(settings, engine);
  const model = values.model || null;
  // A custom server on this machine runs on this computer too.
  const onThisComputer =
    isDevice ||
    (!!spec?.url &&
      /^https?:\/\/(localhost|127\.0\.0\.1|\[::1\])(:|\/|$)/i.test(values.url));
  return {
    slot: "voice",
    active: settings?.assistant_tts_enabled ?? false,
    ready,
    where: onThisComputer ? "device" : "cloud",
    providerId: engine,
    providerKind: "tts",
    providerLabel: t(`settings.assistant.tts.engines.${engine}`, {
      defaultValue: engine,
    }),
    modelLabel:
      engine === "kokoro"
        ? t("modelsHub.voice.kokoroModel")
        : engine === "kitten"
          ? t("modelsHub.voice.kittenModel")
          : prettyModelName(model) || null,
    modelId: isDevice ? engine : model,
    localModel: null,
    borrowsAssistant: false,
    // "Add a key" only when a key is what's missing; an address or a voice
    // still reads as "Needs setup" without pointing at the wrong field.
    issue:
      engine === "kitten" && !ready
        ? "not_downloaded"
        : !ready && spec?.key === "required" && !values.key
          ? "no_key"
          : null,
  };
};

/** Live summaries for all four slots. Pure derivation — no fetching here; the
 *  inputs are kept current once, by `useSlotDataSync` in the shell. */
export const useModelSlots = (): Record<ModelSlot, SlotSummary> => {
  const { t } = useTranslation();
  const { settings, postProcessReadiness } = useSettings();
  const models = useModelStore((state) => state.models);
  const currentModel = useModelStore((state) => state.currentModel);
  const cloudProviders = useSlotDataStore((state) => state.cloudProviders);
  const cloudKeys = useSlotDataStore((state) => state.cloudKeys);

  return useMemo(
    () => ({
      stt: summarizeStt(
        settings,
        models,
        currentModel,
        cloudProviders,
        cloudKeys,
        t,
      ),
      cleanup: summarizeCleanup(settings, models, postProcessReadiness, t),
      assistant: summarizeAssistant(settings, models, t),
      voice: summarizeVoice(settings, t, models),
    }),
    [
      settings,
      models,
      currentModel,
      cloudProviders,
      cloudKeys,
      postProcessReadiness,
      t,
    ],
  );
};
