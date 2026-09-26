import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { UsageStats } from "@/bindings";
import { Page, PageHeader } from "@/components/ui/Page";
import { InfoTip } from "@/components/ui/InfoTip";
import { useUsageStats } from "./home/StatsStrip";
import {
  formatCount,
  formatDuration,
  TYPING_WPM,
  timeSavedSeconds,
  wordsPerMinute,
} from "./home/usageMath";
import {
  averageActiveDay,
  bestDay,
  buildHeatmap,
  busiestWeekday,
  pagesFor,
  parseDayKey,
  type HeatCell,
  type HeatLevel,
} from "./insights/insightsMath";

/** Weeks in the activity grid: about six months. */
const WEEKS = 26;

const LEVEL_CLASSES: Record<HeatLevel, string> = {
  0: "bg-ink/[0.06]",
  1: "bg-accent/25",
  2: "bg-accent/45",
  3: "bg-accent/70",
  4: "bg-accent",
};

const StatCard: React.FC<{
  label: string;
  value: string;
  sub?: string;
  info?: string;
}> = ({ label, value, sub, info }) => (
  <div className="min-w-0 rounded-2xl border border-hairline bg-surface px-5 py-4 elev-card">
    <p className="flex items-center gap-1 text-[0.8125rem] text-muted">
      <span className="truncate">{label}</span>
      {info && <InfoTip text={info} />}
    </p>
    <p className="mt-2 truncate text-[1.75rem] leading-none font-semibold tracking-[-0.02em] text-ink tabular-nums">
      {value}
    </p>
    {sub && <p className="mt-2 truncate text-xs text-muted">{sub}</p>}
  </div>
);

/**
 * Six months of dictation, one square per day, darker for more words. The
 * grid sweeps in when the page opens; hovering a day names it and its count in
 * the line under the grid rather than in a floating tooltip.
 */
const ActivityGrid: React.FC<{ stats: UsageStats }> = ({ stats }) => {
  const { t, i18n } = useTranslation();
  const [hovered, setHovered] = useState<HeatCell | null>(null);
  const map = useMemo(
    () => buildHeatmap(stats.recent_days ?? [], WEEKS),
    [stats.recent_days],
  );
  const monthFormat = useMemo(
    () => new Intl.DateTimeFormat(i18n.language, { month: "short" }),
    [i18n.language],
  );
  const dayFormat = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.language, {
        weekday: "short",
        month: "short",
        day: "numeric",
      }),
    [i18n.language],
  );
  const weekdayFormat = useMemo(
    () => new Intl.DateTimeFormat(i18n.language, { weekday: "short" }),
    [i18n.language],
  );
  // Mon, Wed, Fri, like every activity grid people already know.
  const rowLabels = map.weeks[0].map((cell, row) =>
    row % 2 === 1 ? weekdayFormat.format(cell.date) : "",
  );

  return (
    <section className="rounded-2xl border border-hairline bg-surface p-5 elev-card">
      <div className="mb-4 flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <h2 className="text-[1.0625rem] font-semibold text-ink">
          {t("insights.streak", { count: stats.current_streak_days })}
        </h2>
        <p className="text-xs text-muted">
          {t("insights.longest", { count: stats.longest_streak_days })}
        </p>
      </div>

      <div className="flex gap-2">
        {/* Weekday labels share the grid's rows, so they line up at any width. */}
        <div
          aria-hidden="true"
          className="grid shrink-0 grid-rows-7 gap-1 pt-5 text-[0.6875rem] text-muted"
        >
          {rowLabels.map((label, row) => (
            <span key={row} className="flex items-center leading-none">
              {label}
            </span>
          ))}
        </div>
        <div className="min-w-0 flex-1">
          <div
            aria-hidden="true"
            className="relative mb-1.5 h-3.5 text-[0.6875rem] leading-none text-muted"
          >
            {map.months.map(({ col, date }) => (
              <span
                key={`${col}-${date.getMonth()}`}
                className="absolute top-0"
                style={{ left: `${(col / WEEKS) * 100}%` }}
              >
                {monthFormat.format(date)}
              </span>
            ))}
          </div>
          <div
            role="img"
            aria-label={t("insights.gridLabel", {
              words: formatCount(map.total, i18n.language),
              count: map.activeDays,
            })}
            onMouseLeave={() => setHovered(null)}
            className="grid grid-flow-col grid-rows-7 gap-1"
            style={{
              gridTemplateColumns: `repeat(${WEEKS}, minmax(0, 1fr))`,
            }}
          >
            {map.weeks.map((column, col) =>
              column.map((cell, row) => (
                <div
                  key={cell.key}
                  onMouseEnter={() => setHovered(cell.future ? null : cell)}
                  className={`heat-cell aspect-square rounded-[4px] transition-[box-shadow] duration-150 ${
                    cell.future
                      ? "bg-transparent"
                      : `${LEVEL_CLASSES[cell.level]} hover:shadow-[0_0_0_1.5px_var(--color-ink)]`
                  } ${cell.isToday ? "shadow-[0_0_0_1.5px_var(--color-accent)]" : ""}`}
                  style={{ "--col": col, "--row": row } as React.CSSProperties}
                />
              )),
            )}
          </div>
        </div>
      </div>

      <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
        <p className="min-h-[1.25rem] text-[0.8125rem] text-muted tabular-nums">
          {hovered ? (
            <>
              <span className="font-medium text-ink">
                {dayFormat.format(hovered.date)}
              </span>
              {" · "}
              {hovered.words > 0
                ? t("insights.dayWords", {
                    count: hovered.words,
                    formatted: formatCount(hovered.words, i18n.language),
                  })
                : t("insights.dayNone")}
            </>
          ) : (
            t("insights.gridSummary", {
              words: formatCount(map.total, i18n.language),
              count: map.activeDays,
            })
          )}
        </p>
        <div
          aria-hidden="true"
          className="flex items-center gap-1 text-[0.6875rem] text-muted"
        >
          <span className="me-1">{t("insights.less")}</span>
          {([0, 1, 2, 3, 4] as HeatLevel[]).map((level) => (
            <span
              key={level}
              className={`h-3 w-3 rounded-[3px] ${LEVEL_CLASSES[level]}`}
            />
          ))}
          <span className="ms-1">{t("insights.more")}</span>
        </div>
      </div>
    </section>
  );
};

