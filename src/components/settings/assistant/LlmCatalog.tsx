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
  FileQuestion,
  HardDrive,
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
import { AddLocalModelDialog } from "../models/AddLocalModelDialog";
import type { ModelCardStatus } from "../../onboarding/ModelCard";

/** The built-in (local) llama.cpp provider id, mirrored from the backend. */
const BUILTIN_PROVIDER_ID = "builtin";

/**
 * Which job the catalog is picking a model for.
 *
 * The two roles want genuinely different models — a 0.8B cleanup fine-tune is
 * the best choice for dictation cleanup and useless as a conversational
 * assistant — so they get different featured lists and write to different
 * provider slots. They share everything else: one download list, one Hugging
 * Face importer, one "use a model I already have" flow. Splitting the component
 * instead would have meant maintaining two copies of the download UI, and
 * routing cleanup through the assistant's page (the old behaviour) meant a user
 * changing their cleanup model had to visit a page about something else.
 */
export type LlmCatalogRole = "assistant" | "cleanup";

/**
 * A deliberately small, conversation-first set. The recommendation is an
 * editorial quality/latency choice, never a hardware score.
 */
const RECOMMENDED_ASSISTANT_MODELS = [
  { id: "gemma-4-e2b", supportsVision: true, isRecommended: false },
  { id: "gemma-4-e4b", supportsVision: true, isRecommended: true },
  { id: "gemma-4-12b", supportsVision: true, isRecommended: false },
] as const;

/**
 * The cleanup shortlist: our own fine-tune, then the smallest general models
 * that can do the job. Anything larger is a waste here — cleanup runs after
 * every dictation, so latency matters more than capability, and the transform is
 * narrow enough that a specialist beats a bigger generalist.
 */
const RECOMMENDED_CLEANUP_MODELS = [
  { id: "speakoflow-mini", supportsVision: false, isRecommended: true },
  { id: "gemma-3-1b", supportsVision: false, isRecommended: false },
  { id: "gemma-4-e2b", supportsVision: true, isRecommended: false },
] as const;

type RecommendedModelMeta = {
  id: string;
  supportsVision: boolean;
  isRecommended: boolean;
};

const RECOMMENDED_BY_ROLE: Record<
  LlmCatalogRole,
  readonly RecommendedModelMeta[]
> = {
  assistant: RECOMMENDED_ASSISTANT_MODELS,
  cleanup: RECOMMENDED_CLEANUP_MODELS,
};

interface CatalogModelRowProps {
  model: ModelInfo;
  status: ModelCardStatus;
  meta?: RecommendedModelMeta;
  isRecommended?: boolean;
  /** Screen vision is meaningless for a cleanup model — don't advertise it. */
  hideVisionPill?: boolean;
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
  hideVisionPill = false,
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
  // For a model the user registered from disk, the "description" is really its
  // full path — the longest string in the list, and what turned every row into
  // three lines. Show the filename inline and keep the path in the fold, where a
  // path is the thing you actually came looking for.
  const fileName =
    (model.local_path ?? model.filename).split(/[\\/]/).pop() ?? model.filename;
  const subtitle = model.local_path ? fileName : description;
  const isBusy =
    status === "downloading" ||
    status === "verifying" ||
    status === "extracting";
  // Registered from the user's own disk, but the file is no longer there.
  const isLocalFileMissing = !!model.local_path && !model.is_downloaded;
  // A model discovered inside a linked folder can't be removed one at a time —
  // the next scan finds it again. Unlinking the folder is the action that works,
  // so don't offer one that doesn't. A stale individually-picked entry, though,
  // is exactly the thing the user needs to be able to clear.
  const deleteEligible =
    (model.is_custom || model.is_downloaded || isLocalFileMissing) &&
    !model.local_folder;
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

            {/* One line, always. The full text lives in the fold, so a long path
                or a three-sentence description can't stretch the row. */}
            <p className="mt-1 truncate text-xs leading-relaxed text-muted">
              {subtitle}
            </p>

