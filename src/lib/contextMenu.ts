import type { MouseEvent } from "react";

/**
 * Keep WebView2's browser context menu off a floating surface.
 *
 * On a HUD that menu is never the right answer — Back, Refresh, Save as, Print —
 * and "Refresh" reloads the window out from under whatever it was showing. A menu
 * opened from a window that cannot take the foreground is also a trap of its own:
 * it holds the mouse until it is dismissed, and the click that dismisses it is
 * spent doing that rather than reaching the app underneath.
 *
 * Text fields keep it, since right-click → Paste is something people expect there.
 * The assistant panel does the same with a window listener
 * (`useSuppressContextMenu` in `assistant/hitRegion.ts`).
 */
export function preventBrowserContextMenu(event: MouseEvent): void {
  const target = event.target as Element | null;
  if (target?.closest?.("input, textarea, [contenteditable='true']")) return;
  event.preventDefault();
}
