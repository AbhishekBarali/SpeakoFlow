import type { TFunction } from "i18next";
import type { UsageDay, UsageStats } from "@/bindings";

/**
 * Arithmetic behind the Home page's usage card, kept pure so it is testable.
 *
 * Every figure here is an estimate and the copy says so where it matters, but
 * each estimate is chosen to err against flattering the app: WPM counts the
 * whole recording (pauses included), and "time saved" compares against an
 * average typist rather than a slow one.
 */

/** Average adult keyboard typing speed, the baseline for "time saved". */
export const TYPING_WPM = 40;

/** Speaking rate assumed for words whose recording length is unknown. */
export const FALLBACK_SPEAKING_WPM = 130;

/** Below this much timed audio, a words-per-minute figure is noise. */
export const MIN_WPM_SECONDS = 20;

/** Speaking rate over recordings of known length, or null while there is too little. */
export const wordsPerMinute = (stats: UsageStats): number | null => {
  if (stats.total_audio_seconds < MIN_WPM_SECONDS || stats.timed_words <= 0) {
    return null;
  }
  return Math.round(stats.timed_words / (stats.total_audio_seconds / 60));
};

/** Seconds spent speaking, estimating the part whose audio length is unknown. */
export const speakingSeconds = (stats: UsageStats): number => {
  const untimedWords = Math.max(0, stats.total_words - stats.timed_words);
  const rate = wordsPerMinute(stats) ?? FALLBACK_SPEAKING_WPM;
  return stats.total_audio_seconds + (untimedWords / rate) * 60;
};

/** How much longer typing the same words would have taken, never negative. */
export const timeSavedSeconds = (
  stats: UsageStats,
  typingWpm: number = TYPING_WPM,
): number => {
  if (stats.total_words <= 0 || typingWpm <= 0) return 0;
  const typing = (stats.total_words / typingWpm) * 60;
  return Math.max(0, typing - speakingSeconds(stats));
};

/** "1 hr 49 min", "24 min", "40 sec". */
export const formatDuration = (seconds: number, t: TFunction): string => {
  const total = Math.max(0, Math.round(seconds));
  if (total < 60) return t("home.stats.duration.seconds", { seconds: total });
  const minutesTotal = Math.round(total / 60);
  const hours = Math.floor(minutesTotal / 60);
  const minutes = minutesTotal % 60;
  if (hours === 0) return t("home.stats.duration.minutes", { minutes });
  if (minutes === 0) return t("home.stats.duration.hours", { hours });
  return t("home.stats.duration.hoursMinutes", { hours, minutes });
};

/** "3.6K" for large counts, the plain number below ten thousand. */
export const formatCount = (value: number, locale: string): string => {
  try {
    return new Intl.NumberFormat(locale, {
      notation: value >= 10_000 ? "compact" : "standard",
      maximumFractionDigits: 1,
    }).format(value);
  } catch {
    return String(value);
  }
};

/** The part of the day, for the greeting. */
export const partOfDay = (
  hour: number,
): "morning" | "afternoon" | "evening" | "night" => {
  if (hour >= 5 && hour < 12) return "morning";
  if (hour >= 12 && hour < 17) return "afternoon";
  if (hour >= 17 && hour < 22) return "evening";
  return "night";
};

/** One day of the activity chart. */
export interface DaySlot {
  /** Local calendar date, `YYYY-MM-DD` — the same key `UsageDay.day` uses. */
  key: string;
  date: Date;
  words: number;
  dictations: number;
  isToday: boolean;
}

const localDayKey = (date: Date): string =>
  `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(
    date.getDate(),
  ).padStart(2, "0")}`;

/**
 * The last `count` local days ending today, oldest first, with the days the
 * backend left out (it only sends days with activity) filled in as zero. Built
 * from calendar dates, not by subtracting 24 hours, so a daylight-saving change
 * can neither skip a day nor show one twice.
 */
export const lastNDays = (
  recent: UsageDay[],
  count: number,
  now: Date = new Date(),
): DaySlot[] => {
  const byDay = new Map(recent.map((day) => [day.day, day]));
  const slots: DaySlot[] = [];
  for (let offset = count - 1; offset >= 0; offset -= 1) {
    const date = new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate() - offset,
    );
    const key = localDayKey(date);
    const day = byDay.get(key);
    slots.push({
      key,
      date,
      words: day?.words ?? 0,
      dictations: day?.dictations ?? 0,
      isToday: offset === 0,
    });
  }
  return slots;
};
