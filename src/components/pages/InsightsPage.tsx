import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Share2 } from "lucide-react";
import type { Insights } from "@/bindings";
import { Page, PageHeader } from "@/components/ui/Page";
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
  dayParts,
  FEATURES,
  hasActivity,
  pagesFor,
  parseDayKey,
  type Activity,
  type DayValue,
  type HeatCell,
  type HeatLevel,
} from "./insights/insightsMath";
import { NOVEL_WORDS } from "./insights/shareContent";
import { useInsights } from "./insights/useInsights";
import { ShareDialog } from "./insights/ShareDialog";

/** Weeks in the activity grid: about six months. */
const WEEKS = 26;

const LEVEL_CLASSES: Record<HeatLevel, string> = {
  0: "bg-ink/[0.06]",
  1: "bg-accent/25",
  2: "bg-accent/45",
  3: "bg-accent/70",
  4: "bg-accent",
};

/**
 * One number: the figure, what it counts in small capitals, and under a
 * hairline one line that puts it in context. Every card on the page is this,
 * so the page reads as one set rather than a wall of different boxes.
 */
const StatCard: React.FC<{
  value?: string;
  label: string;
  info?: string;
  /** Shown instead of a figure before the feature has been used. */
  empty?: string;
  className?: string;
  children?: React.ReactNode;
}> = ({ value, label, info, empty, className = "", children }) => (
  <section
    className={`flex min-w-0 flex-col rounded-2xl border border-hairline bg-surface px-6 pt-5 pb-5 elev-card ${className}`}
  >
    {empty ? (
      <>
        <h2 className="flex items-center gap-1 text-[0.6875rem] font-semibold tracking-[0.12em] text-muted uppercase">
          {label}
          {info && <InfoTip text={info} />}
        </h2>
        <p className="mt-3 text-sm text-muted text-pretty">{empty}</p>
      </>
    ) : (
      <>
        <p
          className="truncate text-[2rem] leading-none font-semibold tracking-[-0.025em] text-ink tabular-nums"
          title={value}
        >
          {value}
        </p>
        <h2 className="mt-2.5 flex items-center gap-1 text-[0.6875rem] font-semibold tracking-[0.12em] text-muted uppercase">
          <span className="truncate">{label}</span>
          {info && <InfoTip text={info} />}
        </h2>
        {children && (
          <div className="mt-4 border-t border-hairline pt-3.5 text-[0.8125rem] text-ink/80 text-pretty">
            {children}
          </div>
        )}
      </>
    )}
  </section>
);

/** Speaking against typing, as two bars. */
const SpeedBars: React.FC<{ wpm: number }> = ({ wpm }) => {
  const { t } = useTranslation();
  const max = Math.max(wpm, TYPING_WPM);
  const rows = [
    {
      label: t("insights.share.cards.speed.you"),
      value: wpm,
      bar: "bg-accent",
    },
    {
      label: t("insights.share.cards.speed.typing"),
      value: TYPING_WPM,
      // A typical typist, not the user: a reference, drawn as an outline.
      bar: "border border-dashed border-ink/30",
    },
  ];
  return (
    <div className="space-y-1.5" aria-hidden="true">
      {rows.map((row) => (
        <div key={row.label} className="flex items-center gap-2.5">
          <span className="w-24 shrink-0 truncate text-xs text-muted">
            {row.label}
          </span>
          <span className="h-2 flex-1 overflow-hidden rounded-full">
            <span
              className={`block h-full rounded-full ${row.bar}`}
              style={{ width: `${(row.value / max) * 100}%` }}
            />
          </span>
          <span className="w-8 shrink-0 text-end text-xs text-muted tabular-nums">
            {row.value}
          </span>
        </div>
      ))}
    </div>
  );
};

/**
 * How a day of an activity is said: words dictated, questions asked, time in
 * meetings, or for all of them each of those the day held, in that order.
 */
