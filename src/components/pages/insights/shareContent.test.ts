import { describe, expect, test } from "bun:test";
import type { TFunction } from "i18next";
import type { Insights } from "@/bindings";
import { buildCardDeck, NOVEL_WORDS } from "./shareContent";
import { composeUrl } from "./shareTargets";

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

const noDictation = (base: Insights): Insights => ({
  ...base,
  dictation: {
    ...base.dictation,
    total_dictations: 0,
    total_words: 0,
    timed_words: 0,
    total_audio_seconds: 0,
    today_words: 0,
    today_dictations: 0,
    recent_days: [],
  },
});

const kinds = (data: Insights) =>
  buildCardDeck(data, t, "en", NOW).slides.map((slide) => slide.kind);

describe("the share deck", () => {
  test("tells everything that was used, in order", () => {
    expect(kinds(insights())).toEqual([
      "highlights",
      "words",
      "speed",
      "saved",
      "streak",
      "assistant",
      "meetings",
      "activity",
    ]);
  });

  test("never hands over a card that says zero", () => {
    const deck = kinds(
      noDictation({
        ...insights(),
        meetings: { ...insights().meetings, meetings: 0, seconds: 0, words: 0 },
        current_streak_days: 1,
      }),
    );
    expect(deck).not.toContain("words");
    expect(deck).not.toContain("speed");
    expect(deck).not.toContain("saved");
    expect(deck).not.toContain("meetings");
    expect(deck).not.toContain("streak");
    expect(deck).toContain("assistant");
  });

  test("the highlights card takes the first four figures there are", () => {
    const highlights = buildCardDeck(insights(), t, "en", NOW).slides[0];
    expect(highlights.items?.map((item) => item.label.split("{")[0])).toEqual([
      "insights.share.cards.highlights.words",
      "insights.share.cards.highlights.streak",
      "insights.share.cards.highlights.wpm",
      "insights.share.cards.highlights.questions",
    ]);
  });

  test("the words card shows the number in full and puts it in pages", () => {
    const words = buildCardDeck(insights(), t, "en", NOW).slides.find(
      (slide) => slide.kind === "words",
    );
    expect(words?.value).toBe("27,676");
    expect(words?.note).toContain("insights.share.cards.words.pages");
  });

  test("a novel's worth of words is said as novels", () => {
    const base = insights();
    const words = buildCardDeck(
      {
        ...base,
        dictation: { ...base.dictation, total_words: NOVEL_WORDS * 2 + 10 },
      },
      t,
      "en",
      NOW,
    ).slides.find((slide) => slide.kind === "words");
    expect(words?.note).toContain(
      'insights.share.cards.words.books{"count":2}',
    );
  });

  test("the streak card names the longest streak only when it is longer", () => {
    const streak = (current: number, longest: number) =>
      buildCardDeck(
        insights({
          current_streak_days: current,
          longest_streak_days: longest,
        }),
        t,
        "en",
        NOW,
      ).slides.find((slide) => slide.kind === "streak")?.note;
    expect(streak(12, 19)).toContain("insights.share.cards.streak.longest");
    expect(streak(19, 19)).toBe("insights.share.cards.streak.best");
  });

  test("each card's post speaks for the feature it shows", () => {
    const slides = buildCardDeck(insights(), t, "en", NOW).slides;
    const post = (kind: string) =>
      slides.find((slide) => slide.kind === kind)?.post ?? "";
    expect(post("words")).toContain("insights.share.post.dictation");
    expect(post("assistant")).toContain("insights.share.post.asks");
    expect(post("meetings")).toContain("insights.share.post.meetings");
  });

  test("the voice bars follow the last seven days", () => {
    const { week } = buildCardDeck(insights(), t, "en", NOW);
    expect(week).toHaveLength(7);
    expect(Math.max(...week)).toBe(1);
    expect(week.every((value) => value >= 0 && value <= 1)).toBe(true);
  });

  test("the activity card counts its days and holds six months of weeks", () => {
    const activity = buildCardDeck(insights(), t, "en", NOW).slides.find(
      (slide) => slide.kind === "activity",
    )?.activity;
    expect(activity?.weeks).toHaveLength(26);
    const last = activity?.weeks[25] ?? [];
    // Saturday is the last day of the week, so nothing is still to come.
    expect(last.filter((level) => level < 0)).toHaveLength(0);
  });
});

describe("posting a card", () => {
  test("opens a new post with the text and the link", () => {
    const url = composeUrl("x", "I spoke 10 words.");
    expect(url.startsWith("https://x.com/intent/post?text=")).toBe(true);
    expect(decodeURIComponent(url.split("text=")[1])).toBe(
      "I spoke 10 words. https://speakoflow.com",
    );
  });

  test("Facebook opens on the feed, which takes no text from a link", () => {
    expect(composeUrl("facebook", "anything")).toBe(
      "https://www.facebook.com/",
    );
  });
});
