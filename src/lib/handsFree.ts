import type { AppSettings } from "@/bindings";

/**
 * The hands-free key, as Settings and Home describe it. Mirrors
 * `src-tauri/src/hands_free.rs` (`main_key`, `extend`, `keys_for`), which is
 * what registers it: one key, pressed while the dictation keys are held, that
 * keeps the dictation listening after they are let go. It goes on top of a
 * dictation shortcut of modifier keys alone, on the SpeakoFlow Keys engine,
 * while dictation is held to talk, and only where no other live shortcut is on
 * those keys.
 */

/** Binding id of the hands-free key. Mirrors `HANDS_FREE_BINDING_ID`. */
export const HANDS_FREE_ID = "hands_free";

const MODIFIERS = new Set([
  "ctrl",
  "control",
  "shift",
  "alt",
  "option",
  "opt",
  "super",
  "cmd",
  "command",
  "meta",
  "win",
  "windows",
  "fn",
]);

/** What a press of the key would match, whichever side it is on. */
const ALIASES: Record<string, string> = {
  control: "ctrl",
  option: "alt",
  opt: "alt",
  cmd: "super",
  command: "super",
  meta: "super",
  win: "super",
  windows: "super",
};

const keyName = (part: string) =>
  part
    .trim()
    .toLowerCase()
    .replace(/_(left|right)$/, "");

const parts = (keys: string | null | undefined) =>
  (keys ?? "")
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);

const pressed = (keys: string) =>
  parts(keys)
    .map(keyName)
    .map((key) => ALIASES[key] ?? key)
    .sort()
    .join("+");

/**
 * The one main key in `keys`, which is all the hands-free key keeps:
 * recorded while the dictation keys were held, `ctrl_left+super+space` is
 * Space. `null` for modifiers alone.
 */
export function mainKey(keys: string | null | undefined): string | null {
  const main = parts(keys).filter((part) => !MODIFIERS.has(keyName(part)));
  return main.length === 1 ? main[0].toLowerCase() : null;
}

/** `ctrl_left+super` with `space` gives `ctrl_left+super+space`; keys with a
 *  main key of their own (Linux's `ctrl+space`) give nothing. */
export function extendKeys(
  keys: string | null | undefined,
  key: string | null | undefined,
): string | null {
  const own = parts(keys);
  if (own.length === 0 || !own.every((part) => MODIFIERS.has(keyName(part))))
    return null;
  const main = mainKey(key);
  return main ? `${own.join("+")}+${main}` : null;
}

export type HandsFree =
  /** Registered on `keys`: the dictation keys plus `key`. */
  | { state: "ready"; keys: string; key: string }
  /** The key was removed. */
  | { state: "off" }
  /** Tap to toggle: every dictation already is hands-free. */
  | { state: "tapped" }
  /** This engine, or these dictation keys, cannot have one. */
  | { state: "unavailable"; reason: "engine" | "keys" }
  /** Shortcut `by`, chosen by the user, is on `keys`, so it steps aside. */
  | { state: "taken"; keys: string; key: string; by: string };

/** Whether a shortcut is registered: the separate cleanup shortcut only while
 *  it is the one that cleans up, the assistant's only while it is on, and
 *  Cancel only during a recording (never counted here). */
const isLive = (settings: AppSettings, id: string) => {
  if (id === "cancel" || id === HANDS_FREE_ID) return false;
  if (id === "transcribe_with_post_process")
    return (
      !!settings.post_process_enabled && !settings.post_process_on_dictation
    );
  if (id === "assistant" || id === "assistant_call")
    return settings.assistant_enabled ?? true;
  return true;
};

/** The hands-free key of dictation shortcut `bindingId`, as it stands. */
export function handsFree(
  settings: AppSettings | null | undefined,
  bindingId = "transcribe",
): HandsFree {
  if (!settings) return { state: "off" };
  if (!(settings.push_to_talk ?? true)) return { state: "tapped" };
  const key = mainKey(settings.bindings?.[HANDS_FREE_ID]?.current_binding);
  if (!key) return { state: "off" };
  if (settings.keyboard_implementation !== "handy_keys")
    return { state: "unavailable", reason: "engine" };
  const keys = extendKeys(settings.bindings?.[bindingId]?.current_binding, key);
  if (!keys) return { state: "unavailable", reason: "keys" };
  const by = Object.entries(settings.bindings ?? {}).find(
    ([id, binding]) =>
      id !== bindingId &&
      isLive(settings, id) &&
      // Its other keys count too: either set fires the shortcut.
      [binding?.current_binding, binding?.alternate_binding].some(
        (other) => !!other && pressed(other) === pressed(keys),
      ),
  )?.[0];
  return by ? { state: "taken", keys, key, by } : { state: "ready", keys, key };
}

/**
 * What the shortcut editor saves for `shortcutId` when `keys` were pressed:
 * the keys as they are, except for the hands-free key, which keeps only its
 * main key and is `null` when there is none (modifiers alone).
 */
export function keysToSave(shortcutId: string, keys: string): string | null {
  return shortcutId === HANDS_FREE_ID ? mainKey(keys) : keys;
}
