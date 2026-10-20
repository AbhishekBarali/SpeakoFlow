import { describe, expect, test } from "bun:test";
import type { TFunction } from "i18next";
import type { Insights } from "@/bindings";
import {
  buildCardContent,
  leadActivity,
  VOICEPRINT_DAYS,
} from "./shareContent";

/** Echoes the key and the values it was given, so tests can read which
 *  string was asked for without the translation files. */
const t = ((key: string, options?: Record<string, unknown>) =>
  options && Object.keys(options).length
    ? `${key}${JSON.stringify(options)}`
    : key) as unknown as TFunction;

// Saturday, 26 Sep 2026.
const NOW = new Date(2026, 8, 26, 14, 30);

const insights = (overrides: Partial<Insights> = {}): Insights => ({
  dictation: {
    total_dictations: 1_071,
    total_words: 27_676,
    timed_words: 27_676,
    total_audio_seconds: 12_000,
    today_words: 186,
    today_dictations: 7,
    current_streak_days: 0,
    longest_streak_days: 9,
    active_days: 116,
    recent_days: [
      { day: "2026-09-26", dictations: 7, words: 186, audio_seconds: 80 },
      { day: "2026-07-01", dictations: 1, words: 50, audio_seconds: 20 },
    ],
  },
  assistant: {
    asks: 280,
    call_turns: 23,
    calls: 13,
    call_seconds: 5_460,
    active_days: ["2026-09-25"],
    recent_days: [
      { day: "2026-09-25", asks: 4, call_turns: 0, calls: 0, call_seconds: 0 },
    ],
  },
  meetings: {
    meetings: 23,
    seconds: 54_300,
    words: 129_000,
    longest_seconds: 3_300,
    active_days: ["2026-09-22"],
    recent_days: [
      { day: "2026-09-22", meetings: 1, seconds: 1_500, words: 3_600 },
    ],
  },
  current_streak_days: 12,
  longest_streak_days: 19,
  active_days: 140,
  first_day: "2026-04-03",
  ...overrides,
});

const everything = { assistant: true, meetings: true };

describe("the share card", () => {
  test("leads with dictation and shows every feature that was used", () => {
    const card = buildCardContent(insights(), t, "en", everything, NOW);
    expect(card?.value).toBe("27,676");
    expect(card?.lead).toContain("insights.share.headline.dictation.lead");
    expect(card?.facts.map((fact) => fact.label.split("{")[0])).toEqual([
      "insights.share.fact.saved",
      "insights.share.fact.questionsAsked",
      "insights.share.fact.meetings",
      "insights.share.fact.wpm",
      "insights.share.fact.streak",
      "insights.share.fact.activeDays",
      "insights.share.fact.dictations",
    ]);
    // Questions are quick asks plus things said on calls.
    expect(card?.facts[1].value).toBe("303");
  });

  test("leaves out what the user switched off", () => {
    const card = buildCardContent(
      insights(),
      t,
      "en",
      { assistant: false, meetings: false },
      NOW,
    );
    const labels = card?.facts.map((fact) => fact.label) ?? [];
    expect(labels.some((label) => label.includes("questions"))).toBe(false);
    expect(labels.some((label) => label.includes("meetings"))).toBe(false);
  });

  test("leads with the assistant when there is no dictation", () => {
    const data = insights({
      dictation: { ...insights().dictation, total_words: 0, recent_days: [] },
    });
    expect(leadActivity(data, everything)).toBe("assistant");
    const card = buildCardContent(data, t, "en", everything, NOW);
    expect(card?.value).toBe("303");
    const labels = card?.facts.map((fact) => fact.label.split("{")[0]) ?? [];
    expect(labels).not.toContain("insights.share.fact.questionsAsked");
    expect(labels).not.toContain("insights.share.fact.saved");
    expect(labels).toContain("insights.share.fact.callTime");
  });

  test("leads with meetings when nothing else was used", () => {
    const data = insights({
      dictation: { ...insights().dictation, total_words: 0, recent_days: [] },
      assistant: { ...insights().assistant, asks: 0, call_turns: 0, calls: 0 },
    });
    const card = buildCardContent(data, t, "en", everything, NOW);
    expect(card?.value).toBe("23");
    expect(card?.facts[0].label).toBe("insights.share.fact.meetingTime");
  });

  test("has nothing to show before anything was used", () => {
    const data = insights({
      dictation: { ...insights().dictation, total_words: 0, recent_days: [] },
      assistant: { ...insights().assistant, asks: 0, call_turns: 0, calls: 0 },
      meetings: { ...insights().meetings, meetings: 0 },
    });
    expect(buildCardContent(data, t, "en", everything, NOW)).toBeNull();
  });

  test("draws twelve weeks of the lead feature, ending today", () => {
    const card = buildCardContent(insights(), t, "en", everything, NOW);
    expect(card?.bars).toHaveLength(VOICEPRINT_DAYS);
    expect(card?.bars[VOICEPRINT_DAYS - 1]).toBe(186);
    // July 1st is older than twelve weeks.
    expect(card?.bars.reduce((sum, value) => sum + value, 0)).toBe(186);
  });

  test("says since when, from the first day anything was counted", () => {
    const card = buildCardContent(insights(), t, "en", everything, NOW);
    expect(card?.period).toBe('insights.share.since{"date":"April 2026"}');
  });
});