const useDaySaying = (insights: Insights) => {
  const { t, i18n } = useTranslation();
  const parts = useMemo(() => dayParts(insights), [insights]);
  const words = (value: number) =>
    t("insights.dayWords", {
      count: value,
      formatted: formatCount(value, i18n.language),
    });
  const questions = (value: number) =>
    t("insights.dayQuestions", {
      count: value,
      formatted: formatCount(value, i18n.language),
    });
  return (activity: Activity, day: DayValue): string => {
    switch (activity) {
      case "dictation":
        return words(day.value);
      case "assistant":
        return questions(day.value);
      case "meetings":
        return formatDuration(day.value, t);
      case "all": {
        const held = parts.get(day.day);
        if (!held) return "";
        return [
          held.words > 0 ? words(held.words) : null,
          held.questions > 0 ? questions(held.questions) : null,
          held.meetingSeconds > 0
            ? t("insights.dayMeetings", {
                time: formatDuration(held.meetingSeconds, t),
              })
            : null,
        ]
          .filter(Boolean)
          .join(" · ");
      }
    }
  };
};

/**
 * The busiest weekday, best day and typical day of the activity in view, as
 * a short list beside the grid they are read from. For all of them, the best
 * day says what it held and there is no typical day, since an average of a
 * mixed score means nothing.
 */
const Highlights: React.FC<{ insights: Insights; activity: Activity }> = ({
  insights,
  activity,
}) => {
  const { t, i18n } = useTranslation();
  const say = useDaySaying(insights);
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

  const none = t("home.stats.none");
  const rows: Array<{ label: string; value: string; sub?: string }> = [
    {
      label: t("insights.busiest"),
      value:
        highlights.weekday !== null ? weekdayName(highlights.weekday) : none,
    },
    activity === "all"
      ? {
          label: t("insights.best"),
          value: highlights.best ? longDate(highlights.best.day) : none,
          sub: highlights.best ? say("all", highlights.best) : undefined,
        }
      : {
          label: t("insights.best"),
          value: highlights.best ? say(activity, highlights.best) : none,
          sub: highlights.best ? longDate(highlights.best.day) : undefined,
        },
  ];
  if (activity !== "all") {
    rows.push({
      label: t("insights.typical"),
      value:
        highlights.average > 0
          ? say(activity, { day: "", value: highlights.average })
          : none,
      sub: t(`insights.typicalFor.${activity}`),
    });
  }
  return (
    <dl className="grid content-start gap-x-6 gap-y-5 @xl:grid-cols-3 @3xl:grid-cols-1">
      {rows.map((row) => (
        <div key={row.label} className="min-w-0">
          <dt className="text-[0.6875rem] font-semibold tracking-[0.12em] text-muted uppercase">
            {row.label}
          </dt>
          <dd className="mt-1 truncate text-[0.9375rem] font-semibold text-ink">
            {row.value}
          </dd>
          {row.sub && (
            <dd className="mt-0.5 truncate text-xs text-muted">{row.sub}</dd>
          )}
        </div>
      ))}
    </dl>
  );
};

/**
 * Six months, one square per day, darker for more use, with the streak over
 * it and the day's highlights beside it. Once more than one feature has been
 * used it opens on all of them, the days the streak counts, with each
 * feature a switch away.
 */
