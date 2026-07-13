import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown } from "lucide-react";
import type { ModelInfo } from "@/bindings";
import { NvidiaLogo } from "../icons/BrandLogos";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { formatModelSize } from "@/lib/utils/format";
import { getTranslatedModelName } from "@/lib/utils/modelTranslation";
import type { ModelCardStatus } from "./ModelCard";
import ModelCard from "./ModelCard";
import OnboardingLayout from "./OnboardingLayout";
import WelcomeChoiceCard from "./WelcomeChoiceCard";
import { Button } from "../ui/Button";
import { useModelStore } from "../../stores/modelStore";

interface OnboardingProps {
  onModelSelected: () => void;
}

/** The two featured speech-to-text options, by runtime model id (the backend
 *  exposes catalog models as `<slug>-gguf`). */
const FEATURED = {
  /** English, streaming, blazingly fast. */
  fast: "parakeet-unified-en-0.6b-gguf",
  /** Real-time, 28 languages. */
  multilingual: "nemotron-3.5-asr-streaming-0.6b-gguf",
} as const;

/**
 * Step 1 of the welcome flow: "How should SpeakoFlow hear you?"
 *
 * Two featured cards do the choosing for a first-timer — a fast English option
 * and a multilingual one — pre-selected by the machine's language. "Download and
 * continue" kicks the download off in the BACKGROUND and moves straight to the
 * next step; the store selects the model once its weights land (even after the
 * user has moved on), so nobody watches a progress bar. A quiet "See all models"
 * disclosure opens the full catalog for enthusiasts, and an already-installed
 * model keeps its one-tap fast path.
 */
