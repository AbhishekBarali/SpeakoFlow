import { createContext, useContext } from "react";

/**
 * Where a floating layer — a menu, a tooltip, a dialog — is rendered.
 *
 * Pages are kept mounted and frozen while hidden (see `shell/Freeze.tsx`), and
 * React hides a frozen page by hiding its top-level DOM nodes. A portal into
 * `document.body` is not one of those nodes, so a menu that was open when its
 * page froze stayed on screen, on top of the next page — and could not close,
 * because a frozen tree never commits the state update that would close it.
 * That is how the assistant page's voice menu ended up floating, dead, over
 * the Models page after "Browse all models".
 *
 * Each page layer therefore provides its own portal root, inside the element
 * that gets hidden and made inert with the page. Everything outside a page (the
 * Settings dialog, the toaster) still renders into `document.body`.
 */
const PortalTargetContext = createContext<HTMLElement | null>(null);

export const PortalTargetProvider = PortalTargetContext.Provider;

/** The element floating layers should portal into. Null outside a DOM. */
export const usePortalTarget = (): HTMLElement | null => {
  const target = useContext(PortalTargetContext);
  if (target) return target;
  return typeof document === "undefined" ? null : document.body;
};
