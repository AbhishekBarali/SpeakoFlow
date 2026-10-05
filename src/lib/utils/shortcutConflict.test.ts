import { describe, expect, test } from "bun:test";
import { canonicalShortcut, shortcutUsingKeys } from "./shortcutConflict";

const bindings = {
  transcribe: { current_binding: "ctrl_left+super" },
  assistant: { current_binding: "ctrl_left+alt_left" },
  assistant_call: { current_binding: "shift+super+s" },
  cancel: { current_binding: "escape" },
};

describe("shortcutUsingKeys", () => {
  test("names the shortcut that already has the keys", () => {
    expect(
      shortcutUsingKeys(bindings, "ctrl_left+alt_left", "assistant_call"),
    ).toBe("assistant");
  });

  test("a combination that only starts with another shortcut's keys is free", () => {
    // The reported case: Ctrl+Alt+C for the call while Ctrl+Alt is the ask.
    expect(
      shortcutUsingKeys(bindings, "ctrl_left+alt_left+c", "assistant_call"),
    ).toBeNull();
  });

  test("the shortcut being edited does not conflict with itself", () => {
    expect(
      shortcutUsingKeys(bindings, "shift+super+s", "assistant_call"),
    ).toBeNull();
  });

  test("order and aliases do not hide a conflict", () => {
    expect(shortcutUsingKeys(bindings, "win+shift+S", "transcribe")).toBe(
      "assistant_call",
    );
  });

  test("a side is part of the key", () => {
    expect(
      shortcutUsingKeys(bindings, "ctrl+alt", "assistant_call"),
    ).toBeNull();
  });

  test("a shortcut that is turned off holds no keys", () => {
    const off = { ...bindings, assistant_call: { current_binding: "" } };
    expect(shortcutUsingKeys(off, "", "transcribe")).toBeNull();
    expect(shortcutUsingKeys(off, "c", "transcribe")).toBeNull();
  });
});

describe("canonicalShortcut", () => {
  test("folds case, aliases and order", () => {
    expect(canonicalShortcut("Control+Option+Space")).toBe(
      canonicalShortcut("space+alt+ctrl"),
    );
    expect(canonicalShortcut("command_left+k")).toBe("k+super_left");
  });
});