const ActivityCard: React.FC<{
  insights: Insights;
  activity: Activity;
  activities: Activity[];
  onActivityChange: (activity: Activity) => void;
}> = ({ insights, activity, activities, onActivityChange }) => {
  const { t, i18n } = useTranslation();
  const say = useDaySaying(insights);
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
          questions: say("assistant", { day: "", value: map.total }),
          count: map.activeDays,
        });
      case "meetings":
        return t("insights.gridSummaryMeetings", {
          time: formatDuration(map.total, t),
          count: map.activeDays,
        });
      case "all":
        return t("insights.gridSummaryAll", { count: map.activeDays });
    }
  })();
  const noneLabel = t(`insights.dayNoneFor.${activity}`);

  return (
    <section className="@container rounded-2xl border border-hairline bg-surface px-6 pt-5 pb-6 elev-card">
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-3">
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

      <div className="mt-6 grid gap-x-10 gap-y-6 @3xl:grid-cols-[auto_minmax(0,1fr)]">
        <div className="min-w-0 overflow-x-auto">
          <div className="inline-flex gap-2">
            {/* Weekday labels share the grid's rows, so they line up. */}
            <div
              aria-hidden="true"
              className="grid shrink-0 grid-rows-[repeat(7,1rem)] gap-[3px] pt-5 text-[0.6875rem] text-muted"
            >
              {rowLabels.map((label, row) => (
                <span key={row} className="flex h-4 items-center leading-none">
                  {label}
                </span>
              ))}
            </div>
            <div>
              <div
                aria-hidden="true"
                className="relative mb-1.5 h-3.5 text-[0.6875rem] leading-none text-muted"
              >
                {map.months.map(({ col, date }) => (
                  <span
                    key={`${col}-${date.getMonth()}`}
                    className="absolute top-0"
                    style={{ insetInlineStart: `${col * 19}px` }}
                  >
                    {monthFormat.format(date)}
                  </span>
                ))}
              </div>
              <div
                role="img"
                aria-label={summary}
                onMouseLeave={() => setHovered(null)}
                className="grid grid-flow-col grid-rows-[repeat(7,1rem)] gap-[3px]"
                style={{ gridTemplateColumns: `repeat(${WEEKS}, 1rem)` }}
              >
                {map.weeks.map((column, col) =>
                  column.map((cell, row) => (
                    <div
                      key={`${activity}-${cell.key}`}
                      onMouseEnter={() => setHovered(cell.future ? null : cell)}
                      className={`heat-cell h-4 w-4 rounded-[4px] transition-[box-shadow] duration-150 ${
                        cell.future
                          ? "bg-transparent"
                          : `${LEVEL_CLASSES[cell.level]} hover:shadow-[0_0_0_1.5px_var(--color-ink)]`
                      } ${cell.isToday ? "shadow-[0_0_0_1.5px_var(--color-accent)]" : ""}`}
                      style={
                        { "--col": col, "--row": row } as React.CSSProperties
                      }
                    />
                  )),
                )}
              </div>
              <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
                <p className="min-h-[1.25rem] text-xs text-muted tabular-nums">
                  {hovered ? (
                    <>
                      <span className="font-medium text-ink">
                        {dayFormat.format(hovered.date)}
                      </span>
                      {" · "}
                      {hovered.value > 0
                        ? say(activity, {
                            day: hovered.key,
                            value: hovered.value,
                          })
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
                      className={`h-2.5 w-2.5 rounded-[3px] ${LEVEL_CLASSES[level]}`}
                    />
                  ))}
                  <span className="ms-1">{t("insights.more")}</span>
                </div>
              </div>
            </div>
          </div>
        </div>
        <div className="border-t border-hairline pt-5 @3xl:border-t-0 @3xl:border-s @3xl:ps-10 @3xl:pt-1">
          <Highlights insights={insights} activity={activity} />
        </div>
      </div>
    </section>
  );
};

/**
 * Insights: how much you have said to SpeakoFlow, how fast, and when, across
 * dictation, the assistant and meetings, and cards to share it. Everything
 * is counted on this computer.
 */