const Onboarding: React.FC<OnboardingProps> = ({ onModelSelected }) => {
  const { t, i18n } = useTranslation();
  const {
    models,
    downloadModel,
    downloadingModels,
    verifyingModels,
    extractingModels,
    setPendingSttSelection,
    finalizePendingSttSelection,
  } = useModelStore();
  const [selectedModelId, setSelectedModelId] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(false);

  // Only speech-to-text (transcription) models belong in this step.
  const sttModels = useMemo(
    () => models.filter((m: ModelInfo) => getModelCategory(m) === "stt"),
    [models],
  );

  const fastModel = useMemo(
    () => sttModels.find((m) => m.id === FEATURED.fast) ?? null,
    [sttModels],
  );
  const multiModel = useMemo(
    () => sttModels.find((m) => m.id === FEATURED.multilingual) ?? null,
    [sttModels],
  );

  // Everything that isn't already featured, for the "See all models" fold.
  const catalogModels = useMemo(
    () =>
      sttModels
        .filter((m) => m.id !== FEATURED.fast && m.id !== FEATURED.multilingual)
        .sort((a, b) => {
          const rank = (m: ModelInfo) =>
            m.recommended_rank ?? Number.MAX_SAFE_INTEGER;
          const byRank = rank(a) - rank(b);
          if (byRank !== 0) return byRank;
          return Number(a.size_mb) - Number(b.size_mb);
        }),
    [sttModels],
  );

  // An already-downloaded speech-to-text model (if any) — the fast path that
  // lets a returning/testing user skip the download entirely.
  const installedModel = useMemo(
    () => sttModels.find((m: ModelInfo) => m.is_downloaded) ?? null,
    [sttModels],
  );

  // Pre-select by the machine's language: English speakers get the fast English
  // model, everyone else the multilingual one. Runs once models are available.
  useEffect(() => {
    if (selectedModelId) return;
    const langs = [
      navigator.language,
      ...(navigator.languages ?? []),
      i18n.language,
    ]
      .filter(Boolean)
      .map((l) => l.toLowerCase());
    const prefersEnglish = langs.some((l) => l.startsWith("en"));
    const primary = prefersEnglish ? fastModel : multiModel;
    const fallback = prefersEnglish ? multiModel : fastModel;
    const chosen = primary ?? fallback ?? sttModels[0] ?? null;
    if (chosen) setSelectedModelId(chosen.id);
  }, [selectedModelId, fastModel, multiModel, sttModels, i18n.language]);

  const getModelStatus = (modelId: string): ModelCardStatus => {
    if (modelId in extractingModels) return "extracting";
    if (modelId in verifyingModels) return "verifying";
    if (modelId in downloadingModels) return "downloading";
    const m = models.find((x) => x.id === modelId);
    if (m?.is_downloaded) return "available";
    return "downloadable";
  };

  // Choose a model and continue immediately. The download (if any) runs in the
  // background; the store owns the completion → select handoff via the pending
  // STT selection, so leaving this step doesn't interrupt it. Real download
  // errors surface centrally via the model-download-failed listener.
  const handleChoose = (modelId: string) => {
    const model = models.find((m) => m.id === modelId);
    setPendingSttSelection(modelId);
    if (model?.is_downloaded) {
      void finalizePendingSttSelection(modelId);
    } else {
      // Kick the download off in the background — the card and the in-app
      // status widget carry the progress story; no toast (it covered the
      // footer actions).
      void downloadModel(modelId);
    }
    onModelSelected();
  };

  // Use a model that's already on disk: select it (robustly, via the store) and
  // continue immediately — the selection finishes in the background.
  const handleUseInstalled = () => {
    if (!installedModel) return;
    setPendingSttSelection(installedModel.id);
    void finalizePendingSttSelection(installedModel.id);
    onModelSelected();
  };

  // Whether the currently selected card is already on disk — the primary
  // button then reads "Continue" instead of promising a download.
  const selectedIsInstalled =
    !!selectedModelId &&
    (models.find((m) => m.id === selectedModelId)?.is_downloaded ?? false);

  const footer = (
    <>
      <p className="text-xs text-muted-soft max-w-[55%]">
        {installedModel
          ? t("onboarding.speechToText.installedHint")
          : t("onboarding.speechToText.downloadingHint")}
      </p>
      <div className="flex items-center gap-2 shrink-0">
        {installedModel && !selectedIsInstalled && (
          <Button variant="secondary" size="lg" onClick={handleUseInstalled}>
            {t("onboarding.speechToText.useInstalled")}
          </Button>
        )}
        <Button
          variant="primary"
          size="lg"
          disabled={!selectedModelId}
          onClick={() => selectedModelId && handleChoose(selectedModelId)}
        >
          {selectedIsInstalled
            ? t("onboarding.speechToText.continue")
            : t("onboarding.speechToText.downloadAndContinue")}
        </Button>
      </div>
    </>
  );

  return (
    <OnboardingLayout
      step={1}
      totalSteps={3}
      title={t("onboarding.speechToText.title")}
      subtitle={t("onboarding.speechToText.subtitle")}
      footer={footer}
    >
      {fastModel && (
        <WelcomeChoiceCard
          icon={<NvidiaLogo size={20} />}
          tone="emerald"
          title={getTranslatedModelName(fastModel, t)}
          description={t("onboarding.speechToText.cards.fast.description")}
          sizeLabel={formatModelSize(Number(fastModel.size_mb))}
          selected={selectedModelId === fastModel.id}
          onClick={() => setSelectedModelId(fastModel.id)}
        />
      )}
      {multiModel && (
        <WelcomeChoiceCard
          icon={<NvidiaLogo size={20} />}
          tone="emerald"
          title={getTranslatedModelName(multiModel, t)}
          description={t(
            "onboarding.speechToText.cards.multilingual.description",
          )}
          sizeLabel={formatModelSize(Number(multiModel.size_mb))}
          selected={selectedModelId === multiModel.id}
          onClick={() => setSelectedModelId(multiModel.id)}
        />
      )}

      {/* Quiet disclosure — the full catalog for enthusiasts. Jargon (quant
          badges, sizes, streaming tags) is allowed in here, unlike the two
          featured cards above. */}
      <div className="pt-1">
        <button
          type="button"
          aria-expanded={showAll}
          onClick={() => setShowAll((o) => !o)}
          className="w-full flex items-center justify-center gap-1.5 px-4 py-2 text-xs font-medium text-muted hover:text-ink transition-colors cursor-pointer"
        >
          <span>
            {showAll
              ? t("onboarding.speechToText.hideAllModels")
              : t("onboarding.speechToText.seeAllModels")}
          </span>
          <ChevronDown
            className={`w-3.5 h-3.5 transition-transform duration-200 ${showAll ? "rotate-180" : ""}`}
          />
        </button>

        {showAll && (
          <div className="flex flex-col gap-3 pt-1">
            {catalogModels.map((model: ModelInfo) => (
              <ModelCard
                key={model.id}
                model={model}
                status={getModelStatus(model.id)}
                className={
                  selectedModelId === model.id
                    ? "ring-1 ring-accent/40 border-accent/50"
                    : ""
                }
                onSelect={() => setSelectedModelId(model.id)}
                onDownload={() => setSelectedModelId(model.id)}
                showRecommended={false}
                showScores={false}
                showInlineProgress={false}
              />
            ))}
          </div>
        )}
      </div>
    </OnboardingLayout>
  );
};

export default Onboarding;
