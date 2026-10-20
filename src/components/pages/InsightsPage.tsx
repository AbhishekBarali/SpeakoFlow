import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Share2 } from "lucide-react";
import type { Insights } from "@/bindings";
import { Page, PageHeader } from "@/components/ui/Page";
import { Button } from "@/components/ui/Button";
import { InfoTip } from "@/components/ui/InfoTip";
import { Segmented } from "@/components/ui/Segmented";
import { useNavigation } from "@/components/shell/navigation";
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
import type { FlowSpec } from "./insights/flowGeometry";
import { FlowArt } from "./insights/FlowArt";
import {
  fullNumber,
  NOVEL_WORDS,
  WORKDAY_SECONDS,
} from "./insights/shareContent";
import { useInsights } from "./insights/useInsights";
import { ShareDialog } from "./insights/ShareDialog";
import "@fontsource-variable/plus-jakarta-sans";

/** Weeks in the activity grid: about six months. */
const WEEKS = 26;

const LEVEL_CLASSES: Record<HeatLevel, string> = {
  0: "bg-ink/[0.055]",
  1: "bg-accent/20",
  2: "bg-accent/40",
  3: "bg-accent/65",
  4: "bg-accent",
};

/** The ribbon behind the summary: the share cards' bands as one wave across
 *  the right half, so the figure on the left stays on a quiet ground. */
const SUMMARY_FLOW: FlowSpec = {
  spine: [
    [0.42, 1.3],
    [0.62, -0.5],
    [0.8, 1.5],
    [1.05, -0.3],
  ],
  edges: [-0.4, -0.13, 0.13, 0.4],
  fan: [1, 1],
};
const SUMMARY_FILLS = [
  "var(--insights-flow-1)",
  "var(--insights-flow-2)",
  "var(--insights-flow-3)",
];

/** The first thing the page says: the one number that sums up the most,
 *  large, over the ribbon the share cards are drawn with. */
const Summary: React.FC<{ label: string; value: string; note?: string }> = ({
  label,
  value,
  note,
}) => (
  <section className="insights-summary relative overflow-hidden rounded-3xl border border-hairline elev-card">
    <FlowArt
      spec={SUMMARY_FLOW}
      width={960}
      height={260}
      fills={SUMMARY_FILLS}
      className="absolute inset-0 h-full w-full"
    />
    <div className="relative px-8 pt-8 pb-9">
      <h2 className="text-sm font-medium text-ink/75">{label}</h2>
      <p className="insights-figure mt-2 text-[3.75rem] leading-none text-ink tabular-nums">
        {value}
      </p>
      {note && <p className="mt-3.5 text-[0.9375rem] text-body">{note}</p>}
    </div>
  </section>
);

interface Figure {
  key: string;
  label: string;
  value: string;
  note?: string;
  info?: string;
}

/**
 * The rest of the numbers, three to a row in one card with hairlines between
 * them: a label, the figure, and one quiet line about it. A row with fewer
 * than three shares its width out, so there is never an empty cell.
 */
const Figures: React.FC<{ figures: Figure[] }> = ({ figures }) => {
  if (figures.length === 0) return null;
  const rows: Figure[][] = [];
  for (let index = 0; index < figures.length; index += 3) {
    rows.push(figures.slice(index, index + 3));
  }
  return (
    <section className="overflow-hidden rounded-2xl border border-hairline bg-hairline elev-card">
      <div className="grid gap-px">
        {rows.map((row) => (
          <div
            key={row.map((figure) => figure.key).join()}
            className="grid gap-px @lg:[grid-template-columns:repeat(var(--cols),minmax(0,1fr))]"
            style={{ "--cols": row.length } as React.CSSProperties}
          >
            {row.map((figure) => (
              <div key={figure.key} className="min-w-0 bg-surface px-6 py-5">
                <h3 className="flex items-center gap-1 text-[0.8125rem] text-muted">
                  <span className="truncate">{figure.label}</span>
                  {figure.info && <InfoTip text={figure.info} />}
                </h3>
                <p
                  className="mt-1.5 truncate text-[1.625rem] leading-tight font-semibold tracking-[-0.02em] text-ink tabular-nums"
                  title={figure.value}
                >
                  {figure.value}
                </p>
                {figure.note && (
                  <p className="mt-1 text-[0.8125rem] text-muted text-pretty">
                    {figure.note}
                  </p>
                )}
              </div>
            ))}
          </div>
        ))}
      </div>
    </section>
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
 * The busiest weekday, best day and typical day of the activity in view, in
 * a row under the grid they are read from. For all of them, the best day
 * says what it held and there is no typical day, since an average of a mixed
 * score means nothing.
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
    <dl
      className="grid gap-x-8 gap-y-4 @lg:[grid-template-columns:repeat(var(--cols),minmax(0,1fr))]"
      style={{ "--cols": rows.length } as React.CSSProperties}
    >
      {rows.map((row) => (
        <div key={row.label} className="min-w-0">
          <dt className="text-[0.8125rem] text-muted">{row.label}</dt>
          <dd className="mt-0.5 truncate text-[0.9375rem] font-semibold text-ink">
            {row.value}
          </dd>
          {row.sub && (
            <dd className="mt-0.5 text-[0.8125rem] text-muted text-pretty">
              {row.sub}
            </dd>
          )}
        </div>
      ))}
    </dl>
  );
};

