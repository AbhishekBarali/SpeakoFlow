import { describe, expect, test } from "bun:test";
import { runsOf } from "./shareCard";

const figures = (text: string) =>
  runsOf(text)
    .filter((run) => !run.unit)
    .map((run) => run.text);

describe("runsOf", () => {
  test("splits ASCII figures from their units", () => {
    expect(runsOf("8 hr 11 min")).toEqual([
      { text: "8 ", unit: false },
      { text: "hr", unit: true },
      { text: " 11 ", unit: false },
      { text: "min", unit: true },
    ]);
    expect(runsOf("27.6K")).toEqual([
      { text: "27.6", unit: false },
      { text: "K", unit: true },
    ]);
    expect(runsOf("1,071")).toEqual([{ text: "1,071", unit: false }]);
  });

  test("keeps a no-break or narrow no-break group space inside the figure", () => {
    expect(figures("27\u00A0676")).toEqual(["27\u00A0676"]);
    expect(figures("27\u202F676 mots")).toEqual(["27\u202F676 "]);
  });

  test("reads Arabic-Indic digits and separators as a figure", () => {
    expect(runsOf("٢٧٫٦ ألف")).toEqual([
      { text: "٢٧٫٦ ", unit: false },
      { text: "ألف", unit: true },
    ]);
    expect(figures("٢٧٬٦٧٦")).toEqual(["٢٧٬٦٧٦"]);
  });

  test("a run of spaces alone is spacing, not a figure", () => {
    expect(runsOf("万\u3000")).toEqual([
      { text: "万", unit: true },
      { text: "\u3000", unit: true },
    ]);
  });
});
