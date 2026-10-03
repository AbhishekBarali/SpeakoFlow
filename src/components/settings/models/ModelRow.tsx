import React from "react";
import { useTranslation } from "react-i18next";
import {
  Check,
  Download,
  FileQuestion,
  Loader2,
  MoreHorizontal,
} from "lucide-react";
import type { ModelInfo } from "@/bindings";
import { Button } from "@/components/ui/Button";
import { MenuButton, type MenuItem } from "@/components/ui/Menu";
import { ModelMark } from "@/components/shell/SlotVisuals";

/** What the row's one action is, decided by the caller. */
export type ModelRowAction =
  | { kind: "download"; onClick: () => void }
  | { kind: "use"; onClick: () => void; label?: string; ariaLabel?: string }
  | { kind: "inUse" }
  | { kind: "switching" }
  | { kind: "missing"; title?: string }
  | { kind: "none" };

export interface ModelRowProgress {
  state: "downloading" | "verifying" | "extracting";
  /** 0–100, for downloads. */
  percent?: number;
  /** MB/s, for downloads. */
  speed?: number;
  onCancel?: () => void;
}

export interface ModelRowDetail {
  label: string;
  value: string;
  mono?: boolean;
}

/**
 * One model in a list, with exactly one thing to do on the right.
 *
 * The old rows carried a size, a details chevron, a "Use for ▾" menu and a
 * trash can side by side on every line — four controls to read before you knew
 * what the model was. A row now shows the model (its mark, a clean name, its
 * quantization as a quiet tag, one line about it) and a single action that
 * fits its state: Download, Use, In use, or what is wrong. Everything else —
 * using it for another job, its file details, removing it — is in the ⋯ menu.
 */
