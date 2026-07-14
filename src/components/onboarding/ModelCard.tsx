import React from "react";
import { useTranslation } from "react-i18next";
import {
  Check,
  Cpu,
  Download,
  Globe,
  HardDrive,
  Languages,
  Loader2,
  Radio,
  Trash2,
} from "lucide-react";
import type { ModelInfo } from "@/bindings";
import { getModelBrand } from "../icons/BrandLogos";
import { formatModelSize } from "../../lib/utils/format";
import { extractQuant } from "../../lib/utils/modelQuant";
import {
  getTranslatedModelDescription,
  getTranslatedModelName,
} from "../../lib/utils/modelTranslation";
import { LANGUAGES } from "../../lib/constants/languages";
import Badge from "../ui/Badge";
import { Button } from "../ui/Button";

// Get display text for model's language support
const getLanguageDisplayText = (
  supportedLanguages: string[],
  t: (key: string, options?: Record<string, unknown>) => string,
): string => {
  if (supportedLanguages.length === 1) {
    const langCode = supportedLanguages[0];
    const langName =
      LANGUAGES.find((l) => l.value === langCode)?.label || langCode;
    return t("modelSelector.capabilities.languageOnly", { language: langName });
  }
  return t("modelSelector.capabilities.multiLanguage");
};

/**
 * A "legacy" transcription model runs on the older transcribe-rs (ONNX /
 * whisper.cpp) engine rather than the native transcribe.cpp (GGUF) engine.
 * LLM ("LlamaCpp") and TTS ("Kokoro") models are never "legacy". Exported so
 * the Models settings page can group these under a quiet "Older models"
 * section (PLAN.md Session 6) without duplicating the rule.
 */
export const isLegacyModel = (model: ModelInfo): boolean =>
  model.engine_type !== "TranscribeCpp" &&
  model.engine_type !== "LlamaCpp" &&
  model.engine_type !== "Kokoro";

export type ModelCardStatus =
  | "downloadable"
  | "downloading"
  | "verifying"
  | "extracting"
  | "switching"
  | "active"
  | "available";

interface ModelCardProps {
  model: ModelInfo;
  variant?: "default" | "featured";
  status?: ModelCardStatus;
  disabled?: boolean;
  className?: string;
  onSelect: (modelId: string) => void;
  onDownload?: (modelId: string) => void;
  onDelete?: (modelId: string) => void;
  onCancel?: (modelId: string) => void;
  downloadProgress?: number;
  downloadSpeed?: number; // MB/s
  showRecommended?: boolean;
  /** Show the accuracy/speed meters. Off in the first-run onboarding to keep
   *  the cards clean and focused on the description. */
  showScores?: boolean;
  /** Show the card's own download/verify/extract progress block. Off in the
   *  first-run onboarding, where the shared DownloadProgress strip is the single
   *  consistent place for progress (avoids a duplicate bar on the same screen). */
  showInlineProgress?: boolean;
  /** Show an explicit primary action instead of making the whole card the only
   * download/select affordance. Used where first-time discoverability matters. */
  showPrimaryAction?: boolean;
}

/** Accuracy / speed as a slim, quiet aligned bar — deliberately secondary to
 *  the model name + plain-language description (PLAN.md Session 6). The label
 *  sits in its own fixed column so both rows line up, and the fill is a muted
 *  neutral (not the brand accent) so the meters read as a supporting detail. */
