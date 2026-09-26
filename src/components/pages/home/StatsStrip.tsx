import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight } from "lucide-react";
import { commands, events, type UsageStats } from "@/bindings";
import { InfoTip } from "@/components/ui/InfoTip";
import { useNavigation } from "@/components/shell/navigation";
import {
  formatCount,
  formatDuration,
  TYPING_WPM,
  timeSavedSeconds,
  wordsPerMinute,
} from "./usageMath";

/** Live lifetime usage, refetched whenever a dictation lands. */
export const useUsageStats = (): UsageStats | null => {
  const [stats, setStats] = useState<UsageStats | null>(null);
  const load = useCallback(() => {
    void commands
      .getUsageStats()
      .then((result) => {
        if (result.status === "ok") setStats(result.data);
      })
      .catch(() => {});
  }, []);
  useEffect(() => {
    load();
    const unlisten = events.historyUpdatePayload.listen((event) => {
      if (
        event.payload.action === "added" ||
        event.payload.action === "updated"
      ) {
        load();
      }
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [load]);
  return stats;
};

const Stat: React.FC<{
  value: string;
  label: string;
  info?: string;
}> = ({ value, label, info }) => (
  <div className="min-w-0 bg-surface px-5 py-4">
    <p className="truncate text-[1.625rem] leading-none font-semibold tracking-[-0.02em] text-ink tabular-nums">
      {value}
    </p>
    <p className="mt-2 flex items-center gap-1 text-[0.8125rem] text-muted">
      <span className="truncate">{label}</span>
      {info && <InfoTip text={info} />}
    </p>
  </div>
);

/**
 * Four numbers and a link to Insights. The day-by-day picture lives on the
 * Insights page, so Home stays a glance.
 */
export const StatsStrip: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { navigate } = useNavigation();
  const stats = useUsageStats();

  if (!stats) {
    return (
      <div
        aria-hidden="true"
        className="h-[5.75rem] animate-pulse rounded-2xl border border-hairline bg-surface-muted"
      />
    );
  }

  if (stats.total_words === 0) {
    return (
      <p className="rounded-2xl border border-dashed border-hairline-strong px-5 py-4 text-sm text-muted">
        {t("home.stats.empty")}
      </p>
    );
  }

  const wpm = wordsPerMinute(stats);

  return (
    <section aria-label={t("home.stats.label")}>
      {/* `gap-px` over a hairline fill draws the dividers, so the 2×2 layout
          of a narrow window and the 1×4 of a wide one both get clean rules. */}
      <div className="grid grid-cols-2 gap-px overflow-hidden rounded-2xl border border-hairline bg-hairline elev-card @2xl:grid-cols-4">
        <Stat
          value={formatCount(stats.total_words, i18n.language)}
          label={t("home.stats.wordsLabel")}
        />
        <Stat
          value={wpm !== null ? String(wpm) : t("home.stats.none")}
          label={t("home.stats.wpmLabel")}
          info={t("home.stats.wpmHint")}
        />
        <Stat
          value={formatDuration(timeSavedSeconds(stats), t)}
          label={t("home.stats.savedLabel")}
          info={t("home.stats.savedHint", { wpm: TYPING_WPM })}
        />
        <Stat
          value={String(stats.current_streak_days)}
          label={t("home.stats.streakLabel", {
            count: stats.current_streak_days,
          })}
        />
      </div>
      <div className="mt-2 flex justify-end">
        <button
          type="button"
          onClick={() => navigate("insights")}
          className="group inline-flex cursor-pointer items-center gap-1 rounded-md px-1.5 py-0.5 text-[0.8125rem] font-medium text-muted transition-colors hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
        >
          {t("home.stats.seeInsights")}
          <ArrowRight
            className="h-3.5 w-3.5 transition-transform group-hover:translate-x-0.5 rtl:rotate-180"
            aria-hidden="true"
          />
        </button>
      </div>
    </section>
  );
};
