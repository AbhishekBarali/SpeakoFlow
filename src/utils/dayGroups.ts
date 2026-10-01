/**
 * Group a newest-first list into calendar days: "Today", "Yesterday", a
 * weekday for the rest of the past week, then a date.
 *
 * Shared by History and Meetings so the two lists name days the same way.
 * `now` is a parameter so the labels are testable without a clock.
 */
export interface DayGroup<T> {
  /** Stable per calendar day, for React keys. */
  key: string;
  label: string;
  items: T[];
}

export interface DayLabels {
  today: string;
  yesterday: string;
}

const dayKey = (seconds: number): string => {
  const date = new Date(seconds * 1000);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
};

export const groupByDay = <T>(
  items: readonly T[],
  secondsOf: (item: T) => number,
  locale: string,
  labels: DayLabels,
  now: Date = new Date(),
): DayGroup<T>[] => {
  const today = dayKey(now.getTime() / 1000);
  const yesterday = dayKey(now.getTime() / 1000 - 86_400);
  const weekAgo = now.getTime() - 6 * 86_400_000;
  const groups: DayGroup<T>[] = [];
  for (const item of items) {
    const seconds = secondsOf(item);
    const key = dayKey(seconds);
    let group = groups[groups.length - 1];
    if (!group || group.key !== key) {
      const date = new Date(seconds * 1000);
      let label: string;
      if (key === today) label = labels.today;
      else if (key === yesterday) label = labels.yesterday;
      else {
        try {
          label = new Intl.DateTimeFormat(
            locale,
            date.getTime() >= weekAgo
              ? { weekday: "long" }
              : date.getFullYear() === now.getFullYear()
                ? { weekday: "short", month: "long", day: "numeric" }
                : { year: "numeric", month: "long", day: "numeric" },
          ).format(date);
        } catch {
          label = date.toDateString();
        }
      }
      group = { key, label, items: [] };
      groups.push(group);
    }
    group.items.push(item);
  }
  return groups;
};

/** "4:07 PM" — for a row whose day is already its group's heading. */
export const formatTimeOfDay = (seconds: number, locale: string): string => {
  try {
    return new Intl.DateTimeFormat(locale, {
      hour: "numeric",
      minute: "2-digit",
    }).format(new Date(seconds * 1000));
  } catch {
    return "";
  }
};
