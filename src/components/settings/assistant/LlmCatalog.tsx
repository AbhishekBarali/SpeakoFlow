import React, { useEffect, useMemo, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  HardDrive,
  Info,
  MemoryStick,
  MessageCircle,
  Search,
  Trash2,
  Wand2,
} from "lucide-react";
import { commands, type ModelInfo, type Result } from "@/bindings";
import { formatModelSize } from "@/lib/utils/format";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { splitLocalModelName } from "@/lib/utils/prettyModelName";
import {
  getTranslatedModelDescription,
  getTranslatedModelName,
} from "@/lib/utils/modelTranslation";
import { useModelStore } from "@/stores/modelStore";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "../../ui/Button";
import { InfoTip } from "../../ui/InfoTip";
import type { MenuItem } from "../../ui/Menu";
import { AddCustomModelDialog } from "../models/AddCustomModelDialog";
import { AddLocalModelDialog } from "../models/AddLocalModelDialog";
import {
  ModelRow,
  ModelTag,
  type ModelRowAction,
  type ModelRowDetail,
} from "../models/ModelRow";

/** The built-in (local) llama.cpp provider id, mirrored from the backend. */
const BUILTIN_PROVIDER_ID = "builtin";

/**
 * Which job the catalog is picking a model for.
 *
 * The two roles want genuinely different models — a 0.8B cleanup fine-tune is
 * the best choice for dictation cleanup and useless as a conversational
 * assistant — so they get different featured lists. They share everything
 * else: one download list, one Hugging Face importer, one "use a model I
 * already have" flow, so a model downloaded from either tab can be put to
 * either job from either tab.
 */
export type LlmCatalogRole = "assistant" | "cleanup";

/** Conversation-first picks. An editorial quality/latency choice, never a
 *  hardware score. */
const RECOMMENDED_ASSISTANT_MODELS = [
  { id: "gemma-4-e2b", isRecommended: false },
  { id: "gemma-4-e4b", isRecommended: true },
  { id: "gemma-4-12b", isRecommended: false },
] as const;

/** Our own fine-tune, then the smallest general models that do the job:
 *  cleanup runs after every dictation, so latency beats capability. */
const RECOMMENDED_CLEANUP_MODELS = [
  { id: "speakoflow-mini", isRecommended: true },
  { id: "gemma-3-1b", isRecommended: false },
  { id: "gemma-4-e2b", isRecommended: false },
] as const;

type RecommendedModelMeta = { id: string; isRecommended: boolean };

const RECOMMENDED_BY_ROLE: Record<
  LlmCatalogRole,
  readonly RecommendedModelMeta[]
> = {
  assistant: RECOMMENDED_ASSISTANT_MODELS,
  cleanup: RECOMMENDED_CLEANUP_MODELS,
};

type Busy = "downloading" | "verifying" | "extracting" | null;

interface RoleUse {
  assistant: boolean;
  cleanup: boolean;
}

/** Throw on a failed command so a caller can report it. */
const ok = async (result: Promise<Result<unknown, string>>) => {
  const settled = await result;
  if (settled.status !== "ok") throw new Error(String(settled.error));
};

