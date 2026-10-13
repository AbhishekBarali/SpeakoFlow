import { describe, expect, test } from "bun:test";
import type { AppSettings } from "@/bindings";
import { extendKeys, handsFree, keysToSave, mainKey } from "./handsFree";

/** Enough settings for the rule: the Windows defaults on SpeakoFlow Keys. */
const settings = (
  bindings: Record<string, string>,
  extra: Partial<AppSettings> = {},
) =>
  ({
    keyboard_implementation: "handy_keys",
    push_to_talk: true,
    assistant_push_to_talk: true,
    assistant_enabled: true,
    post_process_enabled: false,
    post_process_on_dictation: false,
    bindings: Object.fromEntries(
      Object.entries({
        transcribe: "ctrl_left+super",
        transcribe_with_post_process: "ctrl_left+super+shift",
        assistant: "ctrl_left+alt_left",
        assistant_call: "ctrl_left+alt_left+c",
        cancel: "escape",
        hands_free: "space",
        ...bindings,
      }).map(([id, keys]) => [
        id,
        {
          id,
          name: id,
          description: "",
          default_binding: keys,
          current_binding: keys,
        },
      ]),
    ),
    ...extra,
  }) as unknown as AppSettings;

describe("mainKey", () => {
  test("keeps the one key that is not a modifier", () => {
    expect(mainKey("space")).toBe("space");
    expect(mainKey("Space")).toBe("space");
    // Recorded while the dictation keys were held.
    expect(mainKey("ctrl_left+super+space")).toBe("space");
    expect(mainKey("return")).toBe("return");
  });

  test("modifiers alone are no key", () => {
    expect(mainKey("shift")).toBeNull();
    expect(mainKey("ctrl_left+super")).toBeNull();
    expect(mainKey("")).toBeNull();
    expect(mainKey(undefined)).toBeNull();
  });
});

describe("extendKeys", () => {
  test("modifier keys gain the hands-free key", () => {
    expect(extendKeys("ctrl_left+super", "space")).toBe(
      "ctrl_left+super+space",
    );
    expect(extendKeys("fn", "return")).toBe("fn+return");
  });

  test("keys with a main key of their own have none", () => {
    expect(extendKeys("ctrl+space", "space")).toBeNull();
    expect(extendKeys("f13", "space")).toBeNull();
    expect(extendKeys("", "space")).toBeNull();
    expect(extendKeys("ctrl_left+super", "")).toBeNull();
  });
});

describe("handsFree", () => {
  test("the Windows defaults have Space", () => {
    expect(handsFree(settings({}))).toEqual({
      state: "ready",
      keys: "ctrl_left+super+space",
      key: "space",
    });
  });

  test("follows a key the user chose, and a key the user removed", () => {
    expect(handsFree(settings({ hands_free: "return" }))).toMatchObject({
      state: "ready",
      keys: "ctrl_left+super+return",
    });
    expect(handsFree(settings({ hands_free: "" }))).toEqual({ state: "off" });
  });

  test("tapping dictation needs none, whatever the ask does", () => {
    expect(handsFree(settings({}, { push_to_talk: false }))).toEqual({
      state: "tapped",
    });
    expect(
      handsFree(settings({}, { assistant_push_to_talk: false })).state,
    ).toBe("ready");
  });

  test("Linux's Ctrl+Space and the other engine have none", () => {
    expect(handsFree(settings({ transcribe: "ctrl+space" }))).toEqual({
      state: "unavailable",
      reason: "keys",
    });
    expect(
      handsFree(settings({}, { keyboard_implementation: "tauri" })),
    ).toEqual({ state: "unavailable", reason: "engine" });
  });

  test("a live shortcut of the user's own on those keys wins", () => {
    expect(handsFree(settings({ assistant: "super+ctrl+space" }))).toEqual({
      state: "taken",
      keys: "ctrl_left+super+space",
      key: "space",
      by: "assistant",
    });
    // Switched off, the assistant holds no keys.
    expect(
      handsFree(
        settings(
          { assistant: "super+ctrl+space" },
          { assistant_enabled: false },
        ),
      ).state,
    ).toBe("ready");
  });

  test("the cleanup shortcut has its own", () => {
    expect(handsFree(settings({}), "transcribe_with_post_process")).toEqual({
      state: "ready",
      keys: "ctrl_left+super+shift+space",
      key: "space",
    });
  });
});

describe("keysToSave", () => {
  test("the hands-free key keeps only its key", () => {
    expect(keysToSave("hands_free", "ctrl_left+super+space")).toBe("space");
    expect(keysToSave("hands_free", "shift")).toBeNull();
  });

  test("every other shortcut keeps what was pressed", () => {
    expect(keysToSave("transcribe", "ctrl_left+super")).toBe("ctrl_left+super");
  });
});
