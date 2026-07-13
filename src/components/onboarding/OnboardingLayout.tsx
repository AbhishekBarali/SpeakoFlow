import React from "react";
import { useTranslation } from "react-i18next";
import Wordmark from "../Wordmark";
import DownloadProgress from "./DownloadProgress";

interface OnboardingLayoutProps {
  /** 1-based index of the current step among the guided model steps. */
  step: number;
  /** Total number of guided model steps (e.g. 2: speech-to-text + AI model). */
  totalSteps: number;
  /** Large heading for this step. */
  title: string;
  /** Optional supporting line under the title. */
  subtitle?: string;
  /** Scrollable body — typically the list of model cards. */
  children: React.ReactNode;
  /** Optional sticky footer actions (skip / continue). */
  footer?: React.ReactNode;
  /**
   * Whether to render the shared background-download strip. On by default; a
   * step turns it off when it shows download progress its own way (Step 2's
   * in-place card morph, Step 3's warm status lines).
   */
  showDownloadProgress?: boolean;
}

/**
 * Shared chrome for the first-run model-selection wizard. Renders the wordmark,
 * a "Step N of total" progress indicator, the step title/subtitle, a scrollable
 * body, and an optional sticky footer. Keeping this in one place makes the two
 * model steps (speech-to-text, then AI model) read as a single guided flow
 * instead of two unrelated screens.
 */
const OnboardingLayout: React.FC<OnboardingLayoutProps> = ({
  step,
  totalSteps,
  title,
  subtitle,
  children,
  footer,
  showDownloadProgress = true,
}) => {
  const { t } = useTranslation();

  return (
    <div className="relative h-full w-full flex flex-col items-center px-6 pt-10 pb-4 gap-6 overflow-hidden">
      {/* Ambient brand glows so the screen has warmth and depth instead of
          flat gray: a strong teal wash from the top, a soft violet answer
          from the corner. */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-x-0 -top-24 h-96 bg-[radial-gradient(55%_100%_at_50%_0%,rgba(20,184,166,0.22),transparent_70%)]"
      />
      <div
        aria-hidden
        className="pointer-events-none absolute -bottom-20 -end-20 h-80 w-96 bg-[radial-gradient(50%_50%_at_50%_50%,rgba(139,92,246,0.1),transparent_70%)]"
      />
      {/* Brand + step progress */}
      <div className="flex flex-col items-center gap-4 shrink-0 w-full max-w-[640px]">
        <Wordmark className="text-3xl" />

        {/* Step dots: past = small filled, current = elongated pill, next =
            quiet dot. Reads at a glance without shouting. */}
        <div
          className="flex items-center gap-2"
          role="progressbar"
          aria-valuenow={step}
          aria-valuemin={1}
          aria-valuemax={totalSteps}
          aria-label={t("onboarding.steps.progress", {
            current: step,
            total: totalSteps,
          })}
        >
          {Array.from({ length: totalSteps }).map((_, i) => (
            <span
              key={i}
              className={`h-1.5 rounded-full transition-all duration-500 ${
                i + 1 === step
                  ? "w-6 bg-accent"
                  : i + 1 < step
                    ? "w-1.5 bg-accent/60"
                    : "w-1.5 bg-hairline-strong"
              }`}
            />
          ))}
        </div>
      </div>

      {/* Title + subtitle */}
      <div className="anim-rise flex flex-col items-center gap-2 text-center shrink-0 max-w-[560px]">
        <h1 className="font-display text-3xl leading-tight text-ink">
          {title}
        </h1>
        {subtitle && (
          <p className="text-[14px] leading-relaxed text-muted max-w-md">
            {subtitle}
          </p>
        )}
      </div>

      {/* Scrollable body */}
      <div className="flex-1 min-h-0 w-full max-w-[600px] overflow-y-auto">
        <div className="anim-rise anim-delay-1 flex flex-col gap-3 pb-2">
          {children}
        </div>
      </div>

      {/* Background download tracker — a single, consistent, non-blocking place
          that shows any in-flight model download (name + size + percent +
          speed) on every onboarding step. */}
      {showDownloadProgress && <DownloadProgress />}

      {/* Sticky footer actions */}
      {footer && (
        <div className="shrink-0 w-full max-w-[600px] flex items-center justify-between gap-3 pt-3 border-t border-hairline">
          {footer}
        </div>
      )}
    </div>
  );
};

export default OnboardingLayout;