/** One catalog model: its state decides the one action on the right. */
const CatalogModelRow: React.FC<{
  model: ModelInfo;
  role: LlmCatalogRole;
  busy: Busy;
  inUse: RoleUse;
  isRecommended: boolean;
  protectedFromDelete: boolean;
  downloadProgress?: number;
  downloadSpeed?: number;
  onUse: (modelId: string, role: LlmCatalogRole) => void;
  onDownload: (modelId: string) => void;
  onDelete: (modelId: string) => void;
  onCancel: (modelId: string) => void;
}> = ({
  model,
  role,
  busy,
  inUse,
  isRecommended,
  protectedFromDelete,
  downloadProgress,
  downloadSpeed,
  onUse,
  onDownload,
  onDelete,
  onCancel,
}) => {
  const { t } = useTranslation();
  const [showDetails, setShowDetails] = useState(false);
  const { name, quant } = splitLocalModelName(
    getTranslatedModelName(model, t),
    model.filename,
  );
  const description = getTranslatedModelDescription(model, t);
  // A model the user registered from disk has its path as its "description";
  // show the file name and keep the path in the details.
  const fileName =
    (model.local_path ?? model.filename).split(/[\\/]/).pop() ?? model.filename;
  const subtitle = model.local_path ? fileName : description;
  const missing = !!model.local_path && !model.is_downloaded;
  const otherRole: LlmCatalogRole =
    role === "cleanup" ? "assistant" : "cleanup";
  const jobName = (target: LlmCatalogRole) =>
    target === "cleanup"
      ? t("catalog.useFor.cleanup")
      : t("catalog.useFor.assistant");

  let action: ModelRowAction;
  if (busy) action = { kind: "none" };
  else if (missing)
    action = {
      kind: "missing",
      title: model.local_path
        ? t("settings.models.localModel.fileMissing", {
            path: model.local_path,
          })
        : undefined,
    };
  else if (!model.is_downloaded)
    action = { kind: "download", onClick: () => onDownload(model.id) };
  else if (inUse[role]) action = { kind: "inUse" };
  else
    action = {
      kind: "use",
      onClick: () => onUse(model.id, role),
      ariaLabel: t("catalog.useNamed", { model: name, job: jobName(role) }),
    };

  // A model found in a linked folder can't be removed on its own — the next
  // scan finds it again — so only offer removal where it works.
  const removable =
    (model.is_custom || model.is_downloaded || missing) && !model.local_folder;

  const menu: MenuItem[] = [];
  if (model.is_downloaded && !busy) {
    menu.push({
      id: "other-role",
      icon: otherRole === "cleanup" ? Wand2 : MessageCircle,
      label: t("catalog.useForJob", { job: jobName(otherRole) }),
      hint:
        otherRole === "assistant" && model.is_cleanup_specialist
          ? t("catalog.useFor.cleanupOnly")
          : undefined,
      checked: inUse[otherRole],
      disabled:
        inUse[otherRole] ||
        (otherRole === "assistant" && model.is_cleanup_specialist),
      onSelect: () => onUse(model.id, otherRole),
    });
  }
  menu.push({
    id: "details",
    icon: Info,
    label: showDetails ? t("catalog.hideDetails") : t("catalog.showDetails"),
    onSelect: () => setShowDetails((open) => !open),
  });
  if (removable && !busy) {
    menu.push({
      id: "delete",
      icon: Trash2,
      tone: "danger",
      separated: true,
      label: model.local_path
        ? t("catalog.removeFromList")
        : t("common.delete"),
      hint: protectedFromDelete
        ? t("settings.assistant.brain.switchBeforeDelete")
        : undefined,
      disabled: protectedFromDelete,
      onSelect: () => onDelete(model.id),
    });
  }

  const details: ModelRowDetail[] = [
    {
      label: t("settings.assistant.brain.detailFile"),
      value: fileName,
      mono: true,
    },
    ...(model.local_path
      ? [
          {
            label: t("settings.assistant.brain.detailLocation"),
            value: model.local_path,
            mono: true,
          },
        ]
      : []),
    ...(quant
      ? [
          {
            label: t("settings.assistant.brain.detailFormat"),
            value: quant,
            mono: true,
          },
        ]
      : []),
    {
      label: t("settings.assistant.brain.detailId"),
      value: model.id,
      mono: true,
    },
  ];

  return (
    <ModelRow
      model={model}
      name={name}
      quant={quant}
      subtitle={showDetails && model.local_path ? description : subtitle}
      size={formatModelSize(Number(model.size_mb))}
      current={inUse[role]}
      badges={
        <>
          {inUse[otherRole] && (
            <ModelTag tone="accent">{jobName(otherRole)}</ModelTag>
          )}
          {isRecommended && !inUse.assistant && !inUse.cleanup && (
            <ModelTag>{t("onboarding.recommended")}</ModelTag>
          )}
        </>
      }
      action={action}
      menu={menu}
      progress={
        busy
          ? {
              state: busy,
              percent: downloadProgress,
              speed: downloadSpeed,
              onCancel:
                busy === "downloading" ? () => onCancel(model.id) : undefined,
            }
          : null
      }
      details={showDetails ? details : null}
    />
  );
};

