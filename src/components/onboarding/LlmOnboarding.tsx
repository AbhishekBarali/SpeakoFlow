import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type ModelInfo } from "@/bindings";
import { formatModelSize } from "@/lib/utils/format";
import { getTranslatedModelName } from "@/lib/utils/modelTranslation";
import { useSettings } from "@/hooks/useSettings";
import { GemmaLogo, QwenLogo } from "../icons/BrandLogos";
import type { WelcomeCardPhase } from "./WelcomeChoiceCard";
import WelcomeChoiceCard from "./WelcomeChoiceCard";
import OnboardingLayout from "./OnboardingLayout";
import { Button } from "../ui/Button";
import { useModelStore } from "../../stores/modelStore";

/** The built-in (local) llama.cpp provider id, mirrored from the backend. */
const BUILTIN_PROVIDER_ID = "builtin";

/** Three curated local-model tiers, by catalog id. */
const TIERS = {
  /** Tiny, text-only — runs on any machine. */
  small: "gemma-3-1b",
  /** Quick all-rounder that also sees the screen. */
  mid: "qwen3.5-2b",
  /** Strongest small model, also sees the screen. */
  capable: "qwen3.5-4b",
} as const;

type Tier = keyof typeof TIERS;

const TIER_ORDER: Tier[] = ["small", "mid", "capable"];

/** Real brand colors for the logo tiles — Gemma sits on Google blue, Qwen on
 *  its purple — so the marks read as the brands they are, consistently. */
const BRAND_TILE = {
  gemma: "bg-gradient-to-br from-[#5b9bf8] to-[#3367d6] text-white shadow-sm",
  qwen: "bg-gradient-to-br from-[#8b7bff] to-[#5546d6] text-white shadow-sm",
} as const;

const TIER_META: Record<
  Tier,
  {
    icon: React.ReactNode;
    tileClassName: string;
    descKey: string;
    pillKey: "seesScreen" | "textOnly";
  }
> = {
  small: {
    icon: <GemmaLogo size={19} />,
    tileClassName: BRAND_TILE.gemma,
    descKey: "onboarding.aiModel.tiers.small.description",
    pillKey: "textOnly",
  },
  mid: {
    icon: <QwenLogo size={19} />,
    tileClassName: BRAND_TILE.qwen,
    descKey: "onboarding.aiModel.tiers.mid.description",
    pillKey: "seesScreen",
  },
  capable: {
    icon: <QwenLogo size={19} />,
    tileClassName: BRAND_TILE.qwen,
    descKey: "onboarding.aiModel.tiers.capable.description",
    pillKey: "seesScreen",
  },
};

interface LlmOnboardingProps {
  /** Advance to the next step (whether a model was chosen or skipped). */
  onComplete: () => void;
}

/**
 * Step 2 of the welcome flow: "Give it a brain (optional)."
 *
 * Three RAM-tiered local-model cards — a tiny text-only pick, a balanced pick,
 * and the most capable — with exactly one "Recommended" badge placed on the tier
 * that best fits this machine's memory (via `get_system_memory_gb`). Tapping a
 * card morphs it into a progress state in place and flips the footer to
 * "Continue" immediately: the download finishes in the background and wires the
 * built-in assistant provider to the model when it lands. "Skip for now" always
 * stays, reassuring the user they can add this later.
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
  } = useModelStore();
  const [chosenId, setChosenId] = useState<string | null>(null);
  const [ramGb, setRamGb] = useState<number | null>(null);

  // Physical RAM (whole GiB, 0 when unknown) drives which single tier is
  // recommended for this machine.
  useEffect(() => {
    commands
      .getSystemMemoryGb()
      .then((gb) => setRamGb(gb))
      .catch(() => setRamGb(0));
  }, []);

  const recommendedTier: Tier = useMemo(() => {
    const gb = ramGb ?? 0;
    if (gb > 0 && gb <= 8) return "small";
    if (gb >= 16) return "capable";
    return "mid";
  }, [ramGb]);

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

  const footer = (
    <>
      <p className="text-xs text-muted-soft max-w-[55%]">
        {hasChosen
          ? t("onboarding.aiModel.downloadingHint")
          : t("onboarding.aiModel.skipHint")}
      </p>
      {hasChosen ? (
        <Button variant="primary" size="lg" onClick={onComplete}>
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
            description={t(meta.descKey)}
            sizeLabel={formatModelSize(Number(model.size_mb))}
            pill={t(`onboarding.aiModel.${meta.pillKey}`)}
            badge={
              tier === recommendedTier ? t("onboarding.recommended") : undefined
            }
            selected={chosenId === model.id}
            disabled={hasChosen && chosenId !== model.id}
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
