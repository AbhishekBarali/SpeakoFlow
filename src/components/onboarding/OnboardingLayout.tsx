import React from "react";
import { useTranslation } from "react-i18next";
import Wordmark from "../Wordmark";

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
}) => {
  const { t } = useTranslation();

  return (
    <div className="h-full w-full flex flex-col items-center px-6 pt-8 pb-4 gap-5">
      {/* Brand + step progress */}
      <div className="flex flex-col items-center gap-4 shrink-0 w-full max-w-[640px]">
        <Wordmark className="text-3xl" />

        {/* Segmented progress indicator: one segment per step, with the
            current/completed steps filled. The active step's label sits below. */}
        <div className="flex flex-col items-center gap-2 w-full">
          <div
            className="flex items-center gap-1.5"
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
                className={`h-1.5 w-10 rounded-full transition-colors duration-300 ${
                  i < step ? "bg-accent" : "bg-hairline-strong"
                }`}
              />
            ))}
          </div>
          <p className="text-xs font-medium text-muted">
            {t("onboarding.steps.progress", {
              current: step,
              total: totalSteps,
            })}
          </p>
        </div>
      </div>

      {/* Title + subtitle */}
      <div className="flex flex-col items-center gap-1.5 text-center shrink-0 max-w-[560px]">
        <h1 className="font-display text-2xl leading-tight text-ink">{title}</h1>
        {subtitle && (
          <p className="text-sm leading-snug text-muted max-w-md">{subtitle}</p>
        )}
      </div>

      {/* Scrollable body */}
      <div className="flex-1 min-h-0 w-full max-w-[600px] overflow-y-auto">
        <div className="flex flex-col gap-3 pb-2">{children}</div>
      </div>

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
