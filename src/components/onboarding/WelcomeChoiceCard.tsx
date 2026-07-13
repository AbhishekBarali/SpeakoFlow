import React from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import Badge from "../ui/Badge";
import { TONE_TILE_VIVID, type SettingTone } from "../ui/tones";

/** Download-lifecycle phase for the in-place morph. */
export type WelcomeCardPhase =
  | "idle"
  | "downloading"
  | "verifying"
  | "extracting"
  | "done";

interface WelcomeChoiceCardProps {
  /** Icon shown in the leading tile (typically a lucide icon). */
  icon: React.ReactNode;
  /** Soft tint for the icon tile. */
  tone?: SettingTone;
  /** Exact tile classes (e.g. a brand-color gradient). Overrides `tone` so
   *  real logos can sit on their real brand color. */
  tileClassName?: string;
  /** Model / option name (product name is fine; no quant jargon). */
  title: string;
  /** One warm, plain-language line. */
  description: string;
  /** Compact size string, e.g. "0.8 GB". */
  sizeLabel?: string;
  /** Quiet capability pill, e.g. "Sees your screen". */
  pill?: string;
  /** Single recommendation badge text (only one card should carry it). */
  badge?: string;
  /** Accent ring when this is the chosen card. */
  selected?: boolean;
  /** Dim + block interaction (e.g. another card is downloading). */
  disabled?: boolean;
  /**
   * When set to a download phase, the meta row morphs in place into a progress
   * state. `progress` drives the bar for the `downloading` phase; the others
   * show an indeterminate pulse.
   */
  phase?: WelcomeCardPhase;
  /** 0–100 download percentage (only meaningful while `phase` is "downloading"). */
  progress?: number;
  onClick: () => void;
}

/**
 * The single card used across the welcome flow — the two featured speech-to-text
 * options in Step 1 and the three assistant-brain tiers in Step 2. It stays
 * deliberately plain (name + one line + size), and when a download is running it
 * morphs the bottom row into a progress bar in place, so the user never has to
 * watch a spinner before moving on.
 */
const WelcomeChoiceCard: React.FC<WelcomeChoiceCardProps> = ({
  icon,
  tone,
  tileClassName,
  title,
  description,
  sizeLabel,
  pill,
  badge,
  selected = false,
  disabled = false,
  phase = "idle",
  progress,
  onClick,
}) => {
  const { t } = useTranslation();
  const isBusy = phase !== "idle" && phase !== "done";
  const clickable = !disabled;

  const handleClick = () => {
    if (!clickable) return;
    onClick();
  };

  const ringClasses = selected
    ? "border-accent/60 ring-1 ring-accent/30 bg-accent/5 shadow-[0_8px_32px_-12px_rgba(20,184,166,0.5)]"
    : "border-hairline-strong";

  const interactiveClasses = disabled
    ? "opacity-50 cursor-not-allowed"
    : "cursor-pointer hover:border-hairline-strong hover:shadow-[0_2px_8px_rgba(0,0,0,0.06)] group";

  const pct =
    typeof progress === "number"
      ? Math.max(0, Math.min(100, Math.round(progress)))
      : 0;

  const statusLabel =
    phase === "verifying"
      ? t("modelSelector.verifyingGeneric")
      : phase === "extracting"
        ? t("modelSelector.extractingGeneric")
        : t("modelSelector.downloading", { percentage: pct });

  return (
    <div
      onClick={handleClick}
      onKeyDown={(e) => {
        if (e.key === "Enter" && clickable) handleClick();
      }}
      role={clickable ? "button" : undefined}
      tabIndex={clickable ? 0 : undefined}
      aria-pressed={clickable ? selected : undefined}
      className={[
        "flex flex-col rounded-2xl px-5 py-4 gap-3 text-left transition-all duration-200 bg-surface border",
        ringClasses,
        interactiveClasses,
      ].join(" ")}
    >
      {/* Header: icon tile + name/description */}
      <div className="flex items-start gap-3.5 w-full">
        <span
          className={`shrink-0 grid place-items-center w-11 h-11 rounded-xl ${
            tileClassName ??
            (tone ? TONE_TILE_VIVID[tone] : "bg-surface-strong")
          }`}
        >
          {icon}
        </span>
        <div className="flex flex-col items-start flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <h3
              className={`text-[15px] font-semibold text-text ${clickable ? "group-hover:text-accent" : ""} transition-colors`}
            >
              {title}
            </h3>
            {badge && <Badge variant="active">{badge}</Badge>}
          </div>
          <p className="text-body text-[13px] leading-relaxed">{description}</p>
        </div>
      </div>

      {/* Bottom row — morphs into a progress bar in place while downloading. */}
      {isBusy ? (
        <div className="w-full">
          <div className="w-full h-1.5 bg-mid-gray/20 rounded-full overflow-hidden">
            <div
              className={`h-full bg-logo-primary rounded-full transition-all duration-300 ${
                phase === "downloading" ? "" : "animate-pulse w-full"
              }`}
              style={phase === "downloading" ? { width: `${pct}%` } : undefined}
            />
          </div>
          <p className="mt-1 flex items-center gap-1.5 text-xs text-text/50">
            <Loader2 className="w-3.5 h-3.5 animate-spin text-accent" />
            <span className="tabular-nums">{statusLabel}</span>
          </p>
        </div>
      ) : (
        (pill || sizeLabel) && (
          <div className="flex items-center gap-3 w-full h-5 text-xs text-text/50">
            {pill && (
              <span className="inline-flex items-center rounded-md border border-hairline-strong px-2 py-0.5 text-[11px] text-muted">
                {pill}
              </span>
            )}
            {sizeLabel && (
              <span className="ms-auto tabular-nums">{sizeLabel}</span>
            )}
          </div>
        )
      )}
    </div>
  );
};

export default WelcomeChoiceCard;
