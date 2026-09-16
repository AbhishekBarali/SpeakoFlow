import { describe, expect, it } from "bun:test";
import { overdueBucket } from "./ReminderPopup";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;

describe("overdueBucket", () => {
  it("says 'just now' for the whole first minute", () => {
    // The popup appears the instant a reminder fires, so this is the state the
    // user sees first. Rounding 30 seconds up to "1 min ago" is a small lie, and
    // small lies about timing are what make people stop trusting an alarm.
    expect(overdueBucket(0, 0)).toEqual({ unit: "now", value: 0 });
    expect(overdueBucket(0, 30_000)).toEqual({ unit: "now", value: 0 });
    expect(overdueBucket(0, 59_999)).toEqual({ unit: "now", value: 0 });
  });

  it("counts whole minutes, then whole hours", () => {
    expect(overdueBucket(0, MINUTE)).toEqual({ unit: "minutes", value: 1 });
    expect(overdueBucket(0, 45 * MINUTE)).toEqual({
      unit: "minutes",
      value: 45,
    });
    // The unit changes exactly at the hour, not before it.
    expect(overdueBucket(0, 59 * MINUTE)).toEqual({
      unit: "minutes",
      value: 59,
    });
    expect(overdueBucket(0, HOUR)).toEqual({ unit: "hours", value: 1 });
    expect(overdueBucket(0, 3 * HOUR + 40 * MINUTE)).toEqual({
      unit: "hours",
      value: 3,
    });
  });

  it("never reports negative time for a clock that moved backwards", () => {
    // A corrected system clock or a resume from sleep can put `now` behind the
    // due time. "-3 min ago" in a notification reads as a bug in the app.
    expect(overdueBucket(10 * MINUTE, 0)).toEqual({ unit: "now", value: 0 });
  });

  it("treats an unparseable due time as just now rather than as NaN", () => {
    // `Date.parse` of a malformed timestamp is NaN, and the popup would
    // otherwise render "NaN min ago" next to a perfectly valid reminder.
    expect(overdueBucket(Number.NaN, Date.now()).unit).toBe("now");
  });
});
