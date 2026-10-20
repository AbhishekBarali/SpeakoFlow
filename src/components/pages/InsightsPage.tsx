import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, MessageCircle, Share2, Users } from "lucide-react";
import type { Insights } from "@/bindings";
import { Page, PageHeader, TextLink } from "@/components/ui/Page";
import { Button } from "@/components/ui/Button";
import { InfoTip } from "@/components/ui/InfoTip";
import { Segmented } from "@/components/ui/Segmented";
import { useNavigation } from "@/components/shell/navigation";
import { useSettings } from "@/hooks/useSettings";
import {
  formatCount,
  formatDuration,
  formatShortDuration,
  TYPING_WPM,
  timeSavedSeconds,
  wordsPerMinute,
} from "./home/usageMath";
import {
  activitySeries,
  assistantQuestions,
  averageActiveDay,
  bestDay,
  buildHeatmap,
  busiestWeekday,
  hasActivity,
  pagesFor,
  parseDayKey,
  type Activity,
  type HeatCell,
  type HeatLevel,
} from "./insights/insightsMath";
import { useInsights } from "./insights/useInsights";
import { ShareDialog } from "./insights/ShareDialog";

/** Weeks in the activity grid: about six months. */
const WEEKS = 26;

const ACTIVITIES: Activity[] = ["dictation", "assistant", "meetings"];

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
 * How one activity's day is said: words dictated, questions asked, or time in
 * meetings.
 */
const useDayValue = () => {
  const { t, i18n } = useTranslation();
  return (activity: Activity, value: number): string => {
    const formatted = formatCount(value, i18n.language);
    switch (activity) {
      case "dictation":
        return t("insights.dayWords", { count: value, formatted });
      case "assistant":
        return t("insights.dayQuestions", { count: value, formatted });
      case "meetings":
        return formatDuration(value, t);
    }
  };
};

/**
 * Six months, one square per day, darker for more use. Dictation by default;
 * the assistant and meetings are a switch away once they have been used. The
 * streak above it counts a day when anything was said to the app.
 */