/**
 * Six months, one square per day, darker for more use, as wide as the card,
 * with the day's highlights under it.
 * Once more than one feature has been used it opens on all of them, the days
 * the streak counts, with each feature a switch away.
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
        <h2 className="flex items-center gap-1.5 text-[0.9375rem] font-semibold text-ink">
          {t("insights.activityTitle")}
          <InfoTip text={t("insights.streakHint")} />
        </h2>
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

      <div className="mt-5">
        <div>
          <div>
            {/* One grid for the weekday labels, the months and the days, so
                they line up. The squares grow to fill the card and the gaps
                between them stay small: spread apart, the weeks stopped
                reading as a calendar, and kept small, the calendar sat in
                half the card with the rest of it empty. In a narrow window
                the squares stop shrinking at 10px and the calendar scrolls
                sideways; the padding keeps the rings on the edge squares
                clear of the scroll box's clip. */}
            <div className="-m-[2px] overflow-x-auto p-[2px]">
              <div
                role="img"
                aria-label={summary}
                onMouseLeave={() => setHovered(null)}
                className="grid items-center gap-[5px]"
                style={{
                  gridTemplateColumns: `auto repeat(${WEEKS}, minmax(10px, 1fr))`,
                  gridTemplateRows: "auto repeat(7, auto)",
                }}
              >
                {map.months.map(({ col, date }) => (
                  <span
                    key={`m-${col}-${date.getMonth()}`}
                    aria-hidden="true"
                    className="pb-1 text-[0.6875rem] leading-none whitespace-nowrap text-muted"
                    style={{ gridRow: 1, gridColumn: `${col + 2} / span 4` }}
                  >
                    {monthFormat.format(date)}
                  </span>
                ))}
                {rowLabels.map((label, row) => (
                  <span
                    key={`d-${row}`}
                    aria-hidden="true"
                    className="pe-2 text-[0.6875rem] leading-none text-muted"
                    style={{ gridRow: row + 2, gridColumn: 1 }}
                  >
                    {label}
                  </span>
                ))}
                {map.weeks.map((column, col) =>
                  column.map((cell, row) => (
                    <div
                      key={`${activity}-${cell.key}`}
                      onMouseEnter={() => setHovered(cell.future ? null : cell)}
                      className={`heat-cell aspect-square w-full rounded-[6px] transition-[box-shadow] duration-150 ${
                        cell.future
                          ? "bg-transparent"
                          : `${LEVEL_CLASSES[cell.level]} hover:shadow-[0_0_0_1.5px_var(--color-ink)]`
                      } ${cell.isToday ? "shadow-[0_0_0_1.5px_var(--color-accent)]" : ""}`}
                      style={
                        {
                          gridRow: row + 2,
                          gridColumn: col + 2,
                          "--col": col,
                          "--row": row,
                        } as React.CSSProperties
                      }
                    />
                  )),
                )}
              </div>
            </div>
            <div className="mt-3 flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
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
        <div className="mt-5 border-t border-hairline pt-5">
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
 *
 * Three things, in the order they are read: the one number that sums it up,
 * the rest of the numbers in a single card, and the calendar. Each feature
 * that has not been used yet is left out rather than shown as a zero.
 */