            <div className="mt-2 flex flex-wrap items-center gap-1.5">
              {meta && !hideVisionPill ? (
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

        <div className="flex shrink-0 items-center gap-1.5 ps-[3.375rem] sm:ps-0">
          {/* The disclosure now earns its label: the row shows one line, and this
              reveals the full description, the file, its location on disk, the
              quantization, and the id — none of which is visible above. */}
          <button
            type="button"
            aria-expanded={detailsOpen}
            aria-controls={detailsId}
            onClick={() => setDetailsOpen((open) => !open)}
            className="inline-flex cursor-pointer items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium text-muted transition-colors duration-150 hover:bg-surface-strong hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
          >
            {t("settings.assistant.brain.details")}
            <ChevronDown
              className={`h-3.5 w-3.5 transition-transform duration-150 motion-reduce:transition-none ${detailsOpen ? "rotate-180" : ""}`}
              aria-hidden="true"
            />
          </button>

          {/* A model the user registered from their own disk has no download URL.
              When it reads as not-on-disk the file moved, was renamed, or is on a
              drive that isn't connected — so "Download" is an action that cannot
              succeed. Say what's wrong and leave only the action that works:
              removing the stale entry. */}
          {status === "downloadable" &&
            (isLocalFileMissing ? (
              <span
                className="inline-flex items-center gap-1.5 rounded-md bg-surface-strong px-2.5 py-1.5 text-[11px] font-medium text-muted"
                title={
                  model.local_path
                    ? t("settings.models.localModel.fileMissing", {
                        path: model.local_path,
                      })
                    : undefined
                }
              >
                <FileQuestion className="h-3.5 w-3.5" aria-hidden="true" />
                {t("settings.models.localModel.missingBadge")}
              </span>
            ) : (
              <Button
                variant="primary"
                size="sm"
                onClick={() => onDownload(model.id)}
              >
                <Download className="h-3.5 w-3.5" aria-hidden="true" />
                {t("modelSelector.download")}
              </Button>
            ))}
          {status === "available" && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => onSelect(model.id)}
            >
              {t("modelSelector.useModel")}
            </Button>
          )}
          {/* Delete lived behind a "Details" disclosure, which was the whole
              problem with that control: the row's description is already visible,
              so expanding "Details" revealed no details — just this button. An
              icon action states what it is, and the confirm dialog (not a
              disclosure) is what protects against a misclick. */}
          {(canDelete || deleteBlocked) && (
            <button
              type="button"
              onClick={canDelete ? () => onDelete(model.id) : undefined}
              disabled={!canDelete}
              aria-label={t("common.delete")}
              title={
                deleteBlocked
                  ? t("settings.assistant.brain.switchBeforeDelete")
                  : t("common.delete")
              }
              className={`inline-flex h-8 w-8 items-center justify-center rounded-lg transition-colors duration-150 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${
                canDelete
                  ? "cursor-pointer text-muted hover:bg-red-500/10 hover:text-red-600 dark:hover:text-red-400"
                  : "cursor-not-allowed text-muted-soft opacity-50"
              }`}
            >
              <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
            </button>
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
          {/* The full description first — for a local model the row only showed
              its filename, so this is the one place the architecture and vision
              support are stated. */}
          <p className="text-xs leading-relaxed text-muted">{description}</p>
          <dl className="mt-3 grid gap-x-4 gap-y-1.5 text-[11px] sm:grid-cols-[auto_1fr]">
            <dt className="font-medium text-muted">
              {t("settings.assistant.brain.detailFile")}
            </dt>
            <dd className="break-all font-mono text-muted">{fileName}</dd>

            {model.local_path && (
              <>
                <dt className="font-medium text-muted">
                  {t("settings.assistant.brain.detailLocation")}
                </dt>
                <dd className="break-all font-mono text-muted">
                  {model.local_path}
                </dd>
              </>
            )}

            {quant && (
              <>
                <dt className="font-medium text-muted">
                  {t("settings.assistant.brain.detailFormat")}
                </dt>
                <dd className="font-mono text-muted">{quant}</dd>
              </>
            )}

            <dt className="font-medium text-muted">
              {t("settings.assistant.brain.detailId")}
            </dt>
            <dd className="break-all font-mono text-muted">{model.id}</dd>
          </dl>
        </div>
      )}
    </article>
  );
};

/**
 * On-device model browser, shared by the assistant and dictation cleanup. The
 * short curated list is ordered by responsiveness and capability for the role;
 * hardware facts are shown as context only and never converted into an automatic
 * model ranking.
 */
