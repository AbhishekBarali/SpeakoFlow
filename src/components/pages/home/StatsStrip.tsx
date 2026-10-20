import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Clock, Flame, Gauge, Type } from "lucide-react";
import { commands, type UsageStats } from "@/bindings";
import { useNavigation } from "@/components/shell/navigation";
import { useInsights } from "../insights/useInsights";
import {
  formatCount,
  formatDuration,
  TYPING_WPM,
  timeSavedSeconds,
  wordsPerMinute,
} from "./usageMath";

/** The dictation figures on their own, read only while `enabled` (Insights
 *  could not be read), so the strip shows them instead of waiting, and read
 *  again whenever `refresh` changes. `"error"` when they cannot be read
 *  either. */
const useUsageStatsFallback = (
  enabled: boolean,
  refresh: number,
): UsageStats | "error" | null => {
  const [stats, setStats] = useState<UsageStats | "error" | null>(null);
  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    void commands
      .getUsageStats()
      .then((result) => {
        if (!cancelled)
          setStats(result.status === "ok" ? result.data : "error");
      })
      .catch(() => {
        if (!cancelled) setStats("error");
      });
    return () => {
      cancelled = true;
    };
  }, [enabled, refresh]);
  return enabled ? stats : null;
};

/** One number and its word, on a line. The explanation, where there is one,
 *  is the hover text rather than an (i) beside every figure. */
const Stat: React.FC<{
  icon: React.ComponentType<{ className?: string }>;
  tint: string;
  value: string;
  label: string;
  hint?: string;
}> = ({ icon: Icon, tint, value, label, hint }) => (
  <span
    title={hint}
    className="inline-flex min-w-0 items-center gap-2 whitespace-nowrap"
  >
    <span
      aria-hidden="true"
      className={`grid h-6 w-6 shrink-0 place-items-center rounded-md ${tint}`}
    >
      <Icon className="h-3.5 w-3.5" />
    </span>
    <span className="text-sm font-semibold text-ink tabular-nums">{value}</span>
    <span className="truncate text-[0.8125rem] text-muted">{label}</span>
  </span>
);

/**
 * Four numbers on one slim line, and the way into Insights.
 *
 * It used to be a row of big tiles — 26px figures in a card as tall as the
 * shortcuts beside it — which pulled the eye away from the parts of Home you
 * act on. The figures are a glance, so they get a line; the full picture is
 * one click away, and the whole line is that click.
 */
export const StatsStrip: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { navigate } = useNavigation();
  const { insights, failed, reads } = useInsights();
  const fallback = useUsageStatsFallback(failed && !insights, reads);
  const stats = insights?.dictation ?? (fallback === "error" ? null : fallback);
  // Without Insights, the streak is the dictation one.
  const streak =
    insights?.current_streak_days ?? stats?.current_streak_days ?? 0;

  if (!stats) {
    // Nothing could be read: leave the line out rather than load forever.
    if (fallback === "error") return null;
    return (
      <div
        aria-hidden="true"
        className="h-11 animate-pulse rounded-xl border border-hairline bg-surface-muted"
      />
    );
  }

  if (stats.total_words === 0) {
    return (
      <p className="rounded-xl border border-dashed border-hairline-strong px-4 py-2.5 text-sm text-muted">
        {t("home.stats.empty")}
      </p>
    );
  }

  const wpm = wordsPerMinute(stats);

  return (
    <button
      type="button"
      onClick={() => navigate("insights")}
      aria-label={`${t("home.stats.label")}. ${t("home.stats.seeInsights")}`}
      className="group flex w-full cursor-pointer flex-wrap items-center gap-x-6 gap-y-2 rounded-xl border border-hairline bg-surface px-4 py-2.5 text-start elev-card elev-hover transition-colors hover:border-hairline-strong focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
    >
      <Stat
        icon={Type}
        tint="bg-accent/10 text-accent"
        value={formatCount(stats.total_words, i18n.language)}
        label={t("home.stats.words")}
      />
      <Stat
        icon={Gauge}
        tint="bg-sky-500/10 text-sky-600 dark:text-sky-400"
        value={wpm !== null ? String(wpm) : t("home.stats.none")}
        label={t("home.stats.wpm")}
        hint={t("home.stats.wpmHint")}
      />
      <Stat
        icon={Clock}
        tint="bg-violet-500/10 text-violet-600 dark:text-violet-400"
        value={formatDuration(timeSavedSeconds(stats), t)}
        label={t("home.stats.saved")}
        hint={t("home.stats.savedHint", { wpm: TYPING_WPM })}
      />
      {/* The streak Insights shows: a day counts when anything was said to
          the app, not only a dictation. */}
      <Stat
        icon={Flame}
        tint="bg-amber-500/10 text-amber-600 dark:text-amber-400"
        value={String(streak)}
        label={t("home.stats.streakUnit", {
          count: streak,
        })}
      />
      <span className="ms-auto inline-flex items-center gap-1 text-[0.8125rem] font-medium text-muted transition-colors group-hover:text-ink">
        {t("home.stats.seeInsights")}
        <ArrowRight
          className="h-3.5 w-3.5 transition-transform group-hover:translate-x-0.5 rtl:rotate-180"
          aria-hidden="true"
        />
      </span>
    </button>
  );
};
