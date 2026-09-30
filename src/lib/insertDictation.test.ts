import { describe, expect, test } from "bun:test";
import { acceptsText } from "./insertDictation";

describe("acceptsText", () => {
  test("text fields and editable regions take a dictation", () => {
    expect(acceptsText({ tagName: "TEXTAREA" })).toBe(true);
    expect(acceptsText({ tagName: "INPUT", type: "text" })).toBe(true);
    expect(acceptsText({ tagName: "input", type: "search" })).toBe(true);
    expect(acceptsText({ tagName: "INPUT", type: "" })).toBe(true);
    expect(acceptsText({ tagName: "DIV", isContentEditable: true })).toBe(true);
  });

  test("a locked field is left alone", () => {
    expect(acceptsText({ tagName: "TEXTAREA", disabled: true })).toBe(false);
    expect(
      acceptsText({ tagName: "INPUT", type: "text", readOnly: true }),
    ).toBe(false);
  });

  test("controls that are not text, and nothing focused, take nothing", () => {
    expect(acceptsText({ tagName: "INPUT", type: "checkbox" })).toBe(false);
    expect(acceptsText({ tagName: "INPUT", type: "range" })).toBe(false);
    expect(acceptsText({ tagName: "BUTTON" })).toBe(false);
    expect(acceptsText({ tagName: "BODY" })).toBe(false);
    expect(acceptsText(null)).toBe(false);
  });
});
