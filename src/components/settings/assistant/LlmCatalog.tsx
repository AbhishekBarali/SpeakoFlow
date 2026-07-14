import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Search, ShieldCheck } from "lucide-react";
import { commands, type ModelInfo } from "@/bindings";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { useModelStore } from "@/stores/modelStore";
import { useSettings } from "@/hooks/useSettings";
import { AddCustomModelDialog } from "../models/AddCustomModelDialog";
import ModelCard, { type ModelCardStatus } from "../../onboarding/ModelCard";

/** The built-in (local) llama.cpp provider id, mirrored from the backend. */
const BUILTIN_PROVIDER_ID = "builtin";

/**
 * On-device assistant model browser. It gives the active model a stable home,
 * keeps SpeakoFlow's curated options easy to compare, and treats Hugging Face
 * as a first-class discovery path instead of a secondary utility action.
 */
export const LlmCatalog: React.FC = () => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const {
    models,
    downloadModel,
    deleteModel,
    cancelDownload,
    downloadingModels,
    verifyingModels,
    extractingModels,
    downloadProgress,
    downloadStats,
  } = useModelStore();

  const activeModelId = settings?.assistant_models?.[BUILTIN_PROVIDER_ID] ?? "";
  const providerIsBuiltin =
    settings?.assistant_provider_id === BUILTIN_PROVIDER_ID;

  const llmModels = useMemo(
    () =>
      models
        .filter((model: ModelInfo) => getModelCategory(model) === "llm")
        .sort((a: ModelInfo, b: ModelInfo) => {
          if (a.is_recommended !== b.is_recommended) {
            return a.is_recommended ? -1 : 1;
          }
          if (a.recommended_rank !== b.recommended_rank) {
            return (a.recommended_rank ?? 999) - (b.recommended_rank ?? 999);
          }
          return Number(a.size_mb) - Number(b.size_mb);
        }),
    [models],
  );
  const curatedModels = llmModels.filter((model) => !model.is_custom);
  const huggingFaceModels = llmModels.filter((model) => model.is_custom);
  const activeModel = providerIsBuiltin
    ? llmModels.find(
        (model) => model.id === activeModelId && model.is_downloaded,
      )
    : undefined;
  const recommendedModels = activeModel
    ? curatedModels.filter((model) => model.id !== activeModel.id)
    : curatedModels;
  const addedModels = activeModel?.is_custom
    ? huggingFaceModels.filter((model) => model.id !== activeModel.id)
    : huggingFaceModels;

  const wireUpProvider = async (modelId: string) => {
    await commands.changeAssistantModelSetting(BUILTIN_PROVIDER_ID, modelId);
    if (!providerIsBuiltin) {
      await commands.setAssistantProvider(BUILTIN_PROVIDER_ID);
    }
    await refreshSettings();
  };

  const handleDownload = async (modelId: string) => {
    const model = models.find(
      (candidate: ModelInfo) => candidate.id === modelId,
    );
    if (model?.is_downloaded) {
      await wireUpProvider(modelId);
      return;
    }
    const ok = await downloadModel(modelId);
    if (ok) await wireUpProvider(modelId);
  };

  const handleSelect = (modelId: string) => {
    void wireUpProvider(modelId);
  };

  const handleDelete = async (modelId: string) => {
    await deleteModel(modelId);
    await refreshSettings();
  };

  const statusFor = (model: ModelInfo): ModelCardStatus => {
    if (model.id in extractingModels) return "extracting";
    if (model.id in verifyingModels) return "verifying";
    if (model.id in downloadingModels) return "downloading";
    if (model.is_downloaded) {
      return providerIsBuiltin && model.id === activeModelId
        ? "active"
        : "available";
    }
    return "downloadable";
  };

  const catalogCardClass = (status: ModelCardStatus) =>
    [
      "!h-full !gap-3 !rounded-2xl !border !px-4 !py-4 !ring-0",
      "[&>div:first-child]:!items-start [&>div:nth-child(2)]:!mt-auto [&>div:nth-child(2)]:!h-auto [&>div:nth-child(2)]:!min-h-8 [&>div:nth-child(2)]:!ps-0 [&>div:nth-child(2)]:!flex-wrap [&_.text-body]:line-clamp-2",
      status === "active"
        ? "!border-accent/35 !bg-accent/[0.07]"
        : "!border-hairline !bg-surface hover:!border-hairline-strong hover:!bg-surface-strong/45",
    ].join(" ");

  const renderModel = (model: ModelInfo, showRecommended = true) => {
    const status = statusFor(model);
    return (
      <ModelCard
        key={model.id}
        model={model}
        status={status}
        onSelect={handleSelect}
        onDownload={handleDownload}
        onDelete={handleDelete}
        onCancel={cancelDownload}
        downloadProgress={downloadProgress[model.id]?.percentage}
        downloadSpeed={downloadStats[model.id]?.speed}
        showRecommended={showRecommended}
        showScores={false}
        showPrimaryAction={true}
        className={catalogCardClass(status)}
      />
    );
  };

  return (
    <div className="space-y-8">
      {activeModel && (
        <section aria-labelledby="current-local-model" className="space-y-2.5">
          <div className="flex items-center gap-2 px-1">
            <ShieldCheck className="h-4 w-4 text-accent" aria-hidden="true" />
            <div>
              <h2
                id="current-local-model"
                className="text-[13.5px] font-semibold tracking-tight text-ink"
              >
                {t("settings.assistant.brain.currentModelTitle")}
              </h2>
              <p className="mt-0.5 text-xs text-muted">
                {t("settings.assistant.brain.currentModelDescription")}
              </p>
            </div>
          </div>
          {renderModel(activeModel, false)}
        </section>
      )}

      <section aria-labelledby="recommended-local-models" className="space-y-3">
        <div className="px-1">
          <h2
            id="recommended-local-models"
            className="text-[13.5px] font-semibold tracking-tight text-ink"
          >
            {t("settings.assistant.brain.recommendedTitle")}
          </h2>
          <p className="mt-0.5 max-w-[62ch] text-xs leading-relaxed text-muted">
            {t("settings.assistant.brain.recommendedDescription")}
          </p>
        </div>
        {recommendedModels.length === 0 ? (
          <div className="rounded-2xl border border-dashed border-hairline-strong px-4 py-5 text-center">
            <p className="text-xs text-muted">
              {t("settings.assistant.brain.catalogEmpty")}
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            {recommendedModels.map((model) => renderModel(model))}
          </div>
        )}
      </section>

      <section aria-labelledby="hugging-face-finder" className="space-y-3">
        <div className="px-1">
          <h2
            id="hugging-face-finder"
            className="text-[13.5px] font-semibold tracking-tight text-ink"
          >
            {t("settings.assistant.brain.finderSectionTitle")}
          </h2>
          <p className="mt-0.5 max-w-[62ch] text-xs leading-relaxed text-muted">
            {t("settings.assistant.brain.finderSectionDescription")}
          </p>
        </div>
        <button
          type="button"
          onClick={() => setAddDialogOpen(true)}
          className="group flex w-full items-center gap-4 rounded-2xl border border-hairline-strong bg-surface px-4 py-4 text-start transition-[background-color,border-color,box-shadow,transform] duration-150 hover:border-accent/40 hover:bg-accent/[0.035] hover:shadow-[0_10px_28px_-22px_var(--color-accent)] focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 active:scale-[0.99] cursor-pointer"
        >
          <span className="grid h-11 w-11 shrink-0 place-items-center rounded-xl bg-accent/12 text-accent">
            <Search className="h-5 w-5" aria-hidden="true" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-sm font-semibold text-ink">
              {t("settings.assistant.brain.finderTitle")}
            </span>
            <span className="mt-0.5 block text-xs leading-relaxed text-muted">
              {t("settings.assistant.brain.finderDescription")}
            </span>
          </span>
          <span className="hidden shrink-0 items-center gap-1.5 rounded-lg bg-accent px-3 py-2 text-xs font-semibold text-on-primary transition-colors group-hover:bg-accent-strong sm:flex">
            {t("settings.assistant.brain.finderAction")}
            <ArrowRight
              className="h-3.5 w-3.5 transition-transform group-hover:translate-x-0.5 motion-reduce:transition-none"
              aria-hidden="true"
            />
          </span>
          <ArrowRight
            className="h-4 w-4 shrink-0 text-muted sm:hidden"
            aria-hidden="true"
          />
        </button>
      </section>

      {addedModels.length > 0 && (
        <section aria-labelledby="hugging-face-models" className="space-y-3">
          <div className="px-1">
            <h2
              id="hugging-face-models"
              className="text-[13.5px] font-semibold tracking-tight text-ink"
            >
              {t("settings.assistant.brain.huggingFaceTitle")}
            </h2>
            <p className="mt-0.5 max-w-[62ch] text-xs leading-relaxed text-muted">
              {t("settings.assistant.brain.huggingFaceDescription")}
            </p>
          </div>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            {addedModels.map((model) => renderModel(model, false))}
          </div>
        </section>
      )}

      <AddCustomModelDialog
        open={addDialogOpen}
        onClose={() => setAddDialogOpen(false)}
      />
    </div>
  );
};

export default LlmCatalog;
