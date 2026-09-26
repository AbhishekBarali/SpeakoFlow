import type { UsageDay } from "@/bindings";

/**
 * Arithmetic behind the Insights page, kept pure so it is testable. Dates are
 * local calendar dates built from year/month/day — never by adding 24 hours —
 * so a daylight-saving change can neither skip a day nor show one twice.
 */

export type HeatLevel = 0 | 1 | 2 | 3 | 4;

export interface HeatCell {
  /** Local calendar date, `YYYY-MM-DD` — the same key `UsageDay.day` uses. */
  key: string;
  date: Date;
  words: number;
  dictations: number;
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
  /** Most words on a single day in the grid. */
  max: number;
  /** Words across the grid. */
  total: number;
  /** Days in the grid with at least one dictation. */
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
export const heatLevel = (words: number, max: number): HeatLevel => {
  if (words <= 0 || max <= 0) return 0;
  const ratio = words / max;
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
  recent: UsageDay[],
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
      const words = Math.max(0, day?.words ?? 0);
      const dictations = Math.max(0, day?.dictations ?? 0);
      if (words > max) max = words;
      total += words;
      if (dictations > 0) activeDays += 1;
      column.push({
        key,
        date,
        words,
        dictations,
        level: 0,
        isToday: key === todayKey,
        future,
      });
    }
    columns.push(column);
  }
  for (const column of columns) {
    for (const cell of column) cell.level = heatLevel(cell.words, max);
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

/** 0 (Sunday) – 6 (Saturday): the weekday with the most words, or null. */
export const busiestWeekday = (days: UsageDay[]): number | null => {
  const totals = new Array<number>(7).fill(0);
  for (const day of days) {
    const date = parseDayKey(day.day);
    if (date && day.words > 0) totals[date.getDay()] += day.words;
  }
  const best = Math.max(...totals);
  return best > 0 ? totals.indexOf(best) : null;
};

/** The single day with the most words, or null when there is none. */
export const bestDay = (days: UsageDay[]): UsageDay | null =>
  days.reduce<UsageDay | null>(
    (best, day) =>
      day.words > 0 && (!best || day.words > best.words) ? day : best,
    null,
  );

/** Words on a typical day you dictate, rounded; 0 when there are none. */
export const averageActiveDay = (days: UsageDay[]): number => {
  const active = days.filter((day) => day.dictations > 0 && day.words > 0);
  if (active.length === 0) return 0;
  const words = active.reduce((sum, day) => sum + day.words, 0);
  return Math.round(words / active.length);
};

/** A printed page holds about this many words. */
export const WORDS_PER_PAGE = 250;

/** How many pages `words` would fill, at least 1 once there is any. */
export const pagesFor = (words: number): number =>
  words <= 0 ? 0 : Math.max(1, Math.round(words / WORDS_PER_PAGE));
