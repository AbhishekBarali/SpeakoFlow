import type { Insights } from "@/bindings";

/**
 * Arithmetic behind the Insights page, kept pure so it is testable. Dates are
 * local calendar dates built from year/month/day — never by adding 24 hours —
 * so a daylight-saving change can neither skip a day nor show one twice.
 */

export type HeatLevel = 0 | 1 | 2 | 3 | 4;

/** A part of the app whose use is counted. */
export type Feature = "dictation" | "assistant" | "meetings";

export const FEATURES: Feature[] = ["dictation", "assistant", "meetings"];

/** What the activity grid and the highlights can be about: one feature, or
 *  all of them, which is what the streak counts. */
export type Activity = "all" | Feature;

/** One day's figure for one activity. */
export interface DayValue {
  /** Local calendar date, `YYYY-MM-DD`. */
  day: string;
  value: number;
}

/**
 * Each day's figure for a feature, as the backend sent it (days without any
 * use are left out): words dictated, questions put to the assistant (quick asks
 * and things said on calls), or seconds of meetings.
 */
export const featureSeries = (
  insights: Insights,
  feature: Feature,
): DayValue[] => {
  switch (feature) {
    case "dictation":
      return (insights.dictation.recent_days ?? []).map((day) => ({
        day: day.day,
        value: day.words,
      }));
    case "assistant":
      return (insights.assistant.recent_days ?? []).map((day) => ({
        day: day.day,
        value: day.asks + day.call_turns,
      }));
    case "meetings":
      return (insights.meetings.recent_days ?? []).map((day) => ({
        day: day.day,
        value: day.seconds,
      }));
  }
};

/**
 * Each day's figure for an activity. For all of them together a day scores
 * the sum of each feature's share of its own busiest day, so a full day of
 * meetings does not drown out a day of dictation; only how dark the square
 * is reads off it.
 */
export const activitySeries = (
  insights: Insights,
  activity: Activity,
): DayValue[] => {
  if (activity !== "all") return featureSeries(insights, activity);
  const scores = new Map<string, number>();
  for (const feature of FEATURES) {
    const series = featureSeries(insights, feature);
    const max = Math.max(0, ...series.map((day) => day.value));
    if (max <= 0) continue;
    for (const day of series) {
      if (day.value <= 0) continue;
      scores.set(day.day, (scores.get(day.day) ?? 0) + day.value / max);
    }
  }
  return [...scores]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([day, value]) => ({ day, value }));
};

/** What one day held, for saying it in words. */
export interface DayParts {
  words: number;
  questions: number;
  meetingSeconds: number;
}

/** Each recent day's dictation, questions and meeting time, by day key. */
export const dayParts = (insights: Insights): Map<string, DayParts> => {
  const parts = new Map<string, DayParts>();
  const entry = (day: string) => {
    let found = parts.get(day);
    if (!found) {
      found = { words: 0, questions: 0, meetingSeconds: 0 };
      parts.set(day, found);
    }
    return found;
  };
  for (const day of featureSeries(insights, "dictation"))
    entry(day.day).words += day.value;
  for (const day of featureSeries(insights, "assistant"))
    entry(day.day).questions += day.value;
  for (const day of featureSeries(insights, "meetings"))
    entry(day.day).meetingSeconds += day.value;
  return parts;
};

/** Questions put to the assistant: quick asks plus things said on calls. */
export const assistantQuestions = (insights: Insights): number =>
  insights.assistant.asks + insights.assistant.call_turns;

/** Whether an activity has ever been used. */
export const hasActivity = (
  insights: Insights,
  activity: Activity,
): boolean => {
  switch (activity) {
    case "all":
      return FEATURES.some((feature) => hasActivity(insights, feature));
    case "dictation":
      return insights.dictation.total_words > 0;
    case "assistant":
      return assistantQuestions(insights) > 0 || insights.assistant.calls > 0;
    case "meetings":
      return insights.meetings.meetings > 0;
  }
};

export interface HeatCell {
  /** Local calendar date, `YYYY-MM-DD` — the same key `DayValue.day` uses. */
  key: string;
  date: Date;
  value: number;
  level: HeatLevel;
  isToday: boolean;
  /** After today: drawn as an empty slot, never as a quiet day. */
  future: boolean;
}

