import React, { useEffect, useMemo, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  ArrowRight,
  Check,
  ChevronDown,
  Download,
  Eye,
  HardDrive,
  Info,
  MemoryStick,
  MessageSquareText,
  Search,
  Trash2,
} from "lucide-react";
import { commands, type ModelInfo } from "@/bindings";
import { formatModelSize } from "@/lib/utils/format";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { extractQuant } from "@/lib/utils/modelQuant";
import {
  getTranslatedModelDescription,
  getTranslatedModelName,
} from "@/lib/utils/modelTranslation";
import { useModelStore } from "@/stores/modelStore";
import { useSettings } from "@/hooks/useSettings";
import { getModelBrand } from "../../icons/BrandLogos";
import Badge from "../../ui/Badge";
import { Button } from "../../ui/Button";
import { AddCustomModelDialog } from "../models/AddCustomModelDialog";
import type { ModelCardStatus } from "../../onboarding/ModelCard";

/** The built-in (local) llama.cpp provider id, mirrored from the backend. */
const BUILTIN_PROVIDER_ID = "builtin";

/**
 * A deliberately small, conversation-first set. The recommendation is an
 * editorial quality/latency choice, never a hardware score.
 */
const RECOMMENDED_LOCAL_MODELS = [
  { id: "gemma-4-e2b", supportsVision: true, isRecommended: false },
  { id: "gemma-4-e4b", supportsVision: true, isRecommended: true },
  { id: "gemma-4-12b", supportsVision: true, isRecommended: false },
] as const;

type RecommendedModelId = (typeof RECOMMENDED_LOCAL_MODELS)[number]["id"];
type RecommendedModelMeta = (typeof RECOMMENDED_LOCAL_MODELS)[number];

const RECOMMENDED_MODEL_IDS = new Set<RecommendedModelId>(
  RECOMMENDED_LOCAL_MODELS.map((model) => model.id),
);

interface CatalogModelRowProps {
  model: ModelInfo;
  status: ModelCardStatus;
  meta?: RecommendedModelMeta;
  isRecommended?: boolean;
  protectedFromDelete?: boolean;
  downloadProgress?: number;
  downloadSpeed?: number;
  onSelect: (modelId: string) => void;
  onDownload: (modelId: string) => void;
  onDelete: (modelId: string) => void;
  onCancel: (modelId: string) => void;
}

