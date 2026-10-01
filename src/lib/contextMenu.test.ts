import { describe, expect, test } from "bun:test";
import { wantsBrowserContextMenu } from "./contextMenu";

/** A stand-in for the element a right-click lands on: `closest` resolves to `field`. */
const targetIn = (field: { tagName: string; type?: string } | null) =>
  ({
    closest: () =>
      field && {
        tagName: field.tagName,
        getAttribute: (name: string) =>
          name === "type" ? (field.type ?? null) : null,
      },
  }) as unknown as EventTarget;

describe("wantsBrowserContextMenu", () => {
  test("keeps the menu in text fields, for Paste", () => {
    for (const type of [undefined, "text", "search", "password", "NUMBER"]) {
      expect(
        wantsBrowserContextMenu({
          target: targetIn({ tagName: "INPUT", type }),
        }),
      ).toBe(true);
    }
    expect(
      wantsBrowserContextMenu({ target: targetIn({ tagName: "TEXTAREA" }) }),
    ).toBe(true);
    expect(
      wantsBrowserContextMenu({ target: targetIn({ tagName: "DIV" }) }),
    ).toBe(true); // contenteditable
  });

  test("suppresses it on ticks, toggles, sliders and pickers", () => {
    for (const type of ["checkbox", "radio", "range", "color", "file"]) {
      expect(
        wantsBrowserContextMenu({
          target: targetIn({ tagName: "INPUT", type }),
        }),
      ).toBe(false);
    }
  });

  test("suppresses it on ordinary content", () => {
    expect(wantsBrowserContextMenu({ target: targetIn(null) })).toBe(false);
  });
});