export const LlmCatalog: React.FC<{ role?: LlmCatalogRole }> = ({
  role = "assistant",
}) => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [localDialogOpen, setLocalDialogOpen] = useState(false);
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

  const isCleanup = role === "cleanup";
  const activeModelId =
    (isCleanup
      ? settings?.post_process_models?.[BUILTIN_PROVIDER_ID]
      : settings?.assistant_models?.[BUILTIN_PROVIDER_ID]) ?? "";
  const providerIsBuiltin =
    (isCleanup
      ? settings?.post_process_provider_id
      : settings?.assistant_provider_id) === BUILTIN_PROVIDER_ID;
  // What the *other* role is using. Only needed to protect it from deletion:
  // assistant and cleanup share one download list, so each can delete the
  // other's model out from under it.
  const otherRoleModelId =
    (isCleanup
      ? settings?.assistant_models?.[BUILTIN_PROVIDER_ID]
      : settings?.post_process_models?.[BUILTIN_PROVIDER_ID]) ?? "";

  const llmModels = useMemo(
    () =>
      models
        .filter((model: ModelInfo) => {
          if (getModelCategory(model) !== "llm") return false;
          // A cleanup fine-tune cannot hold a conversation, so it is not offered
          // as an assistant brain at all — hiding it is kinder than letting
          // someone select it and conclude the assistant is broken.
          if (!isCleanup && model.is_cleanup_specialist) return false;
          return true;
        })
        .sort((a: ModelInfo, b: ModelInfo) =>
          getTranslatedModelName(a, t).localeCompare(
            getTranslatedModelName(b, t),
          ),
        ),
    [models, t, isCleanup],
  );

  const modelById = useMemo(
    () => new Map(llmModels.map((model) => [model.id, model])),
    [llmModels],
  );
  const recommended = RECOMMENDED_BY_ROLE[role];
  const recommendedIds = useMemo(
    () => new Set(recommended.map((meta) => meta.id)),
    [recommended],
  );
  const recommendedModels = recommended.flatMap((meta) => {
    const model = modelById.get(meta.id);
    return model ? [{ model, meta }] : [];
  });
  const activeModel = providerIsBuiltin
    ? llmModels.find(
        (model) => model.id === activeModelId && model.is_downloaded,
      )
    : undefined;
  const unlistedActiveModel =
    activeModel && !recommendedIds.has(activeModel.id)
      ? activeModel
      : undefined;
  const savedModels = llmModels.filter((model) => {
    const isCurated = recommendedIds.has(model.id);
    const isCurrent = model.id === unlistedActiveModel?.id;
    return !isCurated && !isCurrent && (model.is_custom || model.is_downloaded);
  });

  const wireUpProvider = async (modelId: string) => {
    if (isCleanup) {
      // One command: it also keeps the selected cleanup prompt paired with the
      // model, which two separate calls could not do atomically.
      await commands.setCleanupLocalModel(modelId);
    } else {
      await commands.changeAssistantModelSetting(BUILTIN_PROVIDER_ID, modelId);
      if (!providerIsBuiltin) {
        await commands.setAssistantProvider(BUILTIN_PROVIDER_ID);
      }
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
    // A model the user registered from their own disk has no download URL — if
    // it reads as not-on-disk, the file moved, was renamed, or lives on a drive
    // that isn't connected. Say that, rather than starting a download that
    // cannot succeed and failing for a reason that looks unrelated.
    if (model?.local_path) {
      toast.error(
        t("settings.models.localModel.fileMissing", { path: model.local_path }),
      );
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
      // "Delete" would be a lie for a file we don't own: registering one copies
      // nothing, so removing it only forgets the path.
      model?.local_path
        ? t("settings.models.localModel.removeConfirm", {
            modelName,
            path: model.local_path,
          })
        : t("settings.assistant.brain.deleteModelConfirm", { modelName }),
      {
        title: model?.local_path
          ? t("settings.models.localModel.removeTitle")
          : t("settings.models.deleteTitle"),
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
      hideVisionPill={isCleanup}
      // Both slots, not just this role's. The two roles point at the same engine
      // and the same download, so a model that is in use anywhere must not be
      // deletable from here — the backend refuses it either way, and offering a
      // button that fails is worse than not offering it.
      protectedFromDelete={
        model.id === activeModelId || model.id === otherRoleModelId
      }
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
              {t(
                isCleanup
                  ? "settings.dictation.aiCleanup.catalog.recommendedTitle"
                  : "settings.assistant.brain.recommendedTitle",
              )}
            </h2>
            <p className="mt-0.5 max-w-[62ch] text-xs leading-relaxed text-muted">
              {t(
                isCleanup
                  ? "settings.dictation.aiCleanup.catalog.recommendedDescription"
                  : "settings.assistant.brain.recommendedDescription",
              )}
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
        {/* Heading only. The two cards below already carry their own titles and
            subtitles, so a section description here said the same thing twice. */}
        <div className="px-1">
          <h2
            id="hugging-face-finder"
            className="text-[13.5px] font-semibold tracking-tight text-ink"
          >
            {t("settings.assistant.brain.finderSectionTitle")}
          </h2>
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
        <button
          type="button"
          onClick={() => setLocalDialogOpen(true)}
          className="group flex w-full cursor-pointer items-center gap-4 rounded-2xl border border-hairline-strong bg-surface px-4 py-4 text-start transition-[background-color,border-color,box-shadow,transform] duration-150 hover:border-accent/40 hover:bg-accent/[0.035] hover:shadow-[0_10px_28px_-22px_var(--color-accent)] focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 active:scale-[0.99]"
        >
          <span className="grid h-11 w-11 shrink-0 place-items-center rounded-xl bg-accent/12 text-accent">
            <HardDrive className="h-5 w-5" aria-hidden="true" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-sm font-semibold text-ink">
              {t("settings.models.localModel.title")}
            </span>
            <span className="mt-0.5 block text-xs leading-relaxed text-muted">
              {t("settings.models.localModel.subtitle")}
            </span>
          </span>
          <span className="hidden shrink-0 items-center gap-1.5 rounded-lg bg-surface-strong px-3 py-2 text-xs font-semibold text-ink transition-colors group-hover:bg-accent group-hover:text-on-primary sm:flex">
            {t("settings.models.localModel.addButton")}
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
          {/* "Your models" is self-describing; the rows carry the detail. */}
          <div className="px-1">
            <h2
              id="saved-local-models"
              className="text-[13.5px] font-semibold tracking-tight text-ink"
            >
              {t("settings.assistant.brain.huggingFaceTitle")}
            </h2>
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
      <AddLocalModelDialog
        open={localDialogOpen}
        onClose={() => setLocalDialogOpen(false)}
      />
    </div>
  );
};

export default LlmCatalog;