/** Small heading for a list inside a tab: sans, with context in an (i). */
const ListHeading: React.FC<{
  id: string;
  title: string;
  info?: string;
  trailing?: React.ReactNode;
}> = ({ id, title, info, trailing }) => (
  <div className="mb-2.5 flex flex-wrap items-center justify-between gap-2 px-1">
    <div className="flex items-center gap-1">
      <h3 id={id} className="text-[0.8125rem] font-semibold text-muted">
        {title}
      </h3>
      {info && <InfoTip text={info} />}
    </div>
    {trailing}
  </div>
);

/**
 * On-device model browser, shared by the assistant and AI cleanup tabs. The
 * short curated list is ordered by responsiveness and capability for the role;
 * hardware facts are shown as context only.
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
  const assistantOnDevice =
    settings?.assistant_provider_id === BUILTIN_PROVIDER_ID;
  const cleanupOnDevice =
    settings?.post_process_provider_id === BUILTIN_PROVIDER_ID;
  const assistantModelId =
    settings?.assistant_models?.[BUILTIN_PROVIDER_ID] ?? "";
  const cleanupModelId =
    settings?.post_process_models?.[BUILTIN_PROVIDER_ID] ?? "";

  const inUseFor = (model: ModelInfo): RoleUse => ({
    assistant:
      model.is_downloaded && assistantOnDevice && model.id === assistantModelId,
    cleanup:
      model.is_downloaded && cleanupOnDevice && model.id === cleanupModelId,
  });

  const llmModels = useMemo(
    () =>
      models
        .filter((model: ModelInfo) => {
          if (getModelCategory(model) !== "llm") return false;
          // A cleanup fine-tune cannot hold a conversation, so the assistant
          // tab does not list it at all.
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
  const savedModels = llmModels.filter(
    (model) =>
      !recommendedIds.has(model.id) && (model.is_custom || model.is_downloaded),
  );

  const nameOf = (modelId: string) => {
    const model = models.find((candidate) => candidate.id === modelId);
    return model
      ? splitLocalModelName(getTranslatedModelName(model, t), model.filename)
          .name
      : modelId;
  };

  const useFor = async (modelId: string, target: LlmCatalogRole) => {
    try {
      if (target === "cleanup") {
        // One command: it also keeps the selected cleanup prompt paired with
        // the model, which two separate calls could not do atomically.
        await ok(commands.setCleanupLocalModel(modelId));
      } else {
        await ok(
          commands.changeAssistantModelSetting(BUILTIN_PROVIDER_ID, modelId),
        );
        if (!assistantOnDevice) {
          await ok(commands.setAssistantProvider(BUILTIN_PROVIDER_ID));
        }
      }
      await refreshSettings();
      toast.success(
        t("catalog.nowUsing", {
          model: nameOf(modelId),
          job:
            target === "cleanup"
              ? t("catalog.useFor.cleanup")
              : t("catalog.useFor.assistant"),
        }),
      );
    } catch (error) {
      console.error("Failed to assign model:", error);
      toast.error(t("pickers.switchFailed"));
      await refreshSettings();
    }
  };

  const handleDownload = async (modelId: string) => {
    const model = models.find(
      (candidate: ModelInfo) => candidate.id === modelId,
    );
    if (model?.is_downloaded) {
      await useFor(modelId, role);
      return;
    }
    // A model registered from disk has no download URL; if it reads as
    // missing, the file moved or its drive is disconnected.
    if (model?.local_path) {
      toast.error(
        t("settings.models.localModel.fileMissing", { path: model.local_path }),
      );
      return;
    }
    const downloaded = await downloadModel(modelId);
    // A fresh download goes to work for the tab it was downloaded from.
    if (downloaded) await useFor(modelId, role);
  };

  const handleDelete = async (modelId: string) => {
    const model = models.find(
      (candidate: ModelInfo) => candidate.id === modelId,
    );
    const modelName = model ? nameOf(modelId) : modelId;
    const confirmed = await ask(
      // "Delete" would be a lie for a file we don't own: registering one
      // copies nothing, so removing it only forgets the path.
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

  const busyOf = (model: ModelInfo): Busy =>
    model.id in extractingModels
      ? "extracting"
      : model.id in verifyingModels
        ? "verifying"
        : model.id in downloadingModels
          ? "downloading"
          : null;

  const renderModel = (model: ModelInfo, isRecommended = false) => (
    <CatalogModelRow
      key={model.id}
      model={model}
      role={role}
      busy={busyOf(model)}
      inUse={inUseFor(model)}
      isRecommended={isRecommended}
      // Both jobs share one download list, so a model in use by either must
      // not be deletable from here.
      protectedFromDelete={
        model.id === assistantModelId || model.id === cleanupModelId
      }
      onUse={(modelId, target) => void useFor(modelId, target)}
      onDownload={(modelId) => void handleDownload(modelId)}
      onDelete={(modelId) => void handleDelete(modelId)}
      onCancel={cancelDownload}
      downloadProgress={downloadProgress[model.id]?.percentage}
      downloadSpeed={downloadStats[model.id]?.speed}
    />
  );

  const hardwareLabel = hardware
    ? hardware.acceleratorName && hardware.acceleratorMemoryGb
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
          ? t("settings.assistant.brain.acceleratorUnknownWithMemory", {
              memory: hardware.systemMemoryGb,
            })
          : t("settings.assistant.brain.acceleratorUnknown")
    : null;

  return (
    <div className="space-y-8">
      <section aria-labelledby={`recommended-${role}`}>
        <ListHeading
          id={`recommended-${role}`}
          title={t("catalog.recommended")}
          info={t(
            isCleanup
              ? "settings.dictation.aiCleanup.catalog.recommendedDescription"
              : "settings.assistant.brain.recommendedDescription",
          )}
          trailing={
            hardwareLabel && (
              <span className="inline-flex items-center gap-1.5 text-xs text-muted tabular-nums">
                <MemoryStick className="h-3.5 w-3.5" aria-hidden="true" />
                {hardwareLabel}
              </span>
            )
          }
        />
        {recommendedModels.length === 0 ? (
          <p className="rounded-2xl border border-dashed border-hairline-strong px-4 py-5 text-center text-sm text-muted">
            {t("settings.assistant.brain.catalogEmpty")}
          </p>
        ) : (
          <div className="divide-y divide-hairline rounded-2xl border border-hairline bg-surface elev-card">
            {recommendedModels.map(({ model, meta }) =>
              renderModel(model, meta.isRecommended),
            )}
          </div>
        )}
      </section>

      {savedModels.length > 0 && (
        <section aria-labelledby={`saved-${role}`}>
          <ListHeading
            id={`saved-${role}`}
            title={t("settings.assistant.brain.huggingFaceTitle")}
            info={t("catalog.yoursInfo")}
          />
          <div className="divide-y divide-hairline rounded-2xl border border-hairline bg-surface elev-card">
            {savedModels.map((model) => renderModel(model))}
          </div>
        </section>
      )}

      <section
        aria-label={t("settings.assistant.brain.finderSectionTitle")}
        className="flex flex-wrap items-center gap-2"
      >
        <Button variant="secondary" onClick={() => setAddDialogOpen(true)}>
          <Search className="h-4 w-4" aria-hidden="true" />
          {t("catalog.searchHuggingFace")}
        </Button>
        <Button variant="secondary" onClick={() => setLocalDialogOpen(true)}>
          <HardDrive className="h-4 w-4" aria-hidden="true" />
          {t("modelsHub.addLocal")}
        </Button>
      </section>

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
