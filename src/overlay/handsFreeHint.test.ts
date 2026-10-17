import { describe, expect, test } from "bun:test";
import {
  HINT_TIMES,
  hintDue,
  markHintLearned,
  markHintShown,
  readHint,
} from "./handsFreeHint";

const memory = () => {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => void values.set(key, value),
  };
};

describe("hands-free hint", () => {
  test("shows a few times, then never again", () => {
    const storage = memory();
    for (let i = 0; i < HINT_TIMES; i++) {
      expect(hintDue(readHint(storage))).toBe(true);
      markHintShown(storage);
    }
    expect(hintDue(readHint(storage))).toBe(false);
  });

  test("stops at once when the user goes hands-free", () => {
    const storage = memory();
    markHintShown(storage);
    markHintLearned(storage);
    expect(readHint(storage)).toEqual({ shown: 1, learned: true });
    expect(hintDue(readHint(storage))).toBe(false);
  });

  test("a missing or damaged value starts over", () => {
    const storage = memory();
    expect(readHint(storage)).toEqual({ shown: 0, learned: false });
    storage.setItem("speakoflow.overlay.handsFreeHint", "{not json");
    expect(readHint(storage)).toEqual({ shown: 0, learned: false });
    storage.setItem(
      "speakoflow.overlay.handsFreeHint",
      JSON.stringify({ shown: -3, learned: "yes" }),
    );
    expect(readHint(storage)).toEqual({ shown: 0, learned: false });
    expect(readHint(null)).toEqual({ shown: 0, learned: false });
  });
});