const ScoreMeter: React.FC<{ label: string; score: number }> = ({
  label,
  score,
}) => {
  const pct = Math.max(0, Math.min(100, Math.round(score * 100)));
  return (
    <div className="flex items-center gap-2">
      <span className="w-14 shrink-0 text-end text-[9px] font-medium uppercase tracking-[0.06em] text-muted">
        {label}
      </span>
      <div
        className="h-1 w-12 shrink-0 overflow-hidden rounded-full bg-ink/15"
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

const ModelCard: React.FC<ModelCardProps> = ({
  model,
  variant = "default",
  status = "downloadable",
  disabled = false,
  className = "",
  onSelect,
  onDownload,
  onDelete,
  onCancel,
  downloadProgress,
  downloadSpeed,
  showRecommended = true,
  showScores = true,
  showInlineProgress = true,
  showPrimaryAction = false,
}) => {
  const { t } = useTranslation();
  const isFeatured = variant === "featured";
  const isClickable =
    status === "available" || status === "active" || status === "downloadable";
  const cardIsClickable = isClickable && !showPrimaryAction;

  // A "legacy" transcription model runs on the older transcribe-rs (ONNX /
  // whisper.cpp) engine rather than the new native transcribe.cpp (GGUF) one.
  // The catalog groups these under "Older models"; the card itself carries no
  // tag, and the Streaming badge is reserved for the modern engine's live
  // path (Nemotron / Parakeet Unified) so it stays a meaningful signal.
  const isLegacyStt = isLegacyModel(model);

  // Brand mark + tinted tile (NVIDIA / Qwen / Gemma / Whisper / fallback).
  const brand = getModelBrand(model);

  // Get translated model name and description
  const displayName = getTranslatedModelName(model, t);
  const displayDescription = getTranslatedModelDescription(model, t);
  // GGUF quantization tag (e.g. "Q8_0") — shown only on custom models, where
  // it disambiguates variants; catalog models keep the row jargon-free.
  const quant = model.is_custom ? extractQuant(model.filename) : null;
  const showModelSize =
    status === "downloadable" || status === "available" || status === "active";
  const formattedModelSize = formatModelSize(Number(model.size_mb));

  const baseClasses =
    "flex flex-col rounded-2xl px-4 py-3.5 gap-2 text-left transition-all duration-200";

  const getVariantClasses = () => {
    if (status === "active") {
      // Selected state: a quiet accent ring. The tinted "Active" badge
      // carries the status; the card only needs to read as current.
      return "border border-accent/50 bg-surface ring-1 ring-accent/25";
    }
    if (isFeatured) {
      return "border border-hairline-strong bg-surface";
    }
    return "border border-hairline bg-surface";
  };

  const getInteractiveClasses = () => {
    if (!cardIsClickable) return "";
    if (disabled) return "opacity-50 cursor-not-allowed";
    return "cursor-pointer hover:border-hairline-strong hover:shadow-[0_2px_8px_rgba(0,0,0,0.06)]";
  };

  const handleClick = () => {
    if (!cardIsClickable || disabled) return;
    if (status === "downloadable" && onDownload) {
      onDownload(model.id);
    } else {
      onSelect(model.id);
    }
  };

  const handleDelete = (e: React.MouseEvent) => {
    e.stopPropagation();
    onDelete?.(model.id);
  };

  return (
    <div
      onClick={handleClick}
      onKeyDown={(e) => {
        if (e.key === "Enter" && cardIsClickable) handleClick();
      }}
      role={cardIsClickable ? "button" : undefined}
      tabIndex={cardIsClickable ? 0 : undefined}
      className={[
        "group",
        baseClasses,
        getVariantClasses(),
        getInteractiveClasses(),
        className,
      ]
        .filter(Boolean)
        .join(" ")}
    >
      {/* Top section: brand tile + name/description + score bars */}
      <div className="flex justify-between items-center w-full gap-3">
        <span
          className={`shrink-0 grid place-items-center w-9 h-9 rounded-[10px] ${brand.tileClass}`}
        >
          {brand.icon}
        </span>
        <div className="flex flex-col items-start flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <h3
              className={`text-sm font-semibold text-text ${isClickable ? "group-hover:text-accent" : ""} transition-colors`}
            >
              {displayName}
            </h3>
            {status === "active" && (
              <Badge variant="active">
                <Check className="w-3 h-3 mr-1" />
                {t("modelSelector.active")}
              </Badge>
            )}
            {status === "switching" && (
              <Badge variant="secondary">
                <Loader2 className="w-3 h-3 mr-1 animate-spin" />
                {t("modelSelector.switching")}
              </Badge>
            )}
            {model.supports_streaming && !isLegacyStt && (
              <Badge variant="success" className="gap-1">
                <Radio className="w-3 h-3" />
                {t("modelSelector.capabilities.streaming")}
              </Badge>
            )}
            {showRecommended && model.is_recommended && status !== "active" && (
              <Badge variant="outline">{t("onboarding.recommended")}</Badge>
            )}
          </div>
          <p className="text-body text-[13px] leading-relaxed">
            {displayDescription}
          </p>
        </div>
        {showScores && (model.accuracy_score > 0 || model.speed_score > 0) && (
          <div className="hidden sm:flex flex-col gap-1.5 ms-1 shrink-0">
            <ScoreMeter
              label={t("onboarding.modelCard.accuracy")}
              score={model.accuracy_score}
            />
            <ScoreMeter
              label={t("onboarding.modelCard.speed")}
              score={model.speed_score}
            />
          </div>
        )}
      </div>

      {/* Bottom row: quiet metadata; the delete action stays hidden until the
          row is hovered or focused so a list of installed models doesn't read
          as a wall of destructive buttons. */}
      <div className="flex items-center gap-3 w-full -mb-0.5 h-6 ps-12">
        {model.supported_languages.length > 0 && (
          <div
            className="flex items-center gap-1 text-xs text-muted"
            title={
              model.supported_languages.length === 1
                ? t("modelSelector.capabilities.singleLanguage")
                : t("modelSelector.capabilities.languageSelection")
            }
          >
            <Globe className="w-3.5 h-3.5" />
            <span>{getLanguageDisplayText(model.supported_languages, t)}</span>
          </div>
        )}
        {model.supports_translation && (
          <div
            className="flex items-center gap-1 text-xs text-muted"
            title={t("modelSelector.capabilities.translation")}
          >
            <Languages className="w-3.5 h-3.5" />
            <span>{t("modelSelector.capabilities.translate")}</span>
          </div>
        )}
        {quant && (
          <div
            className="flex items-center gap-1 text-xs text-muted"
            title={t("modelSelector.capabilities.quantization")}
          >
            <Cpu className="w-3.5 h-3.5" />
            <span className="font-mono">{quant}</span>
          </div>
        )}
        {showModelSize && (
          <span className="ms-auto flex items-center gap-1.5 text-xs text-muted">
            {status === "downloadable" ? (
              <Download className="w-3.5 h-3.5" />
            ) : (
              <HardDrive className="w-3.5 h-3.5" />
            )}
            <span>{formattedModelSize}</span>
          </span>
        )}
        {showPrimaryAction &&
          (status === "downloadable" || status === "available") && (
            <Button
              variant={status === "downloadable" ? "primary" : "secondary"}
              size="sm"
              disabled={disabled}
              onClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                if (status === "downloadable") {
                  onDownload?.(model.id);
                } else {
                  onSelect(model.id);
                }
              }}
            >
              {status === "downloadable" && (
                <Download className="h-3.5 w-3.5" />
              )}
              {status === "downloadable"
                ? t("modelSelector.download")
                : t("modelSelector.useModel")}
            </Button>
          )}
        {onDelete && (status === "available" || status === "active") && (
          <Button
            variant="ghost"
            size="sm"
            onClick={handleDelete}
            title={t("modelSelector.deleteModel", { modelName: displayName })}
            className="flex items-center gap-1.5 text-muted hover:text-error hover:bg-error/10 opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity"
          >
            <Trash2 className="w-3.5 h-3.5" />
            <span>{t("common.delete")}</span>
          </Button>
        )}
      </div>

      {/* Download/extract progress */}
      {showInlineProgress &&
        status === "downloading" &&
        downloadProgress !== undefined && (
          <div className="w-full mt-3">
            <div className="w-full h-1.5 bg-mid-gray/20 rounded-full overflow-hidden">
              <div
                className="h-full bg-logo-primary rounded-full transition-all duration-300"
                style={{ width: `${downloadProgress}%` }}
              />
            </div>
            <div className="flex items-center justify-between text-xs mt-1">
              <span className="text-muted">
                {t("modelSelector.downloading", {
                  percentage: Math.round(downloadProgress),
                })}
              </span>
              <div className="flex items-center gap-2">
                {downloadSpeed !== undefined && downloadSpeed > 0 && (
                  <span className="tabular-nums text-muted">
                    {t("modelSelector.downloadSpeed", {
                      speed: downloadSpeed.toFixed(1),
                    })}
                  </span>
                )}
                {onCancel && (
                  <Button
                    variant="danger-ghost"
                    size="sm"
                    onClick={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      onCancel(model.id);
                    }}
                    aria-label={t("modelSelector.cancelDownload")}
                  >
                    {t("modelSelector.cancel")}
                  </Button>
                )}
              </div>
            </div>
          </div>
        )}
      {showInlineProgress && status === "verifying" && (
        <div className="w-full mt-3">
          <div className="w-full h-1.5 bg-mid-gray/20 rounded-full overflow-hidden">
            <div className="h-full bg-logo-primary rounded-full animate-pulse w-full" />
          </div>
          <p className="text-xs text-muted mt-1">
            {t("modelSelector.verifyingGeneric")}
          </p>
        </div>
      )}
      {showInlineProgress && status === "extracting" && (
        <div className="w-full mt-3">
          <div className="w-full h-1.5 bg-mid-gray/20 rounded-full overflow-hidden">
            <div className="h-full bg-logo-primary rounded-full animate-pulse w-full" />
          </div>
          <p className="text-xs text-muted mt-1">
            {t("modelSelector.extractingGeneric")}
          </p>
        </div>
      )}
    </div>
  );
};

export default ModelCard;