export const InsightsPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { navigate } = useNavigation();
  const { settings } = useSettings();
  const insights = useInsights();
  const [activity, setActivity] = useState<Activity>("all");
  const [sharing, setSharing] = useState(false);

  const used = useMemo(
    () =>
      insights
        ? FEATURES.filter((feature) => hasActivity(insights, feature))
        : [],
    [insights],
  );
  // All of them, then each feature that has been used. With only one used,
  // "all" would be that feature twice, so the grid just shows it.
  const views: Activity[] = used.length > 1 ? ["all", ...used] : used;
  const shown: Activity = views.includes(activity)
    ? activity
    : (views[0] ?? "dictation");

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
        <div className="flex flex-col items-center gap-3 rounded-2xl border border-dashed border-hairline-strong px-5 py-10 text-center">
          <p className="max-w-md text-sm text-muted text-pretty">
            {t("insights.nothingYet")}
          </p>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => navigate("home")}
          >
            {t("insights.goHome")}
            <ArrowRight
              className="h-3.5 w-3.5 rtl:rotate-180"
              aria-hidden="true"
            />
          </Button>
        </div>
      </Page>
    );
  }

  const { dictation, assistant, meetings } = insights;
  const lang = i18n.language;
  const wpm = wordsPerMinute(dictation);
  const questions = assistantQuestions(insights);
  const assistantOn = settings?.assistant_enabled ?? true;
  const showAssistant = assistantOn || used.includes("assistant");
  const words = dictation.total_words;

  return (
    <Page>
      {header}
      <div className="space-y-4">
        {used.includes("dictation") && (
          <div className="grid gap-4 @lg:grid-cols-2 @3xl:grid-cols-3">
            <StatCard
              value={formatCount(words, lang)}
              label={t("home.stats.wordsLabel")}
            >
              {words >= NOVEL_WORDS
                ? t("insights.share.cards.words.books", {
                    count: Math.floor(words / NOVEL_WORDS),
                  })
                : t("insights.pages", { count: pagesFor(words) })}
            </StatCard>
            <StatCard
              value={wpm !== null ? String(wpm) : t("home.stats.none")}
              label={t("home.stats.wpmLabel")}
              info={t("home.stats.wpmHint")}
            >
              {wpm !== null ? <SpeedBars wpm={wpm} /> : t("home.stats.none")}
            </StatCard>
            <StatCard
              value={formatDuration(timeSavedSeconds(dictation), t)}
              label={t("home.stats.savedLabel")}
              info={t("home.stats.savedHint", { wpm: TYPING_WPM })}
              className="@lg:col-span-2 @3xl:col-span-1"
            >
              {t("insights.dictationsSoFar", {
                count: dictation.total_dictations,
                formatted: formatCount(dictation.total_dictations, lang),
              })}
            </StatCard>
          </div>
        )}

        <div className={`grid gap-4 ${showAssistant ? "@lg:grid-cols-2" : ""}`}>
          {showAssistant && (
            <StatCard
              value={formatCount(questions, lang)}
              label={t("insights.share.cards.assistant.label", {
                count: questions,
              })}
              info={t("insights.assistantCard.info")}
              empty={
                used.includes("assistant")
                  ? undefined
                  : t("insights.assistantCard.empty")
              }
            >
              {assistant.calls > 0
                ? t("insights.share.cards.assistant.calls", {
                    count: assistant.calls,
                    time: formatShortDuration(assistant.call_seconds, t),
                  })
                : t("insights.assistantCard.noCalls")}
            </StatCard>
          )}
          <StatCard
            value={formatCount(meetings.meetings, lang)}
            label={t("insights.share.cards.meetings.label", {
              count: meetings.meetings,
            })}
            info={t("insights.meetingsCard.info")}
            empty={
              used.includes("meetings")
                ? undefined
                : t("insights.meetingsCard.empty")
            }
          >
            {meetings.words > 0
              ? t("insights.share.cards.meetings.note", {
                  time: formatShortDuration(meetings.seconds, t),
                  words: formatCount(meetings.words, lang),
                })
              : t("insights.share.cards.meetings.time", {
                  time: formatShortDuration(meetings.seconds, t),
                })}
          </StatCard>
        </div>

        <ActivityCard
          insights={insights}
          activity={shown}
          activities={views}
          onActivityChange={setActivity}
        />
      </div>

      <ShareDialog
        open={sharing}
        onClose={() => setSharing(false)}
        insights={insights}
      />
    </Page>
  );
};
