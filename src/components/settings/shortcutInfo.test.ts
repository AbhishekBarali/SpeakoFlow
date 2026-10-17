import { describe, expect, test } from "bun:test";
import type { AppSettings } from "@/bindings";
import { shortcutInfo } from "./shortcutInfo";

const t = (key: string) => key;
const settings = (extra: Partial<AppSettings> = {}) =>
  ({
    push_to_talk: true,
    assistant_push_to_talk: true,
    post_process_enabled: false,
    post_process_on_dictation: false,
    ...extra,
  }) as AppSettings;

describe("shortcutInfo", () => {
  test("dictation and the ask say how they are pressed, each on its own setting", () => {
    expect(shortcutInfo(t, settings(), "transcribe")).toBe(
      "home.shortcuts.dictate.whatHold",
    );
    const tapped = settings({ push_to_talk: false });
    expect(shortcutInfo(t, tapped, "transcribe")).toBe(
      "home.shortcuts.dictate.whatTap",
    );
    expect(shortcutInfo(t, tapped, "assistant")).toBe(
      "home.shortcuts.ask.whatHold",
    );
    expect(
      shortcutInfo(t, settings({ assistant_push_to_talk: false }), "assistant"),
    ).toBe("home.shortcuts.ask.whatTap");
  });

  test("dictation that cleans up every time says so", () => {
    expect(
      shortcutInfo(
        t,
        settings({
          post_process_enabled: true,
          post_process_on_dictation: true,
        }),
        "transcribe",
      ),
    ).toBe("home.shortcuts.cleanup.what");
  });

  test("the rest have one sentence each, and unknown ids none", () => {
    expect(shortcutInfo(t, settings(), "assistant_call")).toBe(
      "home.shortcuts.call.what",
    );
    expect(shortcutInfo(t, settings(), "cancel")).toBe(
      "home.shortcuts.cancel.what",
    );
    expect(shortcutInfo(t, null, "transcribe")).toBe(
      "home.shortcuts.dictate.whatHold",
    );
    expect(shortcutInfo(t, settings(), "hands_free")).toBeUndefined();
  });
});
