import React from "react";
import { useTranslation } from "react-i18next";
import { AudioLines, Radio } from "lucide-react";
import { useModelStore } from "@/stores/modelStore";
import { isLegacyModel } from "@/components/onboarding";
import { getModelBrand } from "@/components/icons/BrandLogos";
import {
  getTranslatedModelDescription,
  getTranslatedModelName,
} from "@/lib/utils/modelTranslation";
import { Button } from "@/components/ui/Button";
import Badge from "@/components/ui/Badge";
import type { ModelInfo } from "@/bindings";

interface DictationModelCardProps {
  /** Open the full model catalog (rendered as a sub-page by the parent). */
  onChangeModel: () => void;
}

/** A slim, quiet accuracy/speed meter — label and bar inline, sentence case.
 *  Mirrors the catalog ModelCard meter so the two read as the same idea. */
const ScoreMeter: React.FC<{ label: string; score: number }> = ({
  label,
  score,
}) => {
  const pct = Math.max(0, Math.min(100, Math.round(score * 100)));
  return (
    <div className="flex items-center gap-2">
      <span className="text-[11px] font-medium text-muted">{label}</span>
      <div
        className="h-1 w-20 shrink-0 overflow-hidden rounded-full bg-ink/15"
        role="meter"
        aria-label={label}
        aria-valuenow={pct}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div
          className="h-full rounded-full bg-ink/55"
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
};

/**
 * The hero of the Dictation page: the speech-to-text model that's currently
 * listening. Shows its friendly name, a one-line description, and quiet
 * accuracy/speed meters, with a single "Change model" button that opens the
 * full catalog. When no model is ready yet, it invites the user to pick one.
 *
 * Jargon (quant tags, file sizes, engine names) deliberately stays out of here
 * — that detail lives one level deeper, inside the catalog.
 */
export const DictationModelCard: React.FC<DictationModelCardProps> = ({
  onChangeModel,
}) => {
  const { t } = useTranslation();
  const { models, currentModel, loading } = useModelStore();

  const activeModel: ModelInfo | undefined = models.find(
    (m: ModelInfo) => m.id === currentModel,
  );
  const isReady = !!currentModel && !!activeModel && activeModel.is_downloaded;

  const cardClasses =
    "rounded-2xl border border-hairline bg-surface elev-card p-5";

  if (loading) {
    return (
      <div className={cardClasses}>
        <div className="flex items-center gap-3 text-[13px] text-muted">
          <div className="w-4 h-4 border-2 border-hairline-strong border-t-ink rounded-full animate-spin" />
          {t("settings.dictation.hero.loading")}
        </div>
      </div>
    );
  }

  // No model selected / installed yet — invite the user to pick one.
  if (!isReady) {
    return (
      <div className={cardClasses}>
        <div className="flex items-start justify-between gap-4">
          <div className="flex items-start gap-3 min-w-0">
            <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-accent/12 text-accent">
              <AudioLines className="h-[18px] w-[18px]" />
            </span>
            <div className="min-w-0">
              <p className="text-xs text-muted">
                {t("settings.dictation.hero.eyebrow")}
              </p>
              <h2 className="font-display text-lg leading-tight text-ink">
                {t("settings.dictation.hero.noModelTitle")}
              </h2>
              <p className="mt-1 text-[13px] leading-snug text-muted">
                {t("settings.dictation.hero.noModelCaption")}
              </p>
            </div>
          </div>
          <Button variant="primary" size="sm" onClick={onChangeModel}>
            {t("settings.dictation.hero.chooseModel")}
          </Button>
        </div>
      </div>
    );
  }

  const name = getTranslatedModelName(activeModel, t);
  const description = getTranslatedModelDescription(activeModel, t);
  const brand = getModelBrand(activeModel);
  const hasScores =
    activeModel.accuracy_score > 0 || activeModel.speed_score > 0;

  return (
    <div className={cardClasses}>
      <div className="flex items-start justify-between gap-4">
        <div className="flex items-start gap-3 min-w-0">
          <span
            className={`grid h-9 w-9 shrink-0 place-items-center rounded-[10px] ${brand.tileClass}`}
          >
            {brand.icon}
          </span>
          <div className="min-w-0">
            <p className="text-xs text-muted">
              {t("settings.dictation.hero.eyebrow")}
            </p>
            <div className="flex items-center gap-2 flex-wrap">
              <h2 className="font-display text-lg leading-tight text-ink">
                {name}
              </h2>
              {activeModel.supports_streaming &&
                !isLegacyModel(activeModel) && (
                  <Badge variant="success" className="gap-1">
                    <Radio className="w-3 h-3" />
                    {t("settings.dictation.hero.streams")}
                  </Badge>
                )}
            </div>
            {description && (
              <p className="mt-1 text-[13px] leading-snug text-muted max-w-md">
                {description}
              </p>
            )}
            {hasScores && (
              <div className="mt-2.5 flex flex-wrap items-center gap-x-5 gap-y-1.5">
                <ScoreMeter
                  label={t("settings.dictation.hero.accuracy")}
                  score={activeModel.accuracy_score}
                />
                <ScoreMeter
                  label={t("settings.dictation.hero.speed")}
                  score={activeModel.speed_score}
                />
              </div>
            )}
          </div>
        </div>
        <Button variant="secondary" size="sm" onClick={onChangeModel}>
          {t("settings.dictation.hero.changeModel")}
        </Button>
      </div>
    </div>
  );
};
