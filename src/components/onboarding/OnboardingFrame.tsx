import React from "react";
import { useTranslation } from "react-i18next";
import { Check } from "lucide-react";
import "./onboarding.css";

export type FlowStep = "setup" | "tour" | "finish";
const STEPS: FlowStep[] = ["setup", "tour", "finish"];

/**
 * The page every onboarding step sits on, drawn with the main window's own
 * parts so setup reads as the app rather than as a separate site: the content
 * is the same inset sheet the main shell uses (`bg-canvas`, a hairline edge,
 * the rounded top corner), titles use `font-display` one step above a page
 * title, and actions are ordinary `Button`s in a footer.
 */
export const OnboardingFrame: React.FC<{
  step: FlowStep;
  children: React.ReactNode;
  footer: React.ReactNode;
  /** The tour needs the full page width for its preview. */
  width?: "narrow" | "wide";
}> = ({ step, children, footer, width = "narrow" }) => {
  const { t } = useTranslation();
  const index = STEPS.indexOf(step);
  const column = width === "wide" ? "max-w-5xl" : "max-w-3xl";
  return (
    <div className="flex h-full min-h-0 flex-col bg-canvas-soft">
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-t-[1.25rem] border-t border-hairline bg-canvas elev-pane">
        <header className="shrink-0 px-6 pt-5 sm:px-10">
          <ol
            className={`mx-auto flex w-full ${column} items-center gap-2`}
            aria-label={t("onboarding.flow.progress", {
              current: index + 1,
              total: STEPS.length,
            })}
          >
            {STEPS.map((id, i) => {
              const state =
                i < index ? "done" : i === index ? "current" : "next";
              return (
                <li
                  key={id}
                  className="flex items-center gap-2"
                  aria-current={state === "current" ? "step" : undefined}
                >
                  {i > 0 && (
                    <span
                      aria-hidden="true"
                      className={`h-px w-6 ${i <= index ? "bg-accent/40" : "bg-hairline-strong"}`}
                    />
                  )}
                  <span
                    aria-hidden="true"
                    className={`grid h-5 w-5 place-items-center rounded-full text-[0.6875rem] font-semibold tabular-nums ${
                      state === "current"
                        ? "bg-ink text-on-ink"
                        : state === "done"
                          ? "bg-accent/12 text-accent"
                          : "bg-surface-strong text-muted"
                    }`}
                  >
                    {state === "done" ? (
                      <Check className="h-3 w-3" strokeWidth={3} />
                    ) : (
                      i + 1
                    )}
                  </span>
                  <span
                    className={`text-[0.8125rem] ${
                      state === "current"
                        ? "font-medium text-ink"
                        : "text-muted"
                    }`}
                  >
                    {t(`onboarding.frame.${id}`)}
                  </span>
                </li>
              );
            })}
          </ol>
        </header>
        <main className="ob-scroll min-h-0 flex-1 overflow-y-auto px-6 sm:px-10">
          <div
            key={step}
            className={`ob-page mx-auto flex min-h-full w-full ${column} flex-col justify-center py-10`}
          >
            {children}
          </div>
        </main>
        <footer className="shrink-0 border-t border-hairline px-6 py-4 sm:px-10">
          <div
            className={`mx-auto flex w-full ${column} items-center justify-between gap-3`}
          >
            {footer}
          </div>
        </footer>
      </div>
    </div>
  );
};

/** A step's title and one line under it: the page-title pattern, one size up. */
export const StepHeading: React.FC<{
  title: React.ReactNode;
  body?: React.ReactNode;
  /** Keeps a changing title announced (the finish step's download state). */
  live?: boolean;
  className?: string;
}> = ({ title, body, live = false, className = "" }) => (
  <div className={className}>
    <h1
      className="font-display text-[2rem] text-ink"
      aria-live={live ? "polite" : undefined}
    >
      {title}
    </h1>
    {body && (
      <p className="mt-2.5 max-w-xl text-[0.9375rem] leading-relaxed text-body text-pretty">
        {body}
      </p>
    )}
  </div>
);
