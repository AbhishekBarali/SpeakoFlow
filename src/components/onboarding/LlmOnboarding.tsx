import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type ModelInfo } from "@/bindings";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { useSettings } from "@/hooks/useSettings";
import type { ModelCardStatus } from "./ModelCard";
import ModelCard from "./ModelCard";
import OnboardingLayout from "./OnboardingLayout";
import { Button } from "../ui/Button";
import { useModelStore } from "../../stores/modelStore";

/** The built-in (local) llama.cpp provider id, mirrored from the backend. */
const BUILTIN_PROVIDER_ID = "builtin";

interface LlmOnboardingProps {
  /** Advance to the main app (whether a model was chosen or skipped). */
  onComplete: () => void;
}

/**
 * Step 2 of the first-run flow: optionally choose a local AI model (LLM) for the
 * assistant and transcript cleanup. Only local `LlamaCpp` models are shown here.
 *
 * This step is skippable: the assistant also works with cloud providers, and the
 * local models are large multi-gigabyte downloads. Choosing one wires it to the
 * built-in (local) provider and starts a background download, so the user can
 * continue into the app immediately rather than waiting for it to finish.
 */
const LlmOnboarding: React.FC<LlmOnboardingProps> = ({ onComplete }) => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();
  const {
    models,
    downloadModel,
    downloadingModels,
    verifyingModels,
    extractingModels,
    downloadProgress,
    downloadStats,
  } = useModelStore();
  const [selectedModelId, setSelectedModelId] = useState<string | null>(null);

  // Only local language models (llama.cpp / GGUF) belong in this step.
  const llmModels = useMemo(
    () => models.filter((m: ModelInfo) => getModelCategory(m) === "llm"),
    [models],
  );

  const featuredModels = useMemo(
    () => llmModels.filter((m: ModelInfo) => m.is_recommended),
    [llmModels],
  );

  const otherModels = useMemo(
    () =>
      llmModels
        .filter((m: ModelInfo) => !m.is_recommended)
        .sort(
          (a: ModelInfo, b: ModelInfo) => Number(a.size_mb) - Number(b.size_mb),
        ),
    [llmModels],
  );

  const getModelStatus = (model: ModelInfo): ModelCardStatus => {
    if (model.id in extractingModels) return "extracting";
    if (model.id in verifyingModels) return "verifying";
    if (model.id in downloadingModels) return "downloading";
    if (model.is_downloaded) return "available";
    return "downloadable";
  };

  const getModelDownloadProgress = (modelId: string): number | undefined => {
    return downloadProgress[modelId]?.percentage;
  };

  const getModelDownloadSpeed = (modelId: string): number | undefined => {
    return downloadStats[modelId]?.speed;
  };

  // Choosing a model: download it first, and only point the built-in (local)
  // assistant provider at it once the weights are actually on disk. Doing it in
  // this order means a failed/cancelled download can't leave the assistant
  // "set" to a model that was never downloaded — the state that used to strand
  // the user on a non-downloadable "active" card in Settings → Models.
  const handleChooseModel = async (modelId: string) => {
    setSelectedModelId(modelId);

    const wireUpProvider = async () => {
      try {
        await commands.changeAssistantModelSetting(BUILTIN_PROVIDER_ID, modelId);
        if (settings?.assistant_provider_id !== BUILTIN_PROVIDER_ID) {
          await commands.setAssistantProvider(BUILTIN_PROVIDER_ID);
        }
        await refreshSettings();
      } catch (err) {
        console.error("Failed to set built-in assistant model:", err);
      }
    };

    const model = models.find((m: ModelInfo) => m.id === modelId);
    // Already on disk — nothing to download, wire it up immediately.
    if (model?.is_downloaded) {
      await wireUpProvider();
      return;
    }

    // Download errors surface via the central model-download-failed listener.
    const success = await downloadModel(modelId);
    if (success) {
      await wireUpProvider();
    } else {
      setSelectedModelId(null);
    }
  };

  const renderCard = (model: ModelInfo, featured: boolean) => (
    <ModelCard
      key={model.id}
      model={model}
      variant={featured ? "featured" : "default"}
      status={getModelStatus(model)}
      disabled={selectedModelId !== null && selectedModelId !== model.id}
      onSelect={handleChooseModel}
      onDownload={handleChooseModel}
      downloadProgress={getModelDownloadProgress(model.id)}
      downloadSpeed={getModelDownloadSpeed(model.id)}
      showScores={false}
    />
  );

  const hasChosen = selectedModelId !== null;

  const footer = (
    <>
      <p className="text-xs text-muted-soft max-w-[60%]">
        {hasChosen
          ? t("onboarding.aiModel.downloadingHint")
          : t("onboarding.aiModel.skipHint")}
      </p>
      {hasChosen ? (
        <Button
          variant="primary"
          size="lg"
          onClick={() => {
            toast.success(t("onboarding.aiModel.downloadStarted"));
            onComplete();
          }}
        >
          {t("onboarding.aiModel.continue")}
        </Button>
      ) : (
        <Button variant="secondary" size="lg" onClick={onComplete}>
          {t("onboarding.aiModel.skip")}
        </Button>
      )}
    </>
  );

  return (
    <OnboardingLayout
      step={2}
      totalSteps={2}
      title={t("onboarding.aiModel.title")}
      subtitle={t("onboarding.aiModel.subtitle")}
      footer={footer}
    >
      {featuredModels.map((model) => renderCard(model, true))}
      {otherModels.map((model) => renderCard(model, false))}
    </OnboardingLayout>
  );
};

export default LlmOnboarding;
