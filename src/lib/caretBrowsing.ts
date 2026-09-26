/**
 * Keep WebView2's caret browsing from drawing a text cursor in static text.
 *
 * Caret browsing is a Chromium accessibility mode, toggled with F7, that puts a
 * blinking insertion caret into any text you click — a label, a history row, a
 * heading — as if the whole window were a document. In WebView2 the toggle
 * shows no confirmation and the choice is saved in the profile, so one stray F7
 * leaves every window of the app showing a caret in text that cannot be edited,
 * across restarts. The backend resets the saved preference at launch
 * (`webview_prefs.rs`); this covers the rest:
 *
 * - `caret-color: transparent` on the document, restored for everything that
 *   really is editable. A caret-browsing caret is painted in the text's
 *   `caret-color`, so it becomes invisible while a real field keeps its cursor.
 *   The rules sit in `:where()` so any component that sets its own caret colour
 *   still wins.
 * - F7 is swallowed before the browser acts on it, so the mode cannot be
 *   switched back on from inside the app. Nothing here uses F7.
 *
 * Idempotent, so every window's entry point can call it.
 */

const STYLE_ID = "no-caret-browsing";

export const CARET_BROWSING_CSS = `
:where(:root) { caret-color: transparent; }
:where(input, textarea, select, [contenteditable]:not([contenteditable="false"])) {
  caret-color: auto;
}
`;

/** True for the key that toggles caret browsing, with or without modifiers. */
export const isCaretBrowsingToggle = (event: Pick<KeyboardEvent, "key">) =>
  event.key === "F7";

let installed = false;

export const suppressCaretBrowsing = (doc: Document = document): void => {
  if (installed || typeof doc === "undefined") return;
  installed = true;

  if (!doc.getElementById(STYLE_ID)) {
    const style = doc.createElement("style");
    style.id = STYLE_ID;
    style.textContent = CARET_BROWSING_CSS;
    doc.head.appendChild(style);
  }

  doc.defaultView?.addEventListener(
    "keydown",
    (event) => {
      if (isCaretBrowsingToggle(event)) event.preventDefault();
    },
    { capture: true },
  );
};