/** A compact model row with only decision-making information visible. */
const CatalogModelRow: React.FC<CatalogModelRowProps> = ({
  model,
  status,
  meta,
  isRecommended = false,
  protectedFromDelete = false,
  downloadProgress,
  downloadSpeed,
  onSelect,
  onDownload,
  onDelete,
  onCancel,
}) => {
  const { t } = useTranslation();
  const [detailsOpen, setDetailsOpen] = useState(false);
  const brand = getModelBrand(model);
  const displayName = getTranslatedModelName(model, t).replace(
    /\s*\(vision\)\s*$/i,
    "",
  );
  const description = getTranslatedModelDescription(model, t);
  const quant = extractQuant(model.filename);
  const detailsId = `local-model-details-${model.id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  const isBusy =
    status === "downloading" ||
    status === "verifying" ||
    status === "extracting";
  const deleteEligible = model.is_custom || model.is_downloaded;
  const canDelete = deleteEligible && !isBusy && !protectedFromDelete;
  const deleteBlocked = deleteEligible && !isBusy && protectedFromDelete;
  const progress = Math.max(0, Math.min(100, downloadProgress ?? 0));

  return (
    <article
      className={[
        "transition-colors duration-150",
        status === "active" ? "bg-accent/[0.055]" : "bg-surface",
      ].join(" ")}
    >
      <div className="flex flex-col gap-3 p-4 sm:flex-row sm:items-center">
        <div className="flex min-w-0 flex-1 items-start gap-3.5">
          <span
            className={`grid h-10 w-10 shrink-0 place-items-center rounded-xl ${brand.tileClass}`}
          >
            {brand.icon}
          </span>

          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="text-sm font-semibold tracking-tight text-ink">
                {displayName}
              </h3>
              {status === "active" && (
                <Badge variant="active" className="gap-1">
                  <Check className="h-3 w-3" aria-hidden="true" />
                  {t("modelSelector.active")}
                </Badge>
              )}
              {isRecommended && (
                <Badge variant="active">{t("onboarding.recommended")}</Badge>
              )}
            </div>

            <p className="mt-1 max-w-[68ch] text-xs leading-relaxed text-muted">
              {description}
            </p>

            <div className="mt-2 flex flex-wrap items-center gap-1.5">
              {meta ? (
                <span className="inline-flex items-center gap-1.5 rounded-md bg-surface-strong px-2 py-1 text-[11px] font-medium text-muted">
                  {meta.supportsVision ? (
                    <Eye className="h-3.5 w-3.5" aria-hidden="true" />
                  ) : (
                    <MessageSquareText
                      className="h-3.5 w-3.5"
                      aria-hidden="true"
                    />
                  )}
                  {t(
                    meta.supportsVision
                      ? "onboarding.aiModel.seesScreen"
                      : "onboarding.aiModel.textOnly",
                  )}
                </span>
              ) : model.is_custom ? (
                <span className="inline-flex items-center rounded-md bg-surface-strong px-2 py-1 text-[11px] font-medium text-muted">
                  {t("settings.assistant.characters.custom")}
                </span>
              ) : null}
              <span className="inline-flex items-center gap-1.5 rounded-md bg-surface-strong px-2 py-1 text-[11px] font-medium tabular-nums text-muted">
                <HardDrive className="h-3.5 w-3.5" aria-hidden="true" />
                {formatModelSize(Number(model.size_mb))}
              </span>
            </div>
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-2 ps-[3.375rem] sm:ps-0">
          <button
            type="button"
            aria-expanded={detailsOpen}
            aria-controls={detailsId}
            onClick={() => setDetailsOpen((open) => !open)}
            className="inline-flex cursor-pointer items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium text-muted transition-colors duration-150 hover:bg-surface-strong hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
          >
            <Info className="h-3.5 w-3.5" aria-hidden="true" />
            {t("settings.assistant.brain.details")}
            <ChevronDown
              className={`h-3.5 w-3.5 transition-transform duration-150 motion-reduce:transition-none ${detailsOpen ? "rotate-180" : ""}`}
              aria-hidden="true"
            />
          </button>

          {status === "downloadable" && (
            <Button
              variant="primary"
              size="sm"
              onClick={() => onDownload(model.id)}
            >
              <Download className="h-3.5 w-3.5" aria-hidden="true" />
              {t("modelSelector.download")}
            </Button>
          )}
          {status === "available" && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onSelect(model.id)}
            >
              {t("modelSelector.useModel")}
            </Button>
          )}
        </div>
      </div>

      {isBusy && (
        <div className="px-4 pb-4 sm:ps-[4.375rem]">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-mid-gray/20">
            <div
              className={`h-full rounded-full bg-accent ${status === "downloading" ? "transition-[width] duration-300" : "w-full animate-pulse"}`}
              style={
                status === "downloading" ? { width: `${progress}%` } : undefined
              }
            />
          </div>
          <div className="mt-1.5 flex items-center justify-between gap-3 text-xs text-muted">
            <span>
              {status === "downloading"
                ? t("modelSelector.downloading", {
                    percentage: Math.round(progress),
                  })
                : status === "verifying"
                  ? t("modelSelector.verifyingGeneric")
                  : t("modelSelector.extractingGeneric")}
            </span>
            <span className="flex items-center gap-2">
              {status === "downloading" &&
                downloadSpeed !== undefined &&
                downloadSpeed > 0 && (
                  <span className="tabular-nums">
                    {t("modelSelector.downloadSpeed", {
                      speed: downloadSpeed.toFixed(1),
                    })}
                  </span>
                )}
              {status === "downloading" && (
                <Button
                  variant="danger-ghost"
                  size="sm"
                  onClick={() => onCancel(model.id)}
                >
                  {t("modelSelector.cancel")}
                </Button>
              )}
            </span>
          </div>
        </div>
      )}

      {detailsOpen && (
        <div
          id={detailsId}
          className="border-t border-hairline bg-surface-strong/35 px-4 py-3.5 sm:ps-[4.375rem]"
        >
          <div className="flex flex-wrap items-center gap-2">
            {quant && (
              <span className="rounded-md border border-hairline-strong px-2 py-1 font-mono text-[10px] text-muted">
                {quant}
              </span>
            )}
            {meta?.supportsVision && (
              <span className="text-[11px] text-muted">
                {t("settings.assistant.brain.visionModelNote")}
              </span>
            )}
            {deleteBlocked && (
              <span className="ms-auto text-[11px] text-muted">
                {t("settings.assistant.brain.switchBeforeDelete")}
              </span>
            )}
            {canDelete && (
              <Button
                variant="danger-ghost"
                size="sm"
                onClick={() => onDelete(model.id)}
                className="ms-auto"
              >
                <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
                {t("common.delete")}
              </Button>
            )}
          </div>
        </div>
      )}
    </article>
  );
};

/**
 * On-device assistant model browser. The short curated list is ordered by
 * conversational responsiveness and capability; hardware facts are shown as
 * context only and never converted into an automatic model ranking.
 */
export const LlmCatalog: React.FC = () => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [hardware, setHardware] = useState<{
    acceleratorName?: string;
    acceleratorKind?: string;
    acceleratorMemoryGb?: number;
    systemMemoryGb: number;
  } | null>(null);
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

  useEffect(() => {
    let cancelled = false;

    void Promise.allSettled([
      commands.getSystemMemoryGb(),
      commands.getAvailableAccelerators(),
    ]).then(([memoryResult, acceleratorsResult]) => {
      if (cancelled) return;

      const systemMemoryGb =
        memoryResult.status === "fulfilled" ? memoryResult.value : 0;
      const devices =
        acceleratorsResult.status === "fulfilled"
          ? acceleratorsResult.value.gpu_devices
          : [];
      const accelerator = devices.reduce<(typeof devices)[number] | undefined>(
        (best, device) => {
          if (!best) return device;
          const priority = (kind: string) =>
            kind === "dedicated" ? 2 : kind === "unknown" ? 1 : 0;
          const devicePriority = priority(device.kind);
          const bestPriority = priority(best.kind);
          if (devicePriority !== bestPriority) {
            return devicePriority > bestPriority ? device : best;
          }
          return device.total_vram_mb > best.total_vram_mb ? device : best;
        },
        undefined,
      );

      setHardware({
        acceleratorName: accelerator?.name,
        acceleratorKind: accelerator?.kind,
        acceleratorMemoryGb: accelerator
          ? accelerator.total_vram_mb / 1024
          : undefined,
        systemMemoryGb,
      });
    });

    return () => {
      cancelled = true;
    };
  }, []);

  const activeModelId = settings?.assistant_models?.[BUILTIN_PROVIDER_ID] ?? "";
  const providerIsBuiltin =
    settings?.assistant_provider_id === BUILTIN_PROVIDER_ID;

  const llmModels = useMemo(
    () =>
      models
        .filter((model: ModelInfo) => getModelCategory(model) === "llm")
        .sort((a: ModelInfo, b: ModelInfo) =>
          getTranslatedModelName(a, t).localeCompare(
            getTranslatedModelName(b, t),
          ),
        ),
    [models, t],
  );

  const modelById = useMemo(
    () => new Map(llmModels.map((model) => [model.id, model])),
    [llmModels],
  );
  const recommendedModels = RECOMMENDED_LOCAL_MODELS.flatMap((meta) => {
    const model = modelById.get(meta.id);
    return model ? [{ model, meta }] : [];
  });
  const activeModel = providerIsBuiltin
    ? llmModels.find(
        (model) => model.id === activeModelId && model.is_downloaded,
      )
    : undefined;
  const unlistedActiveModel =
    activeModel &&
    !RECOMMENDED_MODEL_IDS.has(activeModel.id as RecommendedModelId)
      ? activeModel
      : undefined;
  const savedModels = llmModels.filter((model) => {
    const isCurated = RECOMMENDED_MODEL_IDS.has(model.id as RecommendedModelId);
    const isCurrent = model.id === unlistedActiveModel?.id;
    return !isCurated && !isCurrent && (model.is_custom || model.is_downloaded);
  });

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
    const model = models.find(
      (candidate: ModelInfo) => candidate.id === modelId,
    );
    const modelName = model ? getTranslatedModelName(model, t) : modelId;
    const confirmed = await ask(
      t("settings.assistant.brain.deleteModelConfirm", { modelName }),
      {
        title: t("settings.models.deleteTitle"),
        kind: "warning",
      },
    );
    if (!confirmed) return;

    const deleted = await deleteModel(modelId);
    if (!deleted) {
      toast.error(t("settings.assistant.brain.deleteModelFailed"), {
        description: useModelStore.getState().error ?? undefined,
      });
      return;
    }
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

  const renderModel = (
    model: ModelInfo,
    meta?: RecommendedModelMeta,
    isRecommended = false,
  ) => (
    <CatalogModelRow
      key={model.id}
      model={model}
      status={statusFor(model)}
      meta={meta}
      isRecommended={isRecommended}
      protectedFromDelete={model.id === activeModelId}
      onSelect={handleSelect}
      onDownload={(modelId) => void handleDownload(modelId)}
      onDelete={(modelId) => void handleDelete(modelId)}
      onCancel={cancelDownload}
      downloadProgress={downloadProgress[model.id]?.percentage}
      downloadSpeed={downloadStats[model.id]?.speed}
    />
  );

  return (
    <div className="space-y-8">
      {unlistedActiveModel && (
        <section aria-labelledby="current-local-model" className="space-y-2.5">
          <div className="px-1">
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
          <div className="overflow-hidden rounded-2xl border border-hairline-strong">
            {renderModel(unlistedActiveModel)}
          </div>
        </section>
      )}

      <section aria-labelledby="recommended-local-models" className="space-y-3">
        <div className="flex flex-col gap-2 px-1 sm:flex-row sm:items-end sm:justify-between">
          <div>
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
          {hardware && (
            <span className="inline-flex w-fit items-center gap-1.5 rounded-lg border border-hairline bg-surface px-2.5 py-1.5 text-[11px] font-medium tabular-nums text-muted">
              <MemoryStick className="h-3.5 w-3.5" aria-hidden="true" />
              {hardware.acceleratorName && hardware.acceleratorMemoryGb
                ? t(
                    hardware.acceleratorKind === "dedicated"
                      ? "settings.assistant.brain.acceleratorDetectedDedicated"
                      : hardware.acceleratorKind === "integrated"
                        ? "settings.assistant.brain.acceleratorDetectedIntegrated"
                        : "settings.assistant.brain.acceleratorDetected",
                    {
                      name: hardware.acceleratorName,
                      memory: Number(hardware.acceleratorMemoryGb.toFixed(1)),
                    },
                  )
                : hardware.acceleratorName
                  ? t("settings.assistant.brain.acceleratorDetectedNoMemory", {
                      name: hardware.acceleratorName,
                    })
                  : hardware.systemMemoryGb > 0
                    ? t(
                        "settings.assistant.brain.acceleratorUnknownWithMemory",
                        {
                          memory: hardware.systemMemoryGb,
                        },
                      )
                    : t("settings.assistant.brain.acceleratorUnknown")}
            </span>
          )}
        </div>

        {recommendedModels.length === 0 ? (
          <div className="rounded-2xl border border-dashed border-hairline-strong px-4 py-5 text-center">
            <p className="text-xs text-muted">
              {t("settings.assistant.brain.catalogEmpty")}
            </p>
          </div>
        ) : (
          <div className="overflow-hidden rounded-2xl border border-hairline-strong bg-surface divide-y divide-hairline">
            {recommendedModels.map(({ model, meta }) =>
              renderModel(model, meta, meta.isRecommended),
            )}
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
          className="group flex w-full cursor-pointer items-center gap-4 rounded-2xl border border-hairline-strong bg-surface px-4 py-4 text-start transition-[background-color,border-color,box-shadow,transform] duration-150 hover:border-accent/40 hover:bg-accent/[0.035] hover:shadow-[0_10px_28px_-22px_var(--color-accent)] focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 active:scale-[0.99]"
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

      {savedModels.length > 0 && (
        <section aria-labelledby="saved-local-models" className="space-y-3">
          <div className="px-1">
            <h2
              id="saved-local-models"
              className="text-[13.5px] font-semibold tracking-tight text-ink"
            >
              {t("settings.assistant.brain.huggingFaceTitle")}
            </h2>
            <p className="mt-0.5 max-w-[62ch] text-xs leading-relaxed text-muted">
              {t("settings.assistant.brain.huggingFaceDescription")}
            </p>
          </div>
          <div className="overflow-hidden rounded-2xl border border-hairline-strong bg-surface divide-y divide-hairline">
            {savedModels.map((model) => renderModel(model))}
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
