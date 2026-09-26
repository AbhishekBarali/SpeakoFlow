import { describe, expect, test } from "bun:test";
import type { TFunction } from "i18next";
import type { UsageStats } from "@/bindings";
import {
  formatCount,
  formatDuration,
  lastNDays,
  partOfDay,
  speakingSeconds,
  timeSavedSeconds,
  wordsPerMinute,
} from "./usageMath";

const stats = (overrides: Partial<UsageStats> = {}): UsageStats => ({
  total_dictations: 0,
  total_words: 0,
  timed_words: 0,
  total_audio_seconds: 0,
  today_words: 0,
  today_dictations: 0,
  current_streak_days: 0,
  longest_streak_days: 0,
  active_days: 0,
  recent_days: [],
  ...overrides,
});

// Minimal stand-in: renders the key plus its interpolation values.
const t = ((key: string, values?: Record<string, unknown>) =>
  `${key}:${JSON.stringify(values ?? {})}`) as unknown as TFunction;

describe("wordsPerMinute", () => {
  test("is null until there is enough timed audio", () => {
    expect(wordsPerMinute(stats())).toBeNull();
    expect(
      wordsPerMinute(stats({ timed_words: 30, total_audio_seconds: 10 })),
    ).toBeNull();
  });

  test("divides timed words by timed minutes", () => {
    expect(
      wordsPerMinute(stats({ timed_words: 300, total_audio_seconds: 120 })),
    ).toBe(150);
  });
});

describe("speakingSeconds", () => {
  test("uses recorded audio when every word is timed", () => {
    expect(
      speakingSeconds(
        stats({ total_words: 300, timed_words: 300, total_audio_seconds: 120 }),
      ),
    ).toBe(120);
  });

  test("estimates untimed words at the user's own rate", () => {
    // 150 WPM from the timed part, so 150 untimed words add a minute.
    expect(
      speakingSeconds(
        stats({ total_words: 450, timed_words: 300, total_audio_seconds: 120 }),
      ),
    ).toBe(180);
  });

  test("falls back to a typical speaking rate with no timed audio", () => {
    expect(speakingSeconds(stats({ total_words: 130 }))).toBe(60);
  });
});

describe("timeSavedSeconds", () => {
  test("is zero with no words", () => {
    expect(timeSavedSeconds(stats())).toBe(0);
  });

  test("compares typing time with speaking time", () => {
    // 400 words: 10 min to type at 40 WPM, 2 min spoken.
    expect(
      timeSavedSeconds(
        stats({ total_words: 400, timed_words: 400, total_audio_seconds: 120 }),
      ),
    ).toBe(480);
  });

  test("never goes negative for a slow speaker", () => {
    expect(
      timeSavedSeconds(
        stats({ total_words: 20, timed_words: 20, total_audio_seconds: 600 }),
      ),
    ).toBe(0);
  });
});

describe("formatDuration", () => {
  test("picks the unit that reads naturally", () => {
    expect(formatDuration(40, t)).toBe(
      'home.stats.duration.seconds:{"seconds":40}',
    );
    expect(formatDuration(24 * 60, t)).toBe(
      'home.stats.duration.minutes:{"minutes":24}',
    );
    expect(formatDuration(2 * 3600, t)).toBe(
      'home.stats.duration.hours:{"hours":2}',
    );
    expect(formatDuration(3600 + 49 * 60, t)).toBe(
      'home.stats.duration.hoursMinutes:{"hours":1,"minutes":49}',
    );
  });

  test("clamps negative input to zero", () => {
    expect(formatDuration(-5, t)).toBe(
      'home.stats.duration.seconds:{"seconds":0}',
    );
  });
});

describe("formatCount", () => {
  test("keeps small numbers exact and compacts large ones", () => {
    expect(formatCount(9876, "en")).toBe("9,876");
    expect(formatCount(36_000, "en")).toBe("36K");
    expect(formatCount(1_250_000, "en")).toBe("1.3M");
  });
});

describe("partOfDay", () => {
  test("maps hours to a greeting", () => {
    expect(partOfDay(6)).toBe("morning");
    expect(partOfDay(13)).toBe("afternoon");
    expect(partOfDay(19)).toBe("evening");
    expect(partOfDay(23)).toBe("night");
    expect(partOfDay(3)).toBe("night");
  });
});

describe("lastNDays", () => {
  const day = (key: string, words: number) => ({
    day: key,
    words,
    dictations: 1,
    audio_seconds: 10,
  });

  test("returns one slot per day ending today, oldest first", () => {
    const slots = lastNDays([], 30, new Date(2026, 8, 26, 12));
    expect(slots).toHaveLength(30);
    expect(slots[0].key).toBe("2026-08-28");
    expect(slots[29].key).toBe("2026-09-26");
    expect(slots[29].isToday).toBe(true);
    expect(slots.filter((slot) => slot.isToday)).toHaveLength(1);
  });

  test("fills the days the backend left out with zero", () => {
    const slots = lastNDays(
      [day("2026-09-24", 120), day("2026-09-26", 40)],
      3,
      new Date(2026, 8, 26, 9),
    );
    expect(slots.map((slot) => slot.words)).toEqual([120, 0, 40]);
    expect(slots.map((slot) => slot.dictations)).toEqual([1, 0, 1]);
  });

  test("ignores days outside the window", () => {
    const slots = lastNDays([day("2026-01-01", 999)], 7, new Date(2026, 8, 26));
    expect(slots.every((slot) => slot.words === 0)).toBe(true);
  });

  test("counts calendar days across a daylight-saving change", () => {
    // Late March / late October cross a DST change in many zones; stepping by
    // calendar date must still give distinct, consecutive days.
    for (const now of [new Date(2026, 3, 2), new Date(2026, 10, 3)]) {
      const keys = lastNDays([], 30, now).map((slot) => slot.key);
      expect(new Set(keys).size).toBe(30);
    }
  });
});
