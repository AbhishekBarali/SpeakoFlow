import { describe, expect, test } from "bun:test";
import {
  canonicalShortcut,
  shortcutCombos,
  shortcutUsingKeys,
} from "./shortcutConflict";

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

  test("a mouse button is the same button by any of its names", () => {
    expect(canonicalShortcut("back")).toBe(canonicalShortcut("mousex1"));
    expect(canonicalShortcut("Mouse5+ctrl_left")).toBe(
      canonicalShortcut("ctrl_left+mousex2"),
    );
    expect(canonicalShortcut("mmb")).toBe("mousemiddle");
  });
});

describe("other keys", () => {
  const withOther = {
    ...bindings,
    transcribe: {
      current_binding: "ctrl_left+super",
      alternate_binding: "mousex1",
    },
  };

  test("another shortcut's other keys are as taken as its own", () => {
    expect(shortcutUsingKeys(withOther, "back", "assistant", "alternate")).toBe(
      "transcribe",
    );
    expect(shortcutUsingKeys(withOther, "mousex1", "assistant")).toBe(
      "transcribe",
    );
  });

  test("a shortcut's two sets of keys must differ", () => {
    expect(
      shortcutUsingKeys(
        withOther,
        "ctrl_left+super",
        "transcribe",
        "alternate",
      ),
    ).toBe("transcribe");
    expect(
      shortcutUsingKeys(withOther, "mouse4", "transcribe", "primary"),
    ).toBe("transcribe");
    // Its own keys, set again, are its own.
    expect(
      shortcutUsingKeys(withOther, "ctrl_left+super", "transcribe", "primary"),
    ).toBeNull();
  });

  test("a shortcut that is turned off holds no other keys either", () => {
    const off = {
      ...bindings,
      cancel: { current_binding: "", alternate_binding: "f13" },
    };
    expect(shortcutUsingKeys(off, "f13", "transcribe", "alternate")).toBeNull();
  });
});

describe("shortcutCombos", () => {
  test("own keys first, then the other keys, and none when off", () => {
    expect(
      shortcutCombos({
        current_binding: "f13",
        alternate_binding: " mousex2 ",
      }),
    ).toEqual(["f13", "mousex2"]);
    expect(shortcutCombos({ current_binding: "f13" })).toEqual(["f13"]);
    expect(
      shortcutCombos({ current_binding: "", alternate_binding: "mousex2" }),
    ).toEqual([]);
  });
});
