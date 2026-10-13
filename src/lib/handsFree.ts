import type { AppSettings } from "@/bindings";

/**
 * The dictation keys plus Space, as Settings and Home describe them. Mirrors
 * `src-tauri/src/hands_free.rs` (`space_keys` / `space_keys_for`), which is
 * what actually registers them: the keys exist only for a dictation shortcut
 * of modifier keys alone, only on the SpeakoFlow Keys engine, and only while no
 * other live shortcut is on those very keys.
 */

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

const pressed = (keys: string) =>
  keys
    .split("+")
    .map(keyName)
    .filter(Boolean)
    .map((key) => ALIASES[key] ?? key)
    .sort()
    .join("+");

/** `ctrl_left+super` gives `ctrl_left+super+space`; keys with a main key of
 *  their own (Linux's `ctrl+space`) give nothing. */
export function spaceKeys(keys: string | null | undefined): string | null {
  const parts = (keys ?? "")
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);
  if (
    parts.length === 0 ||
    !parts.every((part) => MODIFIERS.has(keyName(part)))
  )
    return null;
  return `${parts.join("+")}+space`;
}

export type HandsFreeSpace =
  /** Registered on `keys`, or would be once switched on. */
  | { state: "ready"; keys: string }
  /** These dictation keys, or this engine, cannot have one. */
  | { state: "unavailable" }
  /** Another shortcut the user chose is on `keys`, so it steps aside. */
  | { state: "taken"; keys: string };

/** Whether a shortcut is registered: the separate cleanup shortcut only while
 *  it is the one that cleans up, the assistant's only while it is on, and
 *  Cancel only during a recording (never counted here). */
const isLive = (settings: AppSettings, id: string) => {
  if (id === "cancel") return false;
  if (id === "transcribe_with_post_process")
    return (
      !!settings.post_process_enabled && !settings.post_process_on_dictation
    );
  if (id === "assistant" || id === "assistant_call")
    return settings.assistant_enabled ?? true;
  return true;
};

/** The hands-free key of dictation shortcut `bindingId`, as it stands. */
export function handsFreeSpace(
  settings: AppSettings | null | undefined,
  bindingId = "transcribe",
): HandsFreeSpace {
  if (!settings || settings.keyboard_implementation !== "handy_keys")
    return { state: "unavailable" };
  const keys = spaceKeys(settings.bindings?.[bindingId]?.current_binding);
  if (!keys) return { state: "unavailable" };
  const taken = Object.entries(settings.bindings ?? {}).some(
    ([id, binding]) =>
      id !== bindingId &&
      isLive(settings, id) &&
      !!binding?.current_binding &&
      pressed(binding.current_binding) === pressed(keys),
  );
  return taken ? { state: "taken", keys } : { state: "ready", keys };
}
