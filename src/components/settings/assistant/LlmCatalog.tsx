import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus } from "lucide-react";
import { commands, type ModelInfo } from "@/bindings";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { useModelStore } from "@/stores/modelStore";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "@/components/ui/Button";
import { AddCustomModelDialog } from "../models/AddCustomModelDialog";
import ModelCard, { type ModelCardStatus } from "../../onboarding/ModelCard";

/** The built-in (local) llama.cpp provider id, mirrored from the backend. */
const BUILTIN_PROVIDER_ID = "builtin";

/**
 * The on-device assistant model catalog, rendered as an Assistant sub-page
 * (opened from the brain picker's "Download a model…" row). It lists the local
 * language models: download, delete, or pick one, and picking wires it to the
 * built-in (local) provider — the same flow the first-run wizard uses.
 *
 * This is one of the few places jargon (quantization, size) is allowed to show,
 * because it's the "advanced catalog" the Voice Guide carves out. It reuses the
 * shared `ModelCard` so a model reads the same here as in onboarding.
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

  // Only local language models (llama.cpp / GGUF) belong here. Recommended
  // first, then smallest-to-largest so lighter machines see approachable
  // options at the top.
  const llmModels = useMemo(
    () =>
      models
        .filter((m: ModelInfo) => getModelCategory(m) === "llm")
        .sort((a: ModelInfo, b: ModelInfo) => {
          if (a.is_recommended !== b.is_recommended)
            return a.is_recommended ? -1 : 1;
          return Number(a.size_mb) - Number(b.size_mb);
        }),
    [models],
  );

  // Point the built-in provider at a model and make sure the provider itself is
  // selected. Mirrors the onboarding wire-up so a model chosen here is live
  // immediately. Only runs once the weights are on disk.
  const wireUpProvider = async (modelId: string) => {
    await commands.changeAssistantModelSetting(BUILTIN_PROVIDER_ID, modelId);
    if (!providerIsBuiltin) {
      await commands.setAssistantProvider(BUILTIN_PROVIDER_ID);
    }
    await refreshSettings();
  };

  // Download errors surface via the central model-download-failed listener in
  // the model store; a model already on disk is wired up immediately.
  const handleDownload = async (modelId: string) => {
    const model = models.find((m: ModelInfo) => m.id === modelId);
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

  return (
    <div className="space-y-2.5">
      {/* Bring-your-own-model: search Hugging Face for any GGUF and add it. */}
      <div className="flex justify-end">
        <Button
          variant="secondary"
          size="sm"
          onClick={() => setAddDialogOpen(true)}
        >
          <Plus className="w-4 h-4" />
          {t("settings.models.customModel.addButton")}
        </Button>
      </div>
      {llmModels.length === 0 ? (
        <p className="text-sm text-muted leading-relaxed">
          {t("settings.assistant.brain.catalogEmpty")}
        </p>
      ) : (
        llmModels.map((model) => (
          <ModelCard
            key={model.id}
            model={model}
            status={statusFor(model)}
            onSelect={handleSelect}
            onDownload={handleDownload}
            onDelete={handleDelete}
            onCancel={cancelDownload}
            downloadProgress={downloadProgress[model.id]?.percentage}
            downloadSpeed={downloadStats[model.id]?.speed}
            showScores={false}
          />
        ))
      )}
      <AddCustomModelDialog
        open={addDialogOpen}
        onClose={() => setAddDialogOpen(false)}
      />
    </div>
  );
};

export default LlmCatalog;
