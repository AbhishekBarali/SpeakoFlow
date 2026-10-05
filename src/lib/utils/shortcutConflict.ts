/**
 * Which other shortcut already uses a set of keys.
 *
 * Two actions cannot share keys: the hotkey engine refuses the second
 * registration, and its own error ("Hotkey already registered:
 * CtrlLeft+OptLeft (id: HotkeyId(2))") names neither the keys in plain words
 * nor the action that holds them. The shortcut editors check here first, so
 * the message can say which shortcut is in the way.
 */

/** Spellings of the same modifier that the engines accept interchangeably. */
const MODIFIER_ALIASES: Record<string, string> = {
  control: "ctrl",
  option: "alt",
  opt: "alt",
  cmd: "super",
  command: "super",
  meta: "super",
  win: "super",
  windows: "super",
};

/**
 * One spelling per key combination: lower case, aliases folded, order
 * ignored. A side stays part of the key (`ctrl_left` is not `ctrl`), because
 * the engine registers those as different hotkeys.
 */
export const canonicalShortcut = (binding: string): string =>
  binding
    .split("+")
    .map((part) => part.trim().toLowerCase())
    .filter(Boolean)
    .map((part) => {
      const side = part.match(/_(left|right)$/)?.[0] ?? "";
      const base = side ? part.slice(0, -side.length) : part;
      return `${MODIFIER_ALIASES[base] ?? base}${side}`;
    })
    .sort()
    .join("+");

/**
 * The id of the shortcut other than `exceptId` whose keys are `keys`, or
 * `null`. A shortcut that is turned off (no keys) never counts.
 */
export const shortcutUsingKeys = (
  bindings: Record<string, { current_binding: string } | undefined>,
  keys: string,
  exceptId: string,
): string | null => {
  const wanted = canonicalShortcut(keys);
  if (!wanted) return null;
  for (const [id, binding] of Object.entries(bindings)) {
    if (id === exceptId || !binding) continue;
    if (canonicalShortcut(binding.current_binding) === wanted) return id;
  }
  return null;
};