const Highlight: React.FC<{ label: string; value: string; sub?: string }> = ({
  label,
  value,
  sub,
}) => (
  <div className="min-w-0 rounded-2xl border border-hairline bg-surface px-5 py-4 elev-card">
    <p className="truncate text-[0.8125rem] text-muted">{label}</p>
    <p className="mt-1.5 truncate text-[1.0625rem] font-semibold text-ink">
      {value}
    </p>
    {sub && <p className="mt-0.5 truncate text-xs text-muted">{sub}</p>}
  </div>
);

/**
 * Insights: how much you have said, how fast, and when — the numbers Home
 * only glances at. Everything is counted on this computer.
 */
export const InsightsPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const stats = useUsageStats();
  const wpm = stats ? wordsPerMinute(stats) : null;

  const highlights = useMemo(() => {
    const days = stats?.recent_days ?? [];
    return {
      weekday: busiestWeekday(days),
      best: bestDay(days),
      average: averageActiveDay(days),
    };
  }, [stats?.recent_days]);

  const weekdayName = (index: number) =>
    new Intl.DateTimeFormat(i18n.language, { weekday: "long" }).format(
      // 4 Jan 1970 was a Sunday.
      new Date(1970, 0, 4 + index),
    );
  const longDate = (key: string) => {
    const date = parseDayKey(key);
    return date
      ? new Intl.DateTimeFormat(i18n.language, {
          month: "short",
          day: "numeric",
          year: "numeric",
        }).format(date)
      : key;
  };

  return (
    <Page>
      <PageHeader
        title={t("nav.insights")}
        description={t("insights.description")}
      />

      {!stats ? (
        <div
          aria-hidden="true"
          className="h-[26rem] animate-pulse rounded-2xl border border-hairline bg-surface-muted"
        />
      ) : stats.total_words === 0 ? (
        <p className="rounded-2xl border border-dashed border-hairline-strong px-5 py-8 text-center text-sm text-muted">
          {t("insights.empty")}
        </p>
      ) : (
        <div className="space-y-4">
          <div className="grid grid-cols-2 gap-3 @3xl:grid-cols-4">
            <StatCard
              label={t("home.stats.wordsLabel")}
              value={formatCount(stats.total_words, i18n.language)}
              sub={t("insights.pages", {
                count: pagesFor(stats.total_words),
              })}
            />
            <StatCard
              label={t("home.stats.wpmLabel")}
              value={wpm !== null ? String(wpm) : t("home.stats.none")}
              sub={
                wpm !== null && wpm > TYPING_WPM
                  ? t("insights.fasterThanTyping", {
                      times: (wpm / TYPING_WPM).toFixed(1),
                    })
                  : undefined
              }
              info={t("home.stats.wpmHint")}
            />
            <StatCard
              label={t("home.stats.savedLabel")}
              value={formatDuration(timeSavedSeconds(stats), t)}
              info={t("home.stats.savedHint", { wpm: TYPING_WPM })}
            />
            <StatCard
              label={t("insights.dictations")}
              value={formatCount(stats.total_dictations, i18n.language)}
              sub={t("insights.today", {
                words: formatCount(stats.today_words, i18n.language),
              })}
            />
          </div>

          <ActivityGrid stats={stats} />

          <div className="grid grid-cols-1 gap-3 @2xl:grid-cols-3">
            <Highlight
              label={t("insights.busiest")}
              value={
                highlights.weekday !== null
                  ? weekdayName(highlights.weekday)
                  : t("home.stats.none")
              }
            />
            <Highlight
              label={t("insights.best")}
              value={
                highlights.best
                  ? t("insights.dayWords", {
                      count: highlights.best.words,
                      formatted: formatCount(
                        highlights.best.words,
                        i18n.language,
                      ),
                    })
                  : t("home.stats.none")
              }
              sub={highlights.best ? longDate(highlights.best.day) : undefined}
            />
            <Highlight
              label={t("insights.typical")}
              value={t("insights.dayWords", {
                count: highlights.average,
                formatted: formatCount(highlights.average, i18n.language),
              })}
              sub={t("insights.typicalSub")}
            />
          </div>
        </div>
      )}
    </Page>
  );
};
