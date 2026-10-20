import type { TFunction } from "i18next";
import type { Insights } from "@/bindings";
import {
  formatCount,
  formatShortDuration,
  timeSavedSeconds,
  wordsPerMinute,
} from "../home/usageMath";
import {
  activitySeries,
  assistantQuestions,
  recentValues,
  type Activity,
} from "./insightsMath";

/**
 * What the share card says, kept apart from how it is drawn (`shareCard.ts`)
 * so it can be tested without fonts, images or a canvas.
 */

export type CardFormat = "square" | "story" | "wide";
export type CardLook = "paper" | "graphite" | "mint" | "dusk";

export const CARD_FORMATS: CardFormat[] = ["square", "story", "wide"];
export const CARD_LOOKS: CardLook[] = ["paper", "graphite", "mint", "dusk"];

export const CARD_SIZES: Record<CardFormat, { width: number; height: number }> =
  {
    square: { width: 1080, height: 1080 },
    story: { width: 1080, height: 1920 },
    wide: { width: 1600, height: 900 },
  };

/** Days in the voiceprint: twelve weeks. */
export const VOICEPRINT_DAYS = 84;

/** Where the card sends people. Not translated: it is an address. */
export const CARD_SITE = "speakoflow.com";

export interface CardFact {
  value: string;
  label: string;
}

export interface CardContent {
  /** The headline figure, formatted. */
  value: string;
  /** The words after it, in ink. */
  lead: string;
  /** The end of the sentence, in the accent colour. */
  accent: string;
  /** Supporting figures, most telling first; each layout takes what it fits. */
  facts: CardFact[];
  /** The voiceprint: one value per day, oldest first. */
  bars: number[];
  barsCaption: string;
  period: string;
  tagline: string;
  /** The whole card as one sentence, for screen readers. */
  label: string;
}

export interface CardInclude {
  assistant: boolean;
  meetings: boolean;
}

/** The headline figure in full ("99,812"): it is the number being shown off. */
const headlineNumber = (value: number, locale: string): string => {
  try {
    return new Intl.NumberFormat(locale, {
      notation: value >= 1_000_000 ? "compact" : "standard",
      maximumFractionDigits: 1,
    }).format(value);
  } catch {
    return String(value);
  }
};

/**
 * Which feature the card is about. Dictation, once there is enough of it to
 * show; otherwise whichever other feature was used.
 */
export const leadActivity = (
  insights: Insights,
  include: CardInclude,
): Activity | null => {
  const words = insights.dictation.total_words;
  const questions = include.assistant ? assistantQuestions(insights) : 0;
  const meetings = include.meetings ? insights.meetings.meetings : 0;
  if (words >= 100 || (words > 0 && questions === 0 && meetings === 0)) {
    return "dictation";
  }
  if (questions > 0) return "assistant";
  if (meetings > 0) return "meetings";
  return words > 0 ? "dictation" : null;
};

/** Everything the card says, or null when there is nothing to show yet. */
export const buildCardContent = (
  insights: Insights,
  t: TFunction,
  locale: string,
  include: CardInclude,
  now: Date = new Date(),
): CardContent | null => {
  const lead = leadActivity(insights, include);
  if (!lead) return null;

  const { dictation, assistant, meetings } = insights;
  const questions = assistantQuestions(insights);
  const count = (value: number) => formatCount(value, locale);

  let value: number;
  switch (lead) {
    case "dictation":
      value = dictation.total_words;
      break;
    case "assistant":
      value = questions;
      break;
    case "meetings":
      value = meetings.meetings;
      break;
  }

  const facts: CardFact[] = [];
  const saved = timeSavedSeconds(dictation);
  const wpm = wordsPerMinute(dictation);
  if (dictation.total_words > 0 && saved >= 60) {
    facts.push({
      value: formatShortDuration(saved, t),
      label: t("insights.share.fact.saved"),
    });
  }
  if (include.assistant && questions > 0 && lead !== "assistant") {
    facts.push({
      value: count(questions),
      label: t("insights.share.fact.questions", { count: questions }),
    });
  }
  if (include.meetings && meetings.meetings > 0 && lead !== "meetings") {
    facts.push({
      value: count(meetings.meetings),
      label: t("insights.share.fact.meetings", { count: meetings.meetings }),
    });
  }
  if (lead === "meetings" && meetings.seconds >= 60) {
    facts.push({
      value: formatShortDuration(meetings.seconds, t),
      label: t("insights.share.fact.meetingTime"),
    });
  }
  if (lead === "meetings" && meetings.words > 0) {
    facts.push({
      value: count(meetings.words),
      label: t("insights.share.fact.meetingWords", { count: meetings.words }),
    });
  }
  if (lead === "assistant" && assistant.call_seconds >= 60) {
    facts.push({
      value: formatShortDuration(assistant.call_seconds, t),
      label: t("insights.share.fact.callTime"),
    });
  }
  if (wpm !== null && dictation.total_words > 0) {
    facts.push({
      value: String(wpm),
      label: t("insights.share.fact.wpm"),
    });
  }
  if (insights.current_streak_days >= 2) {
    facts.push({
      value: count(insights.current_streak_days),
      label: t("insights.share.fact.streak", {
        count: insights.current_streak_days,
      }),
    });
  }
  if (insights.active_days >= 2) {
    facts.push({
      value: count(insights.active_days),
      label: t("insights.share.fact.activeDays", {
        count: insights.active_days,
      }),
    });
  }
  if (lead === "dictation" && dictation.total_dictations > 0) {
    facts.push({
      value: count(dictation.total_dictations),
      label: t("insights.share.fact.dictations", {
        count: dictation.total_dictations,
      }),
    });
  }

  const first = insights.first_day
    ? /^(\d{4})-(\d{2})-(\d{2})$/.exec(insights.first_day)
    : null;
  let period = "";
  if (first) {
    try {
      period = t("insights.share.since", {
        date: new Intl.DateTimeFormat(locale, {
          month: "long",
          year: "numeric",
        }).format(new Date(Number(first[1]), Number(first[2]) - 1, 1)),
      });
    } catch {
      period = "";
    }
  }

  const headline = {
    value: headlineNumber(value, locale),
    lead: t(`insights.share.headline.${lead}.lead`, { count: value }),
    accent: t(`insights.share.headline.${lead}.accent`, { count: value }),
  };
  const summary = [
    `${headline.value} ${headline.lead} ${headline.accent}`,
    ...facts.slice(0, 4).map((fact) => `${fact.value} ${fact.label}`),
  ].join(". ");

  return {
    ...headline,
    facts,
    bars: recentValues(activitySeries(insights, lead), VOICEPRINT_DAYS, now),
    barsCaption: t("insights.share.barsCaption", {
      count: VOICEPRINT_DAYS / 7,
    }),
    period,
    tagline: t("insights.share.tagline"),
    label: `${summary}. ${CARD_SITE}`,
  };
};
