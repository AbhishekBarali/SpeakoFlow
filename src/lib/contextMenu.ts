import type { MouseEvent as ReactMouseEvent } from "react";

/**
 * Keep the web view's own context menu out of the app.
 *
 * Right-clicking anywhere in a window used to open the browser's menu — Back,
 * Refresh, Save as, Print, More tools — which is the tell that the app is a web
 * page, and Refresh reloads the window out from under whatever it was showing.
 * The keyboard half of this (Ctrl+R, F5, Ctrl+P…) is already off in every
 * window (`disable_webview2_browser_accelerators` in `lib.rs`); this is the
 * mouse half. macOS (WKWebView) and Linux (WebKitGTK) offer Reload in the same
 * menu, so it is done here rather than through a WebView2 setting.
 *
 * Two places keep it, because there the menu is the right answer:
 * - a text field, for right-click → Paste;
 * - text the user has selected, for right-click → Copy.
 *
 * "Text field" is deliberately narrow. Checkboxes, radios, sliders, colour
 * pickers and `<select>` are `input`-shaped too, but nothing in the menu
 * applies to them, and matching them is how a right-click on a settings tick
 * used to open Back / Print / More tools / Inspect.
 *
 * What the menu then *contains* is trimmed natively on Windows
 * (`filter_webview2_context_menu` in `lib.rs`), so even where it does open it
 * offers only the editing commands, never Print, More tools or Inspect.
 *
 * A development build also opens it with Shift held, so Inspect is still a
 * right-click away.
 */
const TEXT_INPUT_TYPES = new Set([
  "",
  "text",
  "search",
  "email",
  "url",
  "tel",
  "password",
  "number",
]);

function isTextEntry(target: Element | null): boolean {
  const field = target?.closest?.(
    "input, textarea, [contenteditable]:not([contenteditable='false'])",
  );
  if (!field) return false;
  if (field.tagName === "INPUT") {
    return TEXT_INPUT_TYPES.has(
      (field.getAttribute("type") ?? "").toLowerCase(),
    );
  }
  return true;
}

export function wantsBrowserContextMenu(event: {
  target: EventTarget | null;
  shiftKey?: boolean;
}): boolean {
  if (import.meta.env?.DEV && event.shiftKey) return true;
  if (isTextEntry(event.target as Element | null)) return true;
  const selection =
    typeof window !== "undefined" ? window.getSelection?.() : null;
  return !!selection && !selection.isCollapsed && !!selection.toString().trim();
}

/** A React `onContextMenu` handler applying the same rule. */
export function preventBrowserContextMenu(event: ReactMouseEvent): void {
  if (!wantsBrowserContextMenu(event)) event.preventDefault();
}

let installed = false;

/**
 * Apply the rule to the whole window. Idempotent, so every entry point calls it
 * next to `suppressCaretBrowsing`. Registered in the bubble phase, so a
 * component that draws its own menu on right-click still sees the event first.
 */
export function suppressBrowserContextMenu(target: Window = window): void {
  if (installed || typeof target === "undefined") return;
  installed = true;
  target.addEventListener("contextmenu", (event) => {
    if (!wantsBrowserContextMenu(event)) event.preventDefault();
  });
}