export const ModelRow: React.FC<{
  model: ModelInfo;
  name: string;
  quant?: string | null;
  subtitle?: string;
  /** Small tags after the name (in use for a job, Recommended, Streaming). */
  badges?: React.ReactNode;
  /** A line of capabilities under the subtitle. */
  meta?: React.ReactNode;
  size?: string;
  action: ModelRowAction;
  menu?: MenuItem[];
  progress?: ModelRowProgress | null;
  details?: ModelRowDetail[] | null;
  /** This row is the one doing the tab's job. */
  current?: boolean;
}> = ({
  model,
  name,
  quant,
  subtitle,
  badges,
  meta,
  size,
  action,
  menu,
  progress,
  details,
  current = false,
}) => {
  const { t } = useTranslation();
  const percent = Math.max(0, Math.min(100, progress?.percent ?? 0));

  return (
    <article
      className={`transition-colors duration-150 ${current ? "bg-accent/[0.04]" : ""}`}
    >
      <div className="flex items-center gap-3.5 px-4 py-3">
        <ModelMark model={model} size="lg" />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
            <h3
              className="min-w-0 truncate text-sm font-semibold text-ink"
              title={model.name}
            >
              {name}
            </h3>
            {quant && (
              <span className="shrink-0 rounded bg-surface-strong px-1.5 py-px font-mono text-[0.6875rem] leading-4 text-muted">
                {quant}
              </span>
            )}
            {badges}
          </div>
          {subtitle && (
            <p
              className="mt-0.5 truncate text-[0.8125rem] text-muted"
              title={subtitle}
            >
              {subtitle}
            </p>
          )}
          {meta && (
            <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted">
              {meta}
            </div>
          )}
        </div>

        {size && (
          <span className="hidden shrink-0 text-xs text-muted tabular-nums sm:inline">
            {size}
          </span>
        )}

        {/* A fixed width, wide enough for the widest action, so the sizes to
            its left line up down the list whatever each row's action is. */}
        <div className="flex w-[7.25rem] shrink-0 justify-end">
          {action.kind === "download" && (
            <Button variant="secondary" size="sm" onClick={action.onClick}>
              <Download className="h-3.5 w-3.5" aria-hidden="true" />
              {t("modelSelector.download")}
            </Button>
          )}
          {action.kind === "use" && (
            <Button
              variant="secondary"
              size="sm"
              onClick={action.onClick}
              aria-label={action.ariaLabel}
            >
              {action.label ?? t("catalog.use")}
            </Button>
          )}
          {action.kind === "inUse" && (
            <span className="inline-flex h-8 items-center gap-1.5 rounded-lg bg-accent/10 px-2.5 text-[0.8125rem] font-medium text-accent">
              <Check
                className="h-3.5 w-3.5"
                strokeWidth={2.5}
                aria-hidden="true"
              />
              {t("catalog.inUseShort")}
            </span>
          )}
          {action.kind === "switching" && (
            <span className="inline-flex h-8 items-center gap-1.5 px-2 text-[0.8125rem] text-muted">
              <Loader2
                className="h-3.5 w-3.5 animate-spin"
                aria-hidden="true"
              />
              {t("modelSelector.switching")}
            </span>
          )}
          {action.kind === "missing" && (
            <span
              className="inline-flex h-8 items-center gap-1.5 rounded-lg bg-warning/10 px-2.5 text-xs font-medium text-warning"
              title={action.title}
            >
              <FileQuestion className="h-3.5 w-3.5" aria-hidden="true" />
              {t("settings.models.localModel.missingBadge")}
            </span>
          )}
          {progress && action.kind === "none" && progress.onCancel && (
            <Button variant="ghost" size="sm" onClick={progress.onCancel}>
              {t("modelSelector.cancel")}
            </Button>
          )}
        </div>

        {menu && menu.length > 0 ? (
          <MenuButton
            items={menu}
            ariaLabel={t("catalog.moreFor", { model: name })}
            title={t("catalog.more")}
            className="grid h-8 w-8 shrink-0 cursor-pointer place-items-center rounded-lg text-muted transition-colors hover:bg-ink/[0.06] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 aria-expanded:bg-ink/[0.06] aria-expanded:text-ink"
          >
            <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
          </MenuButton>
        ) : (
          <span aria-hidden="true" className="w-8 shrink-0" />
        )}
      </div>

      {progress && (
        <div className="px-4 pb-3.5 ps-[4.375rem]">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-strong">
            <div
              className={`h-full rounded-full bg-accent ${
                progress.state === "downloading"
                  ? "transition-[width] duration-300"
                  : "w-full animate-pulse"
              }`}
              style={
                progress.state === "downloading"
                  ? { width: `${percent}%` }
                  : undefined
              }
            />
          </div>
          <div className="mt-1.5 flex items-center justify-between gap-3 text-xs text-muted">
            <span>
              {progress.state === "downloading"
                ? t("modelSelector.downloading", {
                    percentage: Math.round(percent),
                  })
                : progress.state === "verifying"
                  ? t("modelSelector.verifyingGeneric")
                  : t("modelSelector.extractingGeneric")}
            </span>
            {progress.state === "downloading" &&
              progress.speed !== undefined &&
              progress.speed > 0 && (
                <span className="tabular-nums">
                  {t("modelSelector.downloadSpeed", {
                    speed: progress.speed.toFixed(1),
                  })}
                </span>
              )}
          </div>
        </div>
      )}

      {details && details.length > 0 && (
        <dl className="grid gap-x-4 gap-y-1.5 border-t border-hairline bg-surface-muted px-4 py-3 ps-[4.375rem] text-xs sm:grid-cols-[auto_1fr]">
          {details.map((detail) => (
            <React.Fragment key={detail.label}>
              <dt className="font-medium text-muted">{detail.label}</dt>
              <dd
                className={`break-all text-body ${detail.mono ? "font-mono" : ""}`}
              >
                {detail.value}
              </dd>
            </React.Fragment>
          ))}
        </dl>
      )}
    </article>
  );
};

/** Accuracy or speed as five quiet dots: a comparison, not a number to read. */
export const ScoreDots: React.FC<{ label: string; score: number }> = ({
  label,
  score,
}) => {
  const filled = Math.max(0, Math.min(5, Math.round(score * 5)));
  return (
    <span
      className="inline-flex items-center gap-1.5"
      role="img"
      aria-label={`${label}: ${filled}/5`}
      title={`${label}: ${filled}/5`}
    >
      <span className="capitalize">{label}</span>
      <span aria-hidden="true" className="flex gap-[3px]">
        {Array.from({ length: 5 }, (_, index) => (
          <span
            key={index}
            className={`h-1.5 w-1.5 rounded-full ${index < filled ? "bg-ink/55" : "bg-ink/15"}`}
          />
        ))}
      </span>
    </span>
  );
};

/** A quiet tag after a model's name: "Assistant", "Recommended", "Live". */
export const ModelTag: React.FC<{
  tone?: "accent" | "neutral" | "success";
  children: React.ReactNode;
  icon?: React.ComponentType<{ className?: string }>;
}> = ({ tone = "neutral", children, icon: Icon }) => (
  <span
    className={`inline-flex shrink-0 items-center gap-1 rounded-md px-1.5 py-px text-[0.6875rem] leading-4 font-medium ${
      tone === "accent"
        ? "bg-accent/10 text-accent"
        : tone === "success"
          ? "bg-success/10 text-success"
          : "border border-hairline-strong text-muted"
    }`}
  >
    {Icon && <Icon className="h-3 w-3" aria-hidden="true" />}
    {children}
  </span>
);
