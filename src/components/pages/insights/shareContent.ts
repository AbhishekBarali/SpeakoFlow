import type { TFunction } from "i18next";
import type { Insights } from "@/bindings";
import {
  formatCount,
  formatShortDuration,
  TYPING_WPM,
  timeSavedSeconds,
  wordsPerMinute,
} from "../home/usageMath";
import {
  activitySeries,
  assistantQuestions,
  buildHeatmap,
  pagesFor,
  recentValues,
  type HeatLevel,
} from "./insightsMath";

/**
 * What the share cards say, kept apart from how they are drawn
 * (`shareCard.ts`) so it can be tested without fonts, images or a canvas.
 *
 * Sharing is a deck of cards, one insight each, the way a person would tell
 * it: how much they have said, how fast, how much time it saved, how many
 * days in a row, and what the assistant and meetings did. A card exists only
 * when its number is worth showing, so a new user is never handed a card
 * that says zero.
 */

export type CardFormat = "square" | "story" | "wide";

export const CARD_FORMATS: CardFormat[] = ["square", "story", "wide"];

export const CARD_SIZES: Record<CardFormat, { width: number; height: number }> =
  {
    square: { width: 1080, height: 1080 },
    story: { width: 1080, height: 1920 },
    wide: { width: 1600, height: 900 },
  };

export type CardKind =
  | "highlights"
  | "words"
  | "speed"
  | "saved"
  | "streak"
  | "assistant"
  | "meetings"
  | "activity";

/** Where a post links to. The cards themselves carry only the logo: an
 *  address printed on an image cannot be clicked. */
export const SHARE_URL = "https://speakoflow.com";

/** Words in a novel, near enough, for "as long as a novel". */
export const NOVEL_WORDS = 80_000;

/** Weeks of activity a card can show; each size draws as many as fit. */
export const ACTIVITY_WEEKS = 26;

export interface CardFigure {
  value: string;
  label: string;
}

export interface SpeedCompare {
  you: number;
  typing: number;
  youLabel: string;
  typingLabel: string;
}

export interface ActivityGrid {
  /** One column per week, oldest first, seven days each; -1 is a day still
   *  to come. */
  weeks: Array<Array<HeatLevel | -1>>;
  /** The first column of each month, with its short name. */
  months: Array<{ col: number; label: string }>;
}

export interface CardSlide {
  kind: CardKind;
  title: string;
  /** The card's figure, when it has one. */
  value?: string;
  /** A word set beside the figure, as in "12 days". */
  unit?: string;
  /** What the figure counts, under it. */
  label?: string;
  /** One sentence under that. */
  note?: string;
  /** The highlights card's figures. */
  items?: CardFigure[];
  /** The speed card's comparison with typing. */
  compare?: SpeedCompare;
  /** The activity card's grid. */
  activity?: ActivityGrid;
  /** What a post that carries this card says, before the link. */
  post: string;
  /** The card as one sentence, for screen readers. */
  alt: string;
}

export interface CardDeck {
  slides: CardSlide[];
  tagline: string;
  /** The last seven days, oldest first, each 0 to 1 of the busiest: the
   *  heights of the voice bars every card is drawn with. */
  week: number[];
}

/** A figure in full ("101,412"): it is the number being shown off. */
const fullNumber = (value: number, locale: string): string => {
  try {
    return new Intl.NumberFormat(locale, {
      notation: value >= 10_000_000 ? "compact" : "standard",
      maximumFractionDigits: 1,
    }).format(value);
  } catch {
    return String(value);
  }
};

