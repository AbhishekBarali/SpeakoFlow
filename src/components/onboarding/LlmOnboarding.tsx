import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type ModelInfo } from "@/bindings";
import { formatModelSize } from "@/lib/utils/format";
import {
  getTranslatedModelDescription,
  getTranslatedModelName,
} from "@/lib/utils/modelTranslation";
import { useSettings } from "@/hooks/useSettings";
import { GemmaLogo } from "../icons/BrandLogos";
import type { WelcomeCardPhase } from "./WelcomeChoiceCard";
import WelcomeChoiceCard from "./WelcomeChoiceCard";
import OnboardingLayout from "./OnboardingLayout";
import { Button } from "../ui/Button";
import { useModelStore } from "../../stores/modelStore";

/** The built-in (local) llama.cpp provider id, mirrored from the backend. */
const BUILTIN_PROVIDER_ID = "builtin";

/** Three current Gemma 4 choices, ordered by response/quality tradeoff. */
const TIERS = {
  /** Quickest current option; lower capability on complex requests. */
  quick: "gemma-4-e2b",
  /** Recommended conversational quality/latency balance. */
  balanced: "gemma-4-e4b",
  /** More capable, but noticeably slower. */
  capable: "gemma-4-12b",
} as const;

type Tier = keyof typeof TIERS;

const TIER_ORDER: Tier[] = ["quick", "balanced", "capable"];

const GEMMA_TILE = "bg-[#4285f4] text-white shadow-sm";

const TIER_META: Record<
  Tier,
  {
    icon: React.ReactNode;
    tileClassName: string;
    pillKey: "seesScreen";
    isRecommended: boolean;
  }
> = {
  quick: {
    icon: <GemmaLogo size={19} />,
    tileClassName: GEMMA_TILE,
    pillKey: "seesScreen",
    isRecommended: false,
  },
  balanced: {
    icon: <GemmaLogo size={19} />,
    tileClassName: GEMMA_TILE,
    pillKey: "seesScreen",
    isRecommended: true,
  },
  capable: {
    icon: <GemmaLogo size={19} />,
    tileClassName: GEMMA_TILE,
    pillKey: "seesScreen",
    isRecommended: false,
  },
};

interface LlmOnboardingProps {
  /** Advance to the next step (whether a model was chosen or skipped). */
  onComplete: () => void;
}

/**
 * Step 2 of the welcome flow: "Give it a brain (optional)."
 *
 * Three current Gemma 4 cards expose the real tradeoff: E2B responds
 * quickest, E4B is the recommended conversational balance, and 12B is more
 * capable but slower. The badge is editorial and never inferred from RAM or
 * VRAM. Tapping a card morphs it into a progress state in place and flips the
 * footer to "Continue" immediately: the download finishes in the background
 * and wires the built-in assistant provider to the model when it lands. "Skip
 * for now" always stays, reassuring the user they can add this later.
 */
const LlmOnboarding: React.FC<LlmOnboardingProps> = ({ onComplete }) => {
  const { t } = useTranslation();
  const { settings, refreshSettings } = useSettings();
  const {
    models,
    downloadModel,
    cancelDownload,
    downloadingModels,
    verifyingModels,
    extractingModels,
    downloadProgress,
  } = useModelStore();
  const [chosenId, setChosenId] = useState<string | null>(null);

  const hasChosen = chosenId !== null;

  const phaseFor = (modelId: string): WelcomeCardPhase => {
    if (modelId in extractingModels) return "extracting";
    if (modelId in verifyingModels) return "verifying";
    if (modelId in downloadingModels) return "downloading";
    const m = models.find((x) => x.id === modelId);
    if (m?.is_downloaded) return "done";
    return "idle";
  };

  // Choosing a model: download it first, and only point the built-in (local)
  // assistant provider at it once the weights are actually on disk. Doing it in
  // this order means a failed/cancelled download can't leave the assistant
  // "set" to a model that was never downloaded. The await survives this
  // component unmounting (the user pressing Continue), so the provider still
  // wires up when the background download completes.
  const handleChoose = async (modelId: string) => {
    setChosenId(modelId);

    const wireUpProvider = async () => {
      try {
        await commands.changeAssistantModelSetting(
          BUILTIN_PROVIDER_ID,
          modelId,
        );
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

    // No toast here: the card morphs into its progress state in place, and a
    // toast would cover the footer's Continue button.
    // Download errors surface via the central model-download-failed listener.
    const success = await downloadModel(modelId);
    if (success) {
      await wireUpProvider();
    } else {
      setChosenId(null);
    }
  };

  const handleCancelChosen = async () => {
    if (!chosenId) return;
    const cancelled = await cancelDownload(chosenId);
    if (cancelled) setChosenId(null);
  };

  const chosenCanBeCancelled =
    chosenId !== null && chosenId in downloadingModels;

  const footer = (
    <>
      <p className="text-xs text-muted max-w-[55%]">
        {hasChosen
          ? t("onboarding.aiModel.downloadingHint")
          : t("onboarding.aiModel.skipHint")}
      </p>
      {hasChosen ? (
        <div className="flex items-center gap-2">
          {chosenCanBeCancelled && (
            <Button
              variant="ghost"
              size="lg"
              onClick={() => void handleCancelChosen()}
            >
              {t("modelSelector.cancelDownload")}
            </Button>
          )}
          <Button variant="primary" size="lg" onClick={onComplete}>
            {t("onboarding.aiModel.continue")}
          </Button>
        </div>
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
      totalSteps={3}
      title={t("onboarding.aiModel.title")}
      subtitle={t("onboarding.aiModel.subtitle")}
      footer={footer}
      showDownloadProgress={false}
    >
      {TIER_ORDER.map((tier) => {
        const model = models.find((m: ModelInfo) => m.id === TIERS[tier]);
        if (!model) return null;
        const meta = TIER_META[tier];
        const cleanName = getTranslatedModelName(model, t).replace(
          /\s*\(vision\)\s*$/i,
          "",
        );
        return (
          <WelcomeChoiceCard
            key={tier}
            icon={meta.icon}
            tileClassName={meta.tileClassName}
            title={cleanName}
            description={getTranslatedModelDescription(model, t)}
            sizeLabel={formatModelSize(Number(model.size_mb))}
            pill={t(`onboarding.aiModel.${meta.pillKey}`)}
            badge={meta.isRecommended ? t("onboarding.recommended") : undefined}
            selected={chosenId === model.id}
            disabled={hasChosen}
            phase={phaseFor(model.id)}
            progress={downloadProgress[model.id]?.percentage}
            actionLabel={
              model.is_downloaded
                ? t("onboarding.aiModel.useDownloaded")
                : t("onboarding.aiModel.download")
            }
            onClick={() => {
              if (!hasChosen) void handleChoose(model.id);
            }}
          />
        );
      })}
    </OnboardingLayout>
  );
};

export default LlmOnboarding;
