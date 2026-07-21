import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronUp, Loader2 } from "lucide-react";
import type { ModelInfo } from "@/bindings";
import { useModelStore } from "../../stores/modelStore";
import { getTranslatedModelName } from "../../lib/utils/modelTranslation";

/**
 * The single, cohesive home for model-download status.
 *
 * Before, the same download was scattered across three surfaces: a
 * "Downloading 6%" status button, a separate speed/progress bar, and a floating
 * bottom-right card. This collapses all of it into one pill that lives centered
 * in the footer:
 *
 *   • Collapsed — a progress ring + percentage (or a spinner for the brief,
 *     percentage-less verify/extract phases).
 *   • Expanded  — click to reveal a compact panel with the model name, live
 *     speed, downloaded / total size, and a progress bar for every active
 *     download.
 *
 * Renders nothing when no download/verify/extract is in flight.
 */

const RING_RADIUS = 7;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

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

type Phase = "downloading" | "verifying" | "extracting";

interface ActiveItem {
  id: string;
  name: string;
  phase: Phase;
  pct: number;
  downloaded: number;
  total: number;
  speed?: number;
}

const DownloadIndicator: React.FC = () => {
  const { t } = useTranslation();
  const models = useModelStore((s) => s.models);
  const downloadingModels = useModelStore((s) => s.downloadingModels);
  const verifyingModels = useModelStore((s) => s.verifyingModels);
  const extractingModels = useModelStore((s) => s.extractingModels);
  const downloadProgress = useModelStore((s) => s.downloadProgress);
  const downloadStats = useModelStore((s) => s.downloadStats);

  const [expanded, setExpanded] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  const items = useMemo<ActiveItem[]>(() => {
    const ids = Array.from(
      new Set([
        ...Object.keys(downloadingModels),
        ...Object.keys(verifyingModels),
        ...Object.keys(extractingModels),
      ]),
    );
    return ids.map((id) => {
      const model = models.find((m: ModelInfo) => m.id === id);
      const progress = downloadProgress[id];
      const stats = downloadStats[id];
      // Sequential lifecycle: download → verify → extract. Prefer the later
      // phase if (rarely) more than one flag is set.
      const phase: Phase =
        id in extractingModels
          ? "extracting"
          : id in verifyingModels
            ? "verifying"
            : "downloading";
      const total =
        progress && progress.total > 0
          ? progress.total
          : model
            ? Number(model.size_mb) * 1024 * 1024
            : 0;
      return {
        id,
        name: model ? getTranslatedModelName(model, t) : id,
        phase,
        pct: Math.max(0, Math.min(100, Math.round(progress?.percentage ?? 0))),
        downloaded: progress?.downloaded ?? 0,
        total,
        speed: stats?.speed,
      };
    });
  }, [
    models,
    downloadingModels,
    verifyingModels,
    extractingModels,
    downloadProgress,
    downloadStats,
    t,
  ]);

  // Auto-collapse once everything finishes so a stale panel never lingers.
  useEffect(() => {
    if (items.length === 0 && expanded) setExpanded(false);
  }, [items.length, expanded]);

  // Close the panel on outside click / Escape while it's open.
  useEffect(() => {
    if (!expanded) return;
    const onPointerDown = (event: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setExpanded(false);
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setExpanded(false);
    };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [expanded]);

  // Collapsed summary. Downloading takes priority (it has a percentage).
  const summary = useMemo(() => {
    const downloading = items.filter((i) => i.phase === "downloading");
    if (downloading.length === 1) {
      return {
        label: t("modelSelector.downloading", {
          percentage: downloading[0].pct,
        }),
        pct: downloading[0].pct,
        showRing: true,
      };
    }
    if (downloading.length > 1) {
      return {
        label: t("modelSelector.downloadingMultiple", {
          count: downloading.length,
        }),
        pct: 0,
        showRing: false,
      };
    }
    if (items.some((i) => i.phase === "verifying")) {
      return {
        label: t("modelSelector.verifyingGeneric"),
        pct: 0,
        showRing: false,
      };
    }
    if (items.some((i) => i.phase === "extracting")) {
      return {
        label: t("modelSelector.extractingGeneric"),
        pct: 0,
        showRing: false,
      };
    }
    return null;
  }, [items, t]);

  if (!summary) return null;

  const dashOffset = RING_CIRCUMFERENCE * (1 - summary.pct / 100);

  return (
    <div className="relative" ref={rootRef}>
      <button
        type="button"
        onClick={() => setExpanded((open) => !open)}
        title={t("modelSelector.downloadDetails")}
        aria-label={t("modelSelector.downloadDetails")}
        aria-expanded={expanded}
        className="flex items-center gap-2 rounded-full border border-hairline bg-surface px-2.5 py-1 text-xs text-text transition-colors hover:border-hairline-strong"
      >
        {summary.showRing ? (
          <span className="relative inline-flex h-4 w-4 items-center justify-center">
            <svg className="h-4 w-4 -rotate-90" viewBox="0 0 18 18">
              <circle
                cx="9"
                cy="9"
                r={RING_RADIUS}
                fill="none"
                strokeWidth="2"
                className="stroke-hairline-strong"
              />
              <circle
                cx="9"
                cy="9"
                r={RING_RADIUS}
                fill="none"
                strokeWidth="2"
                strokeLinecap="round"
                className="stroke-logo-primary transition-[stroke-dashoffset] duration-300"
                strokeDasharray={RING_CIRCUMFERENCE}
                strokeDashoffset={dashOffset}
              />
            </svg>
          </span>
        ) : (
          <Loader2 className="h-3.5 w-3.5 animate-spin text-logo-primary" />
        )}
        <span className="tabular-nums font-medium">{summary.label}</span>
        <ChevronUp
          className={`h-3 w-3 text-muted transition-transform ${
            expanded ? "rotate-180" : ""
          }`}
        />
      </button>

      {expanded && (
        <div
          role="status"
          aria-live="polite"
          className="absolute bottom-full left-1/2 z-50 mb-2 w-80 -translate-x-1/2 space-y-3 rounded-xl border border-hairline bg-surface p-3 shadow-[0_16px_40px_rgba(0,0,0,0.24)]"
        >
          {items.map((item) => {
            const indeterminate = item.phase !== "downloading";
            const phaseLabel =
              item.phase === "extracting"
                ? t("modelSelector.extractingGeneric")
                : item.phase === "verifying"
                  ? t("modelSelector.verifyingGeneric")
                  : null;
            return (
              <div key={item.id} className="space-y-1.5">
                <div className="flex items-center gap-2 text-xs">
                  <span className="min-w-0 flex-1 truncate font-medium text-text">
                    {item.name}
                  </span>
                  <span className="shrink-0 tabular-nums text-muted">
                    {indeterminate ? phaseLabel : `${item.pct}%`}
                  </span>
                </div>
                <div className="h-1.5 w-full overflow-hidden rounded-full bg-hairline-strong">
                  <div
                    className={`h-full rounded-full bg-logo-primary ${
                      indeterminate
                        ? "w-full animate-pulse"
                        : "transition-all duration-300"
                    }`}
                    style={
                      indeterminate ? undefined : { width: `${item.pct}%` }
                    }
                  />
                </div>
                {!indeterminate && (
                  <div className="flex items-center justify-between text-[11px] tabular-nums text-muted">
                    <span>
                      {t("modelSelector.downloadSize", {
                        downloaded: formatBytes(item.downloaded),
                        total: formatBytes(item.total),
                      })}
                    </span>
                    {item.speed !== undefined && item.speed > 0 && (
                      <span>
                        {t("modelSelector.downloadSpeed", {
                          speed: item.speed.toFixed(1),
                        })}
                      </span>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};

export default DownloadIndicator;