export interface Heatmap {
  /** One column per week, Sunday first; each column has seven cells. */
  weeks: HeatCell[][];
  /** Columns where a new month starts, for the labels above the grid. */
  months: Array<{ col: number; date: Date }>;
  /** The highest single day in the grid. */
  max: number;
  /** The sum across the grid. */
  total: number;
  /** Days in the grid with any use. */
  activeDays: number;
}

export const localDayKey = (date: Date): string =>
  `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(
    date.getDate(),
  ).padStart(2, "0")}`;

/** `YYYY-MM-DD` as a local date, or null when it is not one. */
export const parseDayKey = (key: string): Date | null => {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(key);
  if (!match) return null;
  const date = new Date(
    Number(match[1]),
    Number(match[2]) - 1,
    Number(match[3]),
  );
  return Number.isNaN(date.getTime()) ? null : date;
};

/** Relative to the busiest day, in quarters; any activity is at least 1. */
export const heatLevel = (value: number, max: number): HeatLevel => {
  if (value <= 0 || max <= 0) return 0;
  const ratio = value / max;
  if (ratio > 0.75) return 4;
  if (ratio > 0.5) return 3;
  if (ratio > 0.25) return 2;
  return 1;
};

/**
 * The last `weeks` weeks as a grid, ending with the week that contains today.
 * Days the backend left out (it only sends days with activity) are zero.
 */
export const buildHeatmap = (
  recent: DayValue[],
  weeks: number,
  now: Date = new Date(),
): Heatmap => {
  const byDay = new Map(recent.map((day) => [day.day, day]));
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const todayKey = localDayKey(today);
  // The Sunday that starts the first column.
  const start = new Date(
    today.getFullYear(),
    today.getMonth(),
    today.getDate() - today.getDay() - (weeks - 1) * 7,
  );

  const columns: HeatCell[][] = [];
  let max = 0;
  let total = 0;
  let activeDays = 0;
  for (let col = 0; col < weeks; col += 1) {
    const column: HeatCell[] = [];
    for (let row = 0; row < 7; row += 1) {
      const date = new Date(
        start.getFullYear(),
        start.getMonth(),
        start.getDate() + col * 7 + row,
      );
      const key = localDayKey(date);
      const future = date.getTime() > today.getTime();
      const day = future ? undefined : byDay.get(key);
      const value = Math.max(0, day?.value ?? 0);
      if (value > max) max = value;
      total += value;
      if (value > 0) activeDays += 1;
      column.push({
        key,
        date,
        value,
        level: 0,
        isToday: key === todayKey,
        future,
      });
    }
    columns.push(column);
  }
  for (const column of columns) {
    for (const cell of column) cell.level = heatLevel(cell.value, max);
  }

  // A label where a column's first day falls in a new month. The first
  // column is only labelled when its month runs long enough to fit the text.
  const months: Heatmap["months"] = [];
  columns.forEach((column, col) => {
    const first = column[0].date;
    const previous = col > 0 ? columns[col - 1][0].date : null;
    if (previous && previous.getMonth() === first.getMonth()) return;
    if (!previous) {
      const next = columns[2]?.[0].date;
      if (next && next.getMonth() !== first.getMonth()) return;
    }
    months.push({ col, date: first });
  });

  return { weeks: columns, months, max, total, activeDays };
};

/** 0 (Sunday) – 6 (Saturday): the weekday with the most use, or null. */
export const busiestWeekday = (days: DayValue[]): number | null => {
  const totals = new Array<number>(7).fill(0);
  for (const day of days) {
    const date = parseDayKey(day.day);
    if (date && day.value > 0) totals[date.getDay()] += day.value;
  }
  const best = Math.max(...totals);
  return best > 0 ? totals.indexOf(best) : null;
};

/** The single day with the most use, or null when there is none. */
export const bestDay = (days: DayValue[]): DayValue | null =>
  days.reduce<DayValue | null>(
    (best, day) =>
      day.value > 0 && (!best || day.value > best.value) ? day : best,
    null,
  );

/** A typical day of use, averaged over the days with any; 0 when none. */
export const averageActiveDay = (days: DayValue[]): number => {
  const active = days.filter((day) => day.value > 0);
  if (active.length === 0) return 0;
  const total = active.reduce((sum, day) => sum + day.value, 0);
  return Math.round(total / active.length);
};

/** A printed page holds about this many words. */
export const WORDS_PER_PAGE = 250;

/** How many pages `words` would fill, at least 1 once there is any. */
export const pagesFor = (words: number): number =>
  words <= 0 ? 0 : Math.max(1, Math.round(words / WORDS_PER_PAGE));
