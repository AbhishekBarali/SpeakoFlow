import React, { useMemo, useState } from "react";
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
 *
 * The download is NON-BLOCKING (mirrors the AI-model step): picking a model
 * kicks off a background download, shows progress in the shared DownloadProgress
 * strip, and lets the user press Continue right away. The store selects the
 * model (makes it the active recording model) once its weights land on disk —
 * even if the user has already moved on — retrying to absorb the brief
 * post-download engine-load race instead of spamming an error toast.
 */
const Onboarding: React.FC<OnboardingProps> = ({ onModelSelected }) => {
  const { t } = useTranslation();
  const {
    models,
    downloadModel,
    downloadingModels,
    verifyingModels,
    extractingModels,
    downloadProgress,
    downloadStats,
    setPendingSttSelection,
    finalizePendingSttSelection,
  } = useModelStore();
  const [selectedModelId, setSelectedModelId] = useState<string | null>(null);

  const hasChosen = selectedModelId !== null;

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
          const rank = (m: ModelInfo) =>
            m.recommended_rank ?? Number.MAX_SAFE_INTEGER;
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

  // Use a model that's already on disk: select it (robustly, via the store) and
  // continue immediately — the selection finishes in the background.
  const handleUseInstalled = () => {
    if (!installedModel) return;
    setPendingSttSelection(installedModel.id);
    void finalizePendingSttSelection(installedModel.id);
    onModelSelected();
  };

  const handleDownloadModel = async (modelId: string) => {
    setSelectedModelId(modelId);
    // Remember this as the STT model to activate once the download finishes.
    // The store owns the completion → select flow, so it survives the user
    // pressing Continue and leaving this step.
    setPendingSttSelection(modelId);

    // Kick off the background download. Real download errors surface centrally
    // via the model-download-failed listener in the store (Handy #1522), so no
    // toast here. On an immediate failure, reset so the user can pick again.
    const success = await downloadModel(modelId);
    if (!success) {
      setSelectedModelId(null);
      setPendingSttSelection(null);
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

  // Footer mirrors the AI-model step: once a model is chosen the download runs
  // in the background and the user can Continue; otherwise they can jump in with
  // an already-installed model. Never blocks.
  const footer = hasChosen ? (
    <>
      <p className="text-xs text-muted-soft max-w-[60%]">
        {t("onboarding.speechToText.downloadingHint")}
      </p>
      <Button
        variant="primary"
        size="lg"
        onClick={() => {
          toast.success(t("onboarding.speechToText.downloadStarted"));
          onModelSelected();
        }}
      >
        {t("onboarding.speechToText.continue")}
      </Button>
    </>
  ) : installedModel ? (
    <>
      <p className="text-xs text-muted-soft max-w-[60%]">
        {t("onboarding.speechToText.installedHint")}
      </p>
      <Button variant="secondary" size="lg" onClick={handleUseInstalled}>
        {t("onboarding.speechToText.useInstalled")}
      </Button>
    </>
  ) : undefined;

  return (
    <OnboardingLayout
      step={1}
      totalSteps={2}
      title={t("onboarding.speechToText.title")}
      subtitle={t("onboarding.speechToText.subtitle")}
      footer={footer}
    >
      {featuredModels.map((model: ModelInfo) => (
        <ModelCard
          key={model.id}
          model={model}
          variant="featured"
          status={getModelStatus(model.id)}
          disabled={hasChosen && selectedModelId !== model.id}
          onSelect={handleDownloadModel}
          onDownload={handleDownloadModel}
          downloadProgress={getModelDownloadProgress(model.id)}
          downloadSpeed={getModelDownloadSpeed(model.id)}
          showScores={false}
          showInlineProgress={false}
        />
      ))}

      {otherModels.map((model: ModelInfo) => (
        <ModelCard
          key={model.id}
          model={model}
          status={getModelStatus(model.id)}
          disabled={hasChosen && selectedModelId !== model.id}
          onSelect={handleDownloadModel}
          onDownload={handleDownloadModel}
          downloadProgress={getModelDownloadProgress(model.id)}
          downloadSpeed={getModelDownloadSpeed(model.id)}
          showScores={false}
          showInlineProgress={false}
        />
      ))}
    </OnboardingLayout>
  );
};

export default Onboarding;
