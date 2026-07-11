import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { ModelInfo } from "@/bindings";
import { getModelCategory } from "@/lib/utils/modelCategory";
import type { ModelCardStatus } from "./ModelCard";
import ModelCard from "./ModelCard";
import OnboardingLayout from "./OnboardingLayout";
import { Button } from "../ui/Button";
import { useModelStore } from "../../stores/modelStore";

interface OnboardingProps {
  onModelSelected: () => void;
}

/**
 * Step 1 of the first-run flow: choose a speech-to-text (transcription) model.
 * Only transcription engines are shown here — the AI/LLM models live in their
 * own step (see LlmOnboarding) so the two are never mixed into one flat list.
 */
const Onboarding: React.FC<OnboardingProps> = ({ onModelSelected }) => {
  const { t } = useTranslation();
  const {
    models,
    downloadModel,
    selectModel,
    downloadingModels,
    verifyingModels,
    extractingModels,
    downloadProgress,
    downloadStats,
  } = useModelStore();
  const [selectedModelId, setSelectedModelId] = useState<string | null>(null);
  // Guards against firing `selectModel` more than once for the same model. The
  // watcher effect below re-runs on every store change (models, download maps,
  // …), and because `selectModel` is async those re-runs could otherwise kick
  // off several concurrent selects for one model — which is what produced a
  // stack of duplicate error toasts when one call raced ahead of the others.
  const attemptedSelectRef = useRef<string | null>(null);

  const isDownloading = selectedModelId !== null;

  // Only speech-to-text (transcription) models belong in this step.
  const sttModels = useMemo(
    () => models.filter((m: ModelInfo) => getModelCategory(m) === "stt"),
    [models],
  );

  // Recommended models to feature at the top, ordered by their recommendation
  // rank (1 = the default) so the English streaming default leads and the
  // multilingual streaming option (Nemotron, rank 2) sits right below it.
  // Unranked recommended models fall to the end, most-accurate first.
  const featuredModels = useMemo(
    () =>
      sttModels
        .filter((m: ModelInfo) => !m.is_downloaded && m.is_recommended)
        .sort((a: ModelInfo, b: ModelInfo) => {
          const rank = (m: ModelInfo) => m.recommended_rank ?? Number.MAX_SAFE_INTEGER;
          const byRank = rank(a) - rank(b);
          if (byRank !== 0) return byRank;
          return Number(b.accuracy_score) - Number(a.accuracy_score);
        }),
    [sttModels],
  );

  const otherModels = useMemo(
    () =>
      sttModels
        .filter((m: ModelInfo) => !m.is_downloaded && !m.is_recommended)
        .sort(
          (a: ModelInfo, b: ModelInfo) => Number(a.size_mb) - Number(b.size_mb),
        ),
    [sttModels],
  );

  // An already-downloaded speech-to-text model (if any). Lets the user skip the
  // download step and jump straight in — handy when testing, since the models
  // are otherwise re-downloaded on every fresh run.
  const installedModel = useMemo(
    () => sttModels.find((m: ModelInfo) => m.is_downloaded) ?? null,
    [sttModels],
  );

  const handleUseInstalled = () => {
    if (!installedModel) return;
    selectModel(installedModel.id).then((success) => {
      if (success) {
        onModelSelected();
      } else {
        toast.error(t("onboarding.errors.selectModel"), {
          id: "onboarding-select-model",
        });
      }
    });
  };

  // Watch for the selected model to finish downloading + verifying + extracting
  useEffect(() => {
    if (!selectedModelId) return;

    const model = models.find((m) => m.id === selectedModelId);
    const stillDownloading = selectedModelId in downloadingModels;
    const stillVerifying = selectedModelId in verifyingModels;
    const stillExtracting = selectedModelId in extractingModels;

    if (
      model?.is_downloaded &&
      !stillDownloading &&
      !stillVerifying &&
      !stillExtracting
    ) {
      // Only attempt the select once per model, even if the effect re-runs
      // while the async call is still in flight.
      if (attemptedSelectRef.current === selectedModelId) return;
      attemptedSelectRef.current = selectedModelId;

      // Model is ready — select it and transition
      selectModel(selectedModelId).then((success) => {
        if (success) {
          onModelSelected();
        } else {
          toast.error(t("onboarding.errors.selectModel"), {
            id: "onboarding-select-model",
          });
          setSelectedModelId(null);
          attemptedSelectRef.current = null;
        }
      });
    }
  }, [
    selectedModelId,
    models,
    downloadingModels,
    verifyingModels,
    extractingModels,
    selectModel,
    onModelSelected,
    t,
  ]);

  const handleDownloadModel = async (modelId: string) => {
    setSelectedModelId(modelId);

    // Error toast is handled centrally by the model-download-failed event listener
    // in modelStore — no toast here to avoid duplicates.
    const success = await downloadModel(modelId);
    if (!success) {
      setSelectedModelId(null);
    }
  };

  const getModelStatus = (modelId: string): ModelCardStatus => {
    if (modelId in extractingModels) return "extracting";
    if (modelId in verifyingModels) return "verifying";
    if (modelId in downloadingModels) return "downloading";
    return "downloadable";
  };

  const getModelDownloadProgress = (modelId: string): number | undefined => {
    return downloadProgress[modelId]?.percentage;
  };

  const getModelDownloadSpeed = (modelId: string): number | undefined => {
    return downloadStats[modelId]?.speed;
  };

  return (
    <OnboardingLayout
      step={1}
      totalSteps={2}
      title={t("onboarding.speechToText.title")}
      subtitle={t("onboarding.speechToText.subtitle")}
      footer={
        installedModel ? (
          <>
            <p className="text-xs text-muted-soft max-w-[60%]">
              {t("onboarding.speechToText.installedHint")}
            </p>
            <Button
              variant="secondary"
              size="lg"
              onClick={handleUseInstalled}
              disabled={isDownloading}
            >
              {t("onboarding.speechToText.useInstalled")}
            </Button>
          </>
        ) : undefined
      }
    >
      {featuredModels.map((model: ModelInfo) => (
        <ModelCard
          key={model.id}
          model={model}
          variant="featured"
          status={getModelStatus(model.id)}
          disabled={isDownloading}
          onSelect={handleDownloadModel}
          onDownload={handleDownloadModel}
          downloadProgress={getModelDownloadProgress(model.id)}
          downloadSpeed={getModelDownloadSpeed(model.id)}
          showScores={false}
        />
      ))}

      {otherModels.map((model: ModelInfo) => (
        <ModelCard
          key={model.id}
          model={model}
          status={getModelStatus(model.id)}
          disabled={isDownloading}
          onSelect={handleDownloadModel}
          onDownload={handleDownloadModel}
          downloadProgress={getModelDownloadProgress(model.id)}
          downloadSpeed={getModelDownloadSpeed(model.id)}
          showScores={false}
        />
      ))}
    </OnboardingLayout>
  );
};

export default Onboarding;