export const InsightsPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { navigate } = useNavigation();
  const { insights } = useInsights();
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
          className="h-[26rem] animate-pulse rounded-3xl border border-hairline bg-surface-muted"
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
  const words = dictation.total_words;
  const wpm = wordsPerMinute(dictation);
  const saved = timeSavedSeconds(dictation);
  const questions = assistantQuestions(insights);
  const workdays = Math.floor(saved / WORKDAY_SECONDS);

  const callsNote =
    assistant.calls > 0
      ? t("insights.share.cards.assistant.calls", {
          count: assistant.calls,
          time: formatShortDuration(assistant.call_seconds, t),
        })
      : t("insights.assistantCard.noCalls");
  const meetingsNote =
    meetings.seconds >= 60
      ? meetings.words > 0
        ? t("insights.share.cards.meetings.note", {
            time: formatShortDuration(meetings.seconds, t),
            words: formatCount(meetings.words, lang),
          })
        : t("insights.share.cards.meetings.time", {
            time: formatShortDuration(meetings.seconds, t),
          })
      : undefined;

  // The summary leads with words dictated, or with whatever has been used
  // when dictation has not.
  const lead: "dictation" | "assistant" | "meetings" = used.includes(
    "dictation",
  )
    ? "dictation"
    : used.includes("assistant")
      ? "assistant"
      : "meetings";
  const summary =
    lead === "dictation"
      ? {
          label: t("home.stats.wordsLabel"),
          value: fullNumber(words, lang),
          note:
            words >= NOVEL_WORDS
              ? t("insights.share.cards.words.books", {
                  count: Math.floor(words / NOVEL_WORDS),
                })
              : t("insights.pages", { count: pagesFor(words) }),
        }
      : lead === "assistant"
        ? {
            label: t("insights.figures.questions"),
            value: fullNumber(questions, lang),
            note: callsNote,
          }
        : {
            label: t("insights.figures.meetings"),
            value: fullNumber(meetings.meetings, lang),
            note: meetingsNote,
          };

  const figures: Figure[] = [];
  if (used.includes("dictation")) {
    figures.push(
      {
        key: "wpm",
        label: t("home.stats.wpmLabel"),
        value: wpm !== null ? String(wpm) : t("home.stats.none"),
        note:
          wpm !== null && wpm > TYPING_WPM
            ? t("insights.share.cards.speed.note", {
                times: (wpm / TYPING_WPM).toFixed(1),
              })
            : undefined,
        info: t("home.stats.wpmHint"),
      },
      {
        key: "saved",
        label: t("home.stats.savedLabel"),
        value: formatDuration(saved, t),
        note:
          workdays >= 1
            ? t("insights.share.cards.saved.workdays", { count: workdays })
            : undefined,
        info: t("home.stats.savedHint", { wpm: TYPING_WPM }),
      },
    );
  }
  figures.push({
    key: "streak",
    label: t("home.stats.streakLabel", {
      count: insights.current_streak_days,
    }),
    value: formatCount(insights.current_streak_days, lang),
    note: t("insights.longest", { count: insights.longest_streak_days }),
    info: t("insights.streakHint"),
  });
  if (used.includes("dictation")) {
    figures.push({
      key: "dictations",
      label: t("insights.figures.dictations"),
      value: formatCount(dictation.total_dictations, lang),
      note:
        dictation.today_words > 0
          ? t("home.stats.today", {
              words: formatCount(dictation.today_words, lang),
            })
          : undefined,
    });
  }
  if (used.includes("assistant") && lead !== "assistant") {
    figures.push({
      key: "questions",
      label: t("insights.figures.questions"),
      value: formatCount(questions, lang),
      note: callsNote,
      info: t("insights.assistantCard.info"),
    });
  }
  if (used.includes("meetings") && lead !== "meetings") {
    figures.push({
      key: "meetings",
      label: t("insights.figures.meetings"),
      value: formatCount(meetings.meetings, lang),
      note: meetingsNote,
      info: t("insights.meetingsCard.info"),
    });
  }

  return (
    <Page>
      {header}
      <div className="space-y-4">
        <Summary {...summary} />
        <Figures figures={figures} />
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
