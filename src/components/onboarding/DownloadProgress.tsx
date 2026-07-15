import React from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import type { ModelInfo } from "@/bindings";
import { useModelStore } from "../../stores/modelStore";
import { getTranslatedModelName } from "../../lib/utils/modelTranslation";

/** Format a raw byte count as a compact MB/GB string (locale-aware). */
const formatBytes = (bytes: number): string => {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 MB";
  const mb = bytes / (1024 * 1024);
  if (mb >= 1024) {
    const gb = mb / 1024;
    const formatter = new Intl.NumberFormat(undefined, {
      minimumFractionDigits: gb >= 10 ? 0 : 1,
      maximumFractionDigits: gb >= 10 ? 0 : 1,
    });
    return `${formatter.format(gb)} GB`;
  }
  const formatter = new Intl.NumberFormat(undefined, {
    minimumFractionDigits: mb >= 100 ? 0 : 1,
    maximumFractionDigits: mb >= 100 ? 0 : 1,
  });
  return `${formatter.format(mb)} MB`;
};

/**
 * A single, consistent place to show background model downloads. It subscribes
 * to the model store directly, so it renders in the same spot on BOTH the
 * speech-to-text and AI-model onboarding steps and keeps showing progress after
 * the user has moved on — which is what makes the downloads non-blocking.
 *
 * Renders nothing when no download/verify/extract is in flight.
 */
const DownloadProgress: React.FC = () => {
  const { t } = useTranslation();
  const models = useModelStore((s) => s.models);
  const downloadingModels = useModelStore((s) => s.downloadingModels);
  const verifyingModels = useModelStore((s) => s.verifyingModels);
  const extractingModels = useModelStore((s) => s.extractingModels);
  const downloadProgress = useModelStore((s) => s.downloadProgress);
  const downloadStats = useModelStore((s) => s.downloadStats);

  // Union of every model currently downloading, verifying, or extracting.
  const activeIds = Array.from(
    new Set([
      ...Object.keys(downloadingModels),
      ...Object.keys(verifyingModels),
      ...Object.keys(extractingModels),
    ]),
  );

  if (activeIds.length === 0) return null;

  return (
    <div
      className="w-full max-w-[600px] flex flex-col gap-2 shrink-0"
      role="status"
      aria-live="polite"
    >
      {activeIds.map((id) => {
        const model = models.find((m: ModelInfo) => m.id === id);
        const name = model ? getTranslatedModelName(model, t) : id;
        const progress = downloadProgress[id];
        const stats = downloadStats[id];
        const isExtracting = id in extractingModels;
        const isVerifying = id in verifyingModels;
        // Verifying/extracting have no meaningful percentage — show an
        // indeterminate (pulsing, full-width) bar for those phases.
        const indeterminate = isExtracting || isVerifying;
        const pct = progress
          ? Math.max(0, Math.min(100, Math.round(progress.percentage)))
          : 0;

        // Prefer the live byte totals; fall back to the catalog size before the
        // first progress event arrives so the row still reads "… of <size>".
        const totalBytes =
          progress && progress.total > 0
            ? progress.total
            : model
              ? Number(model.size_mb) * 1024 * 1024
              : 0;
        const downloadedBytes = progress ? progress.downloaded : 0;

        let statusLabel: string;
        if (isExtracting) {
          statusLabel = t("modelSelector.extractingGeneric");
        } else if (isVerifying) {
          statusLabel = t("modelSelector.verifyingGeneric");
        } else {
          statusLabel = t("onboarding.downloadProgress.sizeOf", {
            downloaded: formatBytes(downloadedBytes),
            total: formatBytes(totalBytes),
            percentage: pct,
          });
        }

        return (
          <div
            key={id}
            className="w-full rounded-lg border border-hairline bg-surface px-3 py-2"
          >
            <div className="flex items-center justify-between gap-2 text-xs">
              <span className="flex items-center gap-1.5 min-w-0 font-medium text-text">
                <Loader2 className="w-3.5 h-3.5 animate-spin text-accent shrink-0" />
                <span className="truncate">{name}</span>
              </span>
              <span className="tabular-nums text-muted shrink-0">
                {statusLabel}
              </span>
            </div>
            <div className="mt-1.5 w-full h-1.5 bg-hairline-strong rounded-full overflow-hidden">
              <div
                className={`h-full bg-accent rounded-full transition-all duration-300 ${
                  indeterminate ? "animate-pulse w-full" : ""
                }`}
                style={indeterminate ? undefined : { width: `${pct}%` }}
              />
            </div>
            {!indeterminate && stats && stats.speed > 0 && (
              <div className="mt-1 text-[11px] text-muted tabular-nums">
                {t("modelSelector.downloadSpeed", {
                  speed: stats.speed.toFixed(1),
                })}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
};

export default DownloadProgress;