/** Every card worth sharing for these numbers, in the order they are told. */
export const buildCardDeck = (
  insights: Insights,
  t: TFunction,
  locale: string,
  now: Date = new Date(),
): CardDeck => {
  const { dictation, assistant, meetings } = insights;
  const words = dictation.total_words;
  const questions = assistantQuestions(insights);
  const wpm = words > 0 ? wordsPerMinute(dictation) : null;
  const saved = words > 0 ? timeSavedSeconds(dictation) : 0;
  const streak = insights.current_streak_days;
  const count = (value: number) => formatCount(value, locale);
  const full = (value: number) => fullNumber(value, locale);

  const postFor = (key: "dictation" | "asks" | "meetings", value: number) =>
    t(`insights.share.post.${key}`, { count: value, formatted: full(value) });
  const leadPost =
    words > 0
      ? postFor("dictation", words)
      : questions > 0
        ? postFor("asks", questions)
        : postFor("meetings", meetings.meetings);
  const alt = (...parts: Array<string | undefined>) =>
    parts.filter(Boolean).join(". ");

  const slides: CardSlide[] = [];

  // Highlights: up to four figures, the ones there are.
  const items: CardFigure[] = [];
  if (words > 0) {
    items.push({
      value: full(words),
      label: t("insights.share.cards.highlights.words", { count: words }),
    });
  }
  if (streak >= 1) {
    items.push({
      value: count(streak),
      label: t("insights.share.cards.highlights.streak", { count: streak }),
    });
  }
  if (wpm !== null) {
    items.push({
      value: String(wpm),
      label: t("insights.share.cards.highlights.wpm"),
    });
  }
  if (questions > 0) {
    items.push({
      value: full(questions),
      label: t("insights.share.cards.highlights.questions", {
        count: questions,
      }),
    });
  }
  if (meetings.meetings > 0) {
    items.push({
      value: full(meetings.meetings),
      label: t("insights.share.cards.highlights.meetings", {
        count: meetings.meetings,
      }),
    });
  }
  if (saved >= 60) {
    items.push({
      value: formatShortDuration(saved, t),
      label: t("insights.share.cards.highlights.saved"),
    });
  }
  const shown = items.slice(0, 4);
  if (shown.length >= 2) {
    const title = t("insights.share.cards.highlights.title");
    slides.push({
      kind: "highlights",
      title,
      items: shown,
      post: leadPost,
      alt: alt(title, ...shown.map((item) => `${item.value} ${item.label}`)),
    });
  }

  if (words > 0) {
    const title = t("insights.share.cards.words.title");
    const label = t("insights.share.cards.words.label", { count: words });
    const note =
      words >= NOVEL_WORDS
        ? t("insights.share.cards.words.books", {
            count: Math.floor(words / NOVEL_WORDS),
          })
        : pagesFor(words) >= 1
          ? t("insights.share.cards.words.pages", { count: pagesFor(words) })
          : undefined;
    slides.push({
      kind: "words",
      title,
      value: full(words),
      label,
      note,
      post: postFor("dictation", words),
      alt: alt(title, `${full(words)} ${label}`, note),
    });
  }

  if (wpm !== null) {
    const title = t("insights.share.cards.speed.title");
    const label = t("insights.share.cards.speed.label");
    const note =
      wpm > TYPING_WPM
        ? t("insights.share.cards.speed.note", {
            times: (wpm / TYPING_WPM).toFixed(1),
          })
        : undefined;
    slides.push({
      kind: "speed",
      title,
      value: String(wpm),
      label,
      note,
      compare: {
        you: wpm,
        typing: TYPING_WPM,
        youLabel: t("insights.share.cards.speed.you"),
        typingLabel: t("insights.share.cards.speed.typing"),
      },
      post: postFor("dictation", words),
      alt: alt(title, `${wpm} ${label}`, note),
    });
  }

  if (saved >= 60) {
    const title = t("insights.share.cards.saved.title");
    const label = t("insights.share.cards.saved.label");
    const note = t("insights.share.cards.saved.note", { wpm: TYPING_WPM });
    const value = formatShortDuration(saved, t);
    slides.push({
      kind: "saved",
      title,
      value,
      label,
      note,
      post: postFor("dictation", words),
      alt: alt(title, `${value} ${label}`, note),
    });
  }

  if (streak >= 2) {
    const title = t("insights.share.cards.streak.title");
    const unit = t("insights.share.cards.streak.unit", { count: streak });
    const note =
      insights.longest_streak_days > streak
        ? t("insights.share.cards.streak.longest", {
            count: insights.longest_streak_days,
          })
        : t("insights.share.cards.streak.best");
    slides.push({
      kind: "streak",
      title,
      value: count(streak),
      unit,
      note,
      post: leadPost,
      alt: alt(title, `${count(streak)} ${unit}`, note),
    });
  }

  if (questions > 0) {
    const title = t("insights.share.cards.assistant.title");
    const label = t("insights.share.cards.assistant.label", {
      count: questions,
    });
    const note =
      assistant.calls > 0
        ? t("insights.share.cards.assistant.calls", {
            count: assistant.calls,
            time: formatShortDuration(assistant.call_seconds, t),
          })
        : undefined;
    slides.push({
      kind: "assistant",
      title,
      value: full(questions),
      label,
      note,
      post: postFor("asks", questions),
      alt: alt(title, `${full(questions)} ${label}`, note),
    });
  }

  if (meetings.meetings > 0) {
    const title = t("insights.share.cards.meetings.title");
    const label = t("insights.share.cards.meetings.label", {
      count: meetings.meetings,
    });
    const note =
      meetings.seconds >= 60
        ? meetings.words > 0
          ? t("insights.share.cards.meetings.note", {
              time: formatShortDuration(meetings.seconds, t),
              words: count(meetings.words),
            })
          : t("insights.share.cards.meetings.time", {
              time: formatShortDuration(meetings.seconds, t),
            })
        : undefined;
    slides.push({
      kind: "meetings",
      title,
      value: full(meetings.meetings),
      label,
      note,
      post: postFor("meetings", meetings.meetings),
      alt: alt(title, `${full(meetings.meetings)} ${label}`, note),
    });
  }

  const map = buildHeatmap(
    activitySeries(insights, "all"),
    ACTIVITY_WEEKS,
    now,
  );
  if (map.activeDays >= 2) {
    const title = t("insights.share.cards.activity.title");
    const label = t("insights.share.cards.activity.label", {
      count: map.activeDays,
    });
    let monthName: Intl.DateTimeFormat | null = null;
    try {
      monthName = new Intl.DateTimeFormat(locale, { month: "short" });
    } catch {
      monthName = null;
    }
    slides.push({
      kind: "activity",
      title,
      value: count(map.activeDays),
      label,
      activity: {
        weeks: map.weeks.map((column) =>
          column.map((cell) => (cell.future ? -1 : cell.level)),
        ),
        months: map.months.map(({ col, date }) => ({
          col,
          label: monthName ? monthName.format(date) : "",
        })),
      },
      post: leadPost,
      alt: alt(title, `${count(map.activeDays)} ${label}`),
    });
  }

  const recent = recentValues(activitySeries(insights, "all"), 7, now);
  const busiest = Math.max(...recent);
  return {
    slides,
    tagline: t("insights.share.tagline"),
    week: recent.map((value) => (busiest > 0 ? value / busiest : 0)),
  };
};
