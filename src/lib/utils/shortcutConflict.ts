/**
 * Which other shortcut already uses a set of keys.
 *
 * Two actions cannot share keys: the hotkey engine refuses the second
 * registration, and its own error ("Hotkey already registered:
 * CtrlLeft+OptLeft (id: HotkeyId(2))") names neither the keys in plain words
 * nor the action that holds them. The shortcut editors check here first, so
 * the message can say which shortcut is in the way. `combos::conflict` in
 * `src-tauri/src/shortcut/combos.rs` makes the same call in the backend.
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

/** Every name the engine reads a mouse button by, folded to the one it writes. */
const MOUSE_ALIASES: Record<string, string> = {
  mouse1: "mouseleft",
  leftclick: "mouseleft",
  lmb: "mouseleft",
  mouse2: "mouseright",
  rightclick: "mouseright",
  rmb: "mouseright",
  mouse3: "mousemiddle",
  middleclick: "mousemiddle",
  mmb: "mousemiddle",
  mouse4: "mousex1",
  back: "mousex1",
  xbutton1: "mousex1",
  mouse5: "mousex2",
  forward: "mousex2",
  xbutton2: "mousex2",
};

/** A shortcut as the settings store keeps it. */
type StoredShortcut = {
  current_binding: string;
  alternate_binding?: string | null;
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
      return `${MODIFIER_ALIASES[base] ?? MOUSE_ALIASES[base] ?? base}${side}`;
    })
    .sort()
    .join("+");

/** The hands-free key is one key that only ever goes on top of the dictation
 *  keys (`src/lib/handsFree.ts`), so Space there is no shortcut on Space. */
const ADD_ON_KEYS = new Set(["hands_free"]);

/**
 * Every set of keys a shortcut answers to: its own and, when it has them, its
 * other keys. A shortcut that is turned off (no keys of its own) answers to
 * neither.
 */
export const shortcutCombos = (binding: StoredShortcut): string[] => {
  const own = binding.current_binding.trim();
  if (!own) return [];
  const other = binding.alternate_binding?.trim() ?? "";
  return other ? [own, other] : [own];
};

/**
 * The id of the shortcut that already answers to `keys`, if `exceptId`'s
 * `slot` were given them, or `null`.
 *
 * Another shortcut holds keys through its own keys and its other keys alike.
 * `exceptId` itself is the answer when its other set already is `keys`: a
 * shortcut's two sets of keys must differ, or the second does nothing. The
 * hands-free key counts on neither side.
 */
export const shortcutUsingKeys = (
  bindings: Record<string, StoredShortcut | undefined>,
  keys: string,
  exceptId: string,
  slot: "primary" | "alternate" = "primary",
): string | null => {
  const wanted = canonicalShortcut(keys);
  if (!wanted || ADD_ON_KEYS.has(exceptId)) return null;
  const own = bindings[exceptId];
  if (own) {
    const otherSet =
      slot === "primary" ? (own.alternate_binding ?? "") : own.current_binding;
    if (otherSet.trim() && canonicalShortcut(otherSet) === wanted) {
      return exceptId;
    }
  }
  for (const [id, binding] of Object.entries(bindings)) {
    if (id === exceptId || ADD_ON_KEYS.has(id) || !binding) continue;
    if (
      shortcutCombos(binding).some(
        (combo) => canonicalShortcut(combo) === wanted,
      )
    ) {
      return id;
    }
  }
  return null;
};
