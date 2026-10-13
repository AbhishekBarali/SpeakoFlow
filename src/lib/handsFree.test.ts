import { describe, expect, test } from "bun:test";
import type { AppSettings } from "@/bindings";
import { handsFreeSpace, spaceKeys } from "./handsFree";

/** Enough settings for the rule: the Windows defaults on SpeakoFlow Keys. */
const settings = (
  bindings: Record<string, string>,
  extra: Partial<AppSettings> = {},
) =>
  ({
    keyboard_implementation: "handy_keys",
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

describe("spaceKeys", () => {
  test("modifier keys gain Space", () => {
    expect(spaceKeys("ctrl_left+super")).toBe("ctrl_left+super+space");
    expect(spaceKeys("fn")).toBe("fn+space");
  });

  test("keys with a main key of their own have none", () => {
    expect(spaceKeys("ctrl+space")).toBeNull();
    expect(spaceKeys("f13")).toBeNull();
    expect(spaceKeys("")).toBeNull();
    expect(spaceKeys(undefined)).toBeNull();
  });
});

describe("handsFreeSpace", () => {
  test("the Windows defaults have one", () => {
    expect(handsFreeSpace(settings({}))).toEqual({
      state: "ready",
      keys: "ctrl_left+super+space",
    });
  });

  test("Linux's Ctrl+Space and the other engine have none", () => {
    expect(handsFreeSpace(settings({ transcribe: "ctrl+space" }))).toEqual({
      state: "unavailable",
    });
    expect(
      handsFreeSpace(settings({}, { keyboard_implementation: "tauri" })),
    ).toEqual({ state: "unavailable" });
  });

  test("a live shortcut of the user's own on those keys wins", () => {
    expect(handsFreeSpace(settings({ assistant: "super+ctrl+space" }))).toEqual(
      { state: "taken", keys: "ctrl_left+super+space" },
    );
    // Switched off, the assistant holds no keys.
    expect(
      handsFreeSpace(
        settings(
          { assistant: "super+ctrl+space" },
          { assistant_enabled: false },
        ),
      ).state,
    ).toBe("ready");
  });
});
