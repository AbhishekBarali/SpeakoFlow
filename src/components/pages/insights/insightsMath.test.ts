import { describe, expect, test } from "bun:test";
import type { Insights } from "@/bindings";
import {
  activitySeries,
  averageActiveDay,
  bestDay,
  buildHeatmap,
  busiestWeekday,
  dayParts,
  hasActivity,
  heatLevel,
  localDayKey,
  pagesFor,
  parseDayKey,
  type DayValue,
} from "./insightsMath";

const day = (key: string, value: number): DayValue => ({ day: key, value });

const insights = (): Insights => ({
  dictation: {
    total_dictations: 3,
    total_words: 120,
    timed_words: 120,
    total_audio_seconds: 60,
    today_words: 0,
    today_dictations: 0,
    current_streak_days: 0,
    longest_streak_days: 1,
    active_days: 1,
    recent_days: [
      { day: "2026-09-20", dictations: 3, words: 120, audio_seconds: 60 },
    ],
  },
  assistant: {
    asks: 4,
    call_turns: 6,
    calls: 1,
    call_seconds: 300,
    active_days: ["2026-09-21"],
    recent_days: [
      {
        day: "2026-09-21",
        asks: 4,
        call_turns: 6,
        calls: 1,
        call_seconds: 300,
      },
    ],
  },
  meetings: {
    meetings: 0,
    seconds: 0,
    words: 0,
    longest_seconds: 0,
    active_days: [],
    recent_days: [],
  },
  current_streak_days: 0,
  longest_streak_days: 2,
  active_days: 2,
  first_day: "2026-09-20",
});

// Saturday, 26 Sep 2026, mid-afternoon.
const NOW = new Date(2026, 8, 26, 14, 30);

describe("buildHeatmap", () => {
  test("has one seven-day column per week, Sundays first", () => {
    const map = buildHeatmap([], 26, NOW);
    expect(map.weeks).toHaveLength(26);
    for (const column of map.weeks) {
      expect(column).toHaveLength(7);
      expect(column[0].date.getDay()).toBe(0);
    }
  });

  test("ends with the week that contains today", () => {
    const map = buildHeatmap([], 26, NOW);
    const last = map.weeks[25];
    const today = last.find((cell) => cell.isToday);
    expect(today?.key).toBe("2026-09-26");
    expect(map.weeks.flat().filter((cell) => cell.isToday)).toHaveLength(1);
  });

  test("marks the days after today as future, not quiet", () => {
    // A Wednesday leaves Thursday–Saturday of the last column in the future.
    const map = buildHeatmap([day("2026-09-24", 50)], 4, new Date(2026, 8, 23));
    const last = map.weeks[3];
    expect(last.slice(4).every((cell) => cell.future)).toBe(true);
    expect(last.slice(0, 4).some((cell) => cell.future)).toBe(false);
    // Activity recorded for a future day is ignored rather than drawn.
    expect(map.total).toBe(0);
  });

  test("fills days with their values and counts totals", () => {
    const map = buildHeatmap(
      [day("2026-09-26", 400), day("2026-09-20", 100), day("2025-01-01", 999)],
      26,
      NOW,
    );
    expect(map.max).toBe(400);
    expect(map.total).toBe(500);
    expect(map.activeDays).toBe(2);
    const cells = map.weeks.flat();
    expect(cells.find((cell) => cell.key === "2026-09-26")?.level).toBe(4);
    expect(cells.find((cell) => cell.key === "2026-09-20")?.level).toBe(1);
  });

  test("labels each month once, where its first column starts", () => {
    const map = buildHeatmap([], 26, NOW);
    const months = map.months.map((month) => month.date.getMonth());
    expect(new Set(months).size).toBe(months.length);
    for (const { col, date } of map.months) {
      expect(map.weeks[col][0].date.getTime()).toBe(date.getTime());
    }
  });

  test("steps by calendar day across daylight-saving changes", () => {
    for (const now of [new Date(2026, 3, 2), new Date(2026, 10, 3)]) {
      const keys = buildHeatmap([], 26, now)
        .weeks.flat()
        .map((cell) => cell.key);
      expect(new Set(keys).size).toBe(26 * 7);
    }
  });
});

describe("heatLevel", () => {
  test("is zero without activity and at least one with any", () => {
    expect(heatLevel(0, 100)).toBe(0);
    expect(heatLevel(5, 0)).toBe(0);
    expect(heatLevel(1, 1000)).toBe(1);
    expect(heatLevel(30, 100)).toBe(2);
    expect(heatLevel(60, 100)).toBe(3);
    expect(heatLevel(100, 100)).toBe(4);
  });
});

describe("highlights", () => {
  const days = [
    day("2026-09-23", 300), // Wednesday
    day("2026-09-16", 250), // Wednesday
    day("2026-09-26", 400), // Saturday
    day("2026-09-21", 0),
  ];

  test("finds the busiest weekday by total use", () => {
    expect(busiestWeekday(days)).toBe(3);
    expect(busiestWeekday([])).toBeNull();
  });

  test("finds the best single day", () => {
    expect(bestDay(days)?.day).toBe("2026-09-26");
    expect(bestDay([day("2026-09-01", 0)])).toBeNull();
  });

  test("averages only the days with any use", () => {
    expect(averageActiveDay(days)).toBe(317);
    expect(averageActiveDay([])).toBe(0);
  });

  test("turns words into pages", () => {
    expect(pagesFor(0)).toBe(0);
    expect(pagesFor(40)).toBe(1);
    expect(pagesFor(28_912)).toBe(116);
  });
});

describe("activities", () => {
  test("read each feature's own daily figure", () => {
    const data = insights();
    expect(activitySeries(data, "dictation")).toEqual([day("2026-09-20", 120)]);
    // Questions: quick asks plus things said on calls.
    expect(activitySeries(data, "assistant")).toEqual([day("2026-09-21", 10)]);
    expect(activitySeries(data, "meetings")).toEqual([]);
  });

  test("know which features were ever used", () => {
    const data = insights();
    expect(hasActivity(data, "dictation")).toBe(true);
    expect(hasActivity(data, "assistant")).toBe(true);
    expect(hasActivity(data, "meetings")).toBe(false);
    expect(hasActivity(data, "all")).toBe(true);
  });

  test("score all of them by each feature's own busiest day", () => {
    const data = insights();
    data.dictation.recent_days.push({
      day: "2026-09-21",
      dictations: 1,
      words: 60,
      audio_seconds: 30,
    });
    // Dictation peaks at 120 words and questions at 10, so a day with 60
    // words and all 10 questions scores a half plus a whole.
    expect(activitySeries(data, "all")).toEqual([
      day("2026-09-20", 1),
      day("2026-09-21", 1.5),
    ]);
  });

  test("say what each day held", () => {
    const parts = dayParts(insights());
    expect(parts.get("2026-09-20")).toEqual({
      words: 120,
      questions: 0,
      meetingSeconds: 0,
    });
    expect(parts.get("2026-09-21")?.questions).toBe(10);
  });
});

describe("day keys", () => {
  test("round-trip a local date", () => {
    const date = parseDayKey("2026-02-28");
    expect(date?.getDate()).toBe(28);
    expect(localDayKey(date as Date)).toBe("2026-02-28");
    expect(parseDayKey("not a date")).toBeNull();
  });
});
