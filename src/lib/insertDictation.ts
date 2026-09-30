/**
 * Put a dictation into whichever field of this window has the caret.
 *
 * The backend sends `dictation-into-focus` instead of pasting when the main
 * window itself is the one in front (see `delivers_in_app` in actions.rs): a
 * synthetic Ctrl+V cannot reach a field in our own webview, because the paste
 * holds the thread the webview needs until the clipboard has been restored.
 */

/** Anything a field-like element exposes that decides whether text can go in. */
export interface EditableLike {
  tagName?: string;
  type?: string;
  disabled?: boolean;
  readOnly?: boolean;
  isContentEditable?: boolean;
}

/** `<input>` types that take free text. A checkbox or a range cannot. */
const TEXT_INPUTS = new Set([
  "",
  "text",
  "search",
  "email",
  "url",
  "tel",
  "password",
]);

export function acceptsText(element: EditableLike | null | undefined): boolean {
  if (!element) return false;
  if (element.isContentEditable) return true;
  if (element.disabled || element.readOnly) return false;
  const tag = (element.tagName ?? "").toUpperCase();
  if (tag === "TEXTAREA") return true;
  if (tag === "INPUT")
    return TEXT_INPUTS.has((element.type ?? "").toLowerCase());
  return false;
}

/**
 * Insert `text` at the caret of the focused field. Returns false, changing
 * nothing, when no field has the caret — the dictation is still in History.
 */
export function insertDictation(text: string): boolean {
  if (!text || typeof document === "undefined") return false;
  const target = document.activeElement as (HTMLElement & EditableLike) | null;
  if (!acceptsText(target)) return false;
  // `insertText` goes through the browser's own editing path: it fires the
  // `input` event React listens to (so a controlled field keeps its state),
  // replaces a selection, and lands on the undo stack like typing does.
  if (document.execCommand("insertText", false, text)) return true;
  // A runtime that refuses the command: set the value the way React expects
  // (through the native setter), then announce it.
  if (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement
  ) {
    const start = target.selectionStart ?? target.value.length;
    const end = target.selectionEnd ?? start;
    const next = target.value.slice(0, start) + text + target.value.slice(end);
    const proto = Object.getPrototypeOf(target) as object;
    Object.getOwnPropertyDescriptor(proto, "value")?.set?.call(target, next);
    target.setSelectionRange(start + text.length, start + text.length);
    target.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  }
  return false;
}