const ActivityGrid: React.FC<{
  insights: Insights;
  activity: Activity;
  activities: Activity[];
  onActivityChange: (activity: Activity) => void;
}> = ({ insights, activity, activities, onActivityChange }) => {
  const { t, i18n } = useTranslation();
  const dayValue = useDayValue();
  const [hovered, setHovered] = useState<HeatCell | null>(null);
  const map = useMemo(
    () => buildHeatmap(activitySeries(insights, activity), WEEKS),
    [insights, activity],
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

  const summary = (() => {
    switch (activity) {
      case "dictation":
        return t("insights.gridSummary", {
          words: formatCount(map.total, i18n.language),
          count: map.activeDays,
        });
      case "assistant":
        return t("insights.gridSummaryAssistant", {
          questions: dayValue("assistant", map.total),
          count: map.activeDays,
        });
      case "meetings":
        return t("insights.gridSummaryMeetings", {
          time: formatDuration(map.total, t),
          count: map.activeDays,
        });
    }
  })();
  const noneLabel = t(`insights.dayNoneFor.${activity}`);

  return (
    <section className="rounded-2xl border border-hairline bg-surface p-5 elev-card">
      <div className="mb-4 flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-0.5">
          <h2 className="flex items-center gap-1.5 text-[1.0625rem] font-semibold text-ink">
            {t("insights.streak", { count: insights.current_streak_days })}
            <InfoTip text={t("insights.streakHint")} />
          </h2>
          <p className="text-xs text-muted">
            {t("insights.longest", { count: insights.longest_streak_days })}
          </p>
        </div>
        {activities.length > 1 && (
          <Segmented<Activity>
            size="sm"
            label={t("insights.activity.label")}
            value={activity}
            onChange={(next) => {
              setHovered(null);
              onActivityChange(next);
            }}
            options={activities.map((value) => ({
              value,
              label: t(`insights.activity.${value}`),
            }))}
          />
        )}
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
            aria-label={summary}
            onMouseLeave={() => setHovered(null)}
            className="grid grid-flow-col grid-rows-7 gap-1"
            style={{
              gridTemplateColumns: `repeat(${WEEKS}, minmax(0, 1fr))`,
            }}
          >
            {map.weeks.map((column, col) =>
              column.map((cell, row) => (
                <div
                  key={`${activity}-${cell.key}`}
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
              {hovered.value > 0
                ? dayValue(activity, hovered.value)
                : noneLabel}
            </>
          ) : (
            summary
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

/** The busiest weekday, best day and typical day of the activity in view. */
const Highlights: React.FC<{ insights: Insights; activity: Activity }> = ({
  insights,
  activity,
}) => {
  const { t, i18n } = useTranslation();
  const dayValue = useDayValue();
  const highlights = useMemo(() => {
    const days = activitySeries(insights, activity);
    return {
      weekday: busiestWeekday(days),
      best: bestDay(days),
      average: averageActiveDay(days),
    };
  }, [insights, activity]);

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
            ? dayValue(activity, highlights.best.value)
            : t("home.stats.none")
        }
        sub={highlights.best ? longDate(highlights.best.day) : undefined}
      />
      <Highlight
        label={t("insights.typical")}
        value={
          highlights.average > 0
            ? dayValue(activity, highlights.average)
            : t("home.stats.none")
        }
        sub={t(`insights.typicalFor.${activity}`)}
      />
    </div>
  );
};

/**
 * One feature beyond dictation: its name, the way to its page, and three
 * figures side by side. Before it has been used it says how to start instead
 * of showing a row of zeros.
 */
const FeatureCard: React.FC<{
  icon: React.ComponentType<{ className?: string }>;
  title: string;
  info: string;
  open: { label: string; onClick: () => void };
  figures: Array<{ value: string; label: string }>;
  empty: string | null;
}> = ({ icon: Icon, title, info, open, figures, empty }) => (
  <section className="flex min-w-0 flex-col rounded-2xl border border-hairline bg-surface px-5 py-4 elev-card">
    <div className="flex items-center justify-between gap-3">
      <div className="flex min-w-0 items-center gap-2.5">
        <span
          aria-hidden="true"
          className="grid h-7 w-7 shrink-0 place-items-center rounded-lg bg-surface-strong text-ink/70"
        >
          <Icon className="h-4 w-4" />
        </span>
        <h2 className="truncate text-[0.9375rem] font-semibold text-ink">
          {title}
        </h2>
        <InfoTip text={info} />
      </div>
      <TextLink onClick={open.onClick} className="-me-1.5 shrink-0">
        {open.label}
        <ArrowRight className="h-3.5 w-3.5 rtl:rotate-180" aria-hidden="true" />
      </TextLink>
    </div>
    {empty ? (
      <p className="mt-4 text-sm text-muted text-pretty">{empty}</p>
    ) : (
      <dl className="mt-4 grid grid-cols-3 divide-x divide-hairline rtl:divide-x-reverse">
        {figures.map((figure) => (
          <div
            key={figure.label}
            className="flex min-w-0 flex-col-reverse px-3 first:ps-0 last:pe-0 @4xl:px-4"
          >
            <dt className="mt-1 truncate text-xs text-muted">{figure.label}</dt>
            <dd className="truncate text-[1.3125rem] leading-tight font-semibold tracking-[-0.02em] text-ink tabular-nums">
              {figure.value}
            </dd>
          </div>
        ))}
      </dl>
    )}
  </section>
);

/**
 * Insights: how much you have said to SpeakoFlow, how fast, and when, across
 * dictation, the assistant and meetings — and a card to share it. Everything
 * is counted on this computer.
 */
export const InsightsPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { navigate } = useNavigation();
  const { settings } = useSettings();
  const insights = useInsights();
  const [activity, setActivity] = useState<Activity>("dictation");
  const [sharing, setSharing] = useState(false);

  const used = useMemo(
    () =>
      insights
        ? ACTIVITIES.filter((value) => hasActivity(insights, value))
        : [],
    [insights],
  );
  // The grid shows dictation unless it was never used; a choice of something
  // with nothing to show falls back too.
  const shown: Activity = used.includes(activity)
    ? activity
    : (used[0] ?? "dictation");

  const header = (
    <PageHeader
      title={t("nav.insights")}
      description={t("insights.about")}
      actions={
        <Button
          onClick={() => setSharing(true)}
          disabled={used.length === 0}
          title={used.length === 0 ? t("insights.share.nothingYet") : undefined}
        >
          <Share2 className="h-4 w-4" aria-hidden="true" />
          {t("insights.share.button")}
        </Button>
      }
    />
  );

  if (!insights) {
    return (
      <Page>
        {header}
        <div
          aria-hidden="true"
          className="h-[26rem] animate-pulse rounded-2xl border border-hairline bg-surface-muted"
        />
      </Page>
    );
  }

  if (used.length === 0) {
    return (
      <Page>
        {header}
        <p className="rounded-2xl border border-dashed border-hairline-strong px-5 py-8 text-center text-sm text-muted">
          {t("insights.nothingYet")}
        </p>
      </Page>
    );
  }

  const { dictation, assistant, meetings } = insights;
  const wpm = wordsPerMinute(dictation);
  const questions = assistantQuestions(insights);
  const assistantOn = settings?.assistant_enabled ?? true;
  const showAssistant = assistantOn || used.includes("assistant");

  return (
    <Page>
      {header}
      <div className="space-y-4">
        {used.includes("dictation") && (
          <div className="grid grid-cols-2 gap-3 @3xl:grid-cols-4">
            <StatCard
              label={t("home.stats.wordsLabel")}
              value={formatCount(dictation.total_words, i18n.language)}
              sub={t("insights.pages", {
                count: pagesFor(dictation.total_words),
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
              value={formatDuration(timeSavedSeconds(dictation), t)}
              info={t("home.stats.savedHint", { wpm: TYPING_WPM })}
            />
            <StatCard
              label={t("insights.dictations")}
              value={formatCount(dictation.total_dictations, i18n.language)}
              sub={t("insights.today", {
                words: formatCount(dictation.today_words, i18n.language),
              })}
            />
          </div>
        )}

        <div
          className={`grid grid-cols-1 gap-3 ${showAssistant ? "@3xl:grid-cols-2" : ""}`}
        >
          {showAssistant && (
            <FeatureCard
              icon={MessageCircle}
              title={t("insights.activity.assistant")}
              info={t("insights.assistantCard.info")}
              open={{
                label: t("insights.open"),
                onClick: () => navigate("assistant"),
              }}
              empty={
                used.includes("assistant")
                  ? null
                  : t("insights.assistantCard.empty")
              }
              figures={[
                {
                  value: formatCount(questions, i18n.language),
                  label: t("insights.assistantCard.questions", {
                    count: questions,
                  }),
                },
                {
                  value: formatCount(assistant.calls, i18n.language),
                  label: t("insights.assistantCard.calls", {
                    count: assistant.calls,
                  }),
                },
                {
                  value: formatShortDuration(assistant.call_seconds, t),
                  label: t("insights.assistantCard.callTime"),
                },
              ]}
            />
          )}
          <FeatureCard
            icon={Users}
            title={t("insights.activity.meetings")}
            info={t("insights.meetingsCard.info")}
            open={{
              label: t("insights.open"),
              onClick: () => navigate("meetings"),
            }}
            empty={
              used.includes("meetings")
                ? null
                : t("insights.meetingsCard.empty")
            }
            figures={[
              {
                value: formatCount(meetings.meetings, i18n.language),
                label: t("insights.meetingsCard.meetings", {
                  count: meetings.meetings,
                }),
              },
              {
                value: formatShortDuration(meetings.seconds, t),
                label: t("insights.meetingsCard.time"),
              },
              {
                value: formatCount(meetings.words, i18n.language),
                label: t("insights.meetingsCard.words", {
                  count: meetings.words,
                }),
              },
            ]}
          />
        </div>

        <ActivityGrid
          insights={insights}
          activity={shown}
          activities={used}
          onActivityChange={setActivity}
        />

        <Highlights insights={insights} activity={shown} />
      </div>

      <ShareDialog
        open={sharing}
        onClose={() => setSharing(false)}
        insights={insights}
      />
    </Page>
  );
};
