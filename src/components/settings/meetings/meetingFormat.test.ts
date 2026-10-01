import { expect, test } from "bun:test";
import { groupByDay } from "@/utils/dayGroups";
import { formatDuration, speakerInitial } from "./speakers";

test("a length reads as a duration, not as a time of day", () => {
  expect(formatDuration(45_000, "en")).toBe("45 sec");
  expect(formatDuration(1_834_000, "en")).toBe("31 min");
  expect(formatDuration(3_600_000, "en")).toBe("1 hr");
  expect(formatDuration(3_900_000, "en")).toBe("1 hr 5 min");
  // A minute-and-a-bit rounds rather than truncating to a misleading "1 min".
  expect(formatDuration(100_000, "en")).toBe("2 min");
});

test("nonsense lengths do not render as negative or NaN", () => {
  expect(formatDuration(-5, "en")).toBe("0 sec");
});

test("an avatar initial is one grapheme, upper-cased", () => {
  expect(speakerInitial("priya")).toBe("P");
  expect(speakerInitial("  Ünal ")).toBe("Ü");
  expect(speakerInitial("")).toBe("?");
});

test("meetings are grouped under the same day names as History", () => {
  const now = new Date(2026, 9, 1, 20, 0, 0);
  const at = (daysAgo: number, hour: number) =>
    new Date(2026, 9, 1 - daysAgo, hour).getTime() / 1000;
  const groups = groupByDay(
    [at(0, 18), at(0, 9), at(1, 15), at(3, 10), at(30, 10)],
    (seconds) => seconds,
    "en",
    { today: "Today", yesterday: "Yesterday" },
    now,
  );
  expect(groups.map((group) => group.label)).toEqual([
    "Today",
    "Yesterday",
    "Monday",
    "Tue, September 1",
  ]);
  expect(groups[0].items).toHaveLength(2);
});
