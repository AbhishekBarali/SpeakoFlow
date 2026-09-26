import { describe, expect, test } from "bun:test";
import { splitTip } from "./tipText";

describe("splitTip", () => {
  test("keeps a short sentence whole", () => {
    expect(splitTip("Reads replies aloud during a call.")).toEqual({
      lead: "Reads replies aloud during a call.",
      rest: null,
    });
  });

  test("leads with the first sentence of a long explanation", () => {
    const text =
      "When you fix a word SpeakoFlow got wrong, it remembers your spelling for next time. To do that it reads the text field you just dictated into, for about a minute afterwards.";
    expect(splitTip(text)).toEqual({
      lead: "When you fix a word SpeakoFlow got wrong, it remembers your spelling for next time.",
      rest: "To do that it reads the text field you just dictated into, for about a minute afterwards.",
    });
  });

  test("does not split on abbreviations or decimals", () => {
    const text =
      "Use a model like Gemma 4 E2B, e.g. for quick replies at 1.5 GB of memory. The rest of this explanation goes on for a while so that it is long enough to be split.";
    expect(splitTip(text).lead).toBe(
      "Use a model like Gemma 4 E2B, e.g. for quick replies at 1.5 GB of memory.",
    );
  });

  test("leaves a long single sentence alone rather than cutting a word", () => {
    const text =
      "A single very long sentence with no full stop in the middle that keeps going and going well past the length where the tooltip would normally prefer to stop and offer more";
    expect(splitTip(text)).toEqual({ lead: text, rest: null });
  });

  test("does not offer More for a tiny remainder", () => {
    expect(
      splitTip(
        "Condenses long chats so the assistant never forgets the start of a conversation. Always on.",
      ).rest,
    ).toBeNull();
  });

  test("handles question and exclamation marks and quotes", () => {
    const text =
      'Say "remind me in twenty minutes" and it will. Everything else about how reminders are stored, fired, snoozed and re-shown after a restart lives here.';
    expect(splitTip(text).lead).toBe(
      'Say "remind me in twenty minutes" and it will.',
    );
  });

  test("is empty-safe", () => {
    expect(splitTip("")).toEqual({ lead: "", rest: null });
    expect(splitTip("   ")).toEqual({ lead: "", rest: null });
  });
});
