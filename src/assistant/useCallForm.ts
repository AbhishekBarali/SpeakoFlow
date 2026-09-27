import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/bindings";

/**
 * Which form the live call is in, including the moments in between.
 *
 * Opening and closing the conversation resizes the window, and a window resize
 * is not something a webview can animate: for a frame or two the old picture is
 * drawn at the new window's corner, so whatever is on screen at that instant
 * visibly jumps. That jump is what made the conversation open and close with a
 * lurch. So, like the quick-ask card, nothing is on screen while the window
 * changes shape, and each form animates in only once its window exists:
 *
 *   open    bar ─► leaving (the bar fades out) ─► opening (Rust grows the
 *           window; nothing drawn) ─► open (the panel grows up out of the bar)
 *   close   open ─► closing (the panel folds down into the bar) ─► shrinking
 *           (Rust shrinks the window; nothing drawn) ─► bar (the bar settles in)
 *
 * Rust reports when the new geometry is in place (`assistant-call-frame`). A
 * fallback timer covers a report that never comes, so a lost event costs a
 * moment rather than a call stuck between forms.
 */
export type CallForm =
  | "bar"
  | "leaving"
  | "opening"
  | "open"
  | "closing"
  | "shrinking";

/** Keep in step with the `call-*` animations in `CallBar.css`. */
export const CALL_FORM_MS = {
  /** The bar and bubble fading out before the window grows. */
  leave: 120,
  /** The panel folding down before the window shrinks. */
  close: 220,
  /** How long to wait for Rust's geometry report before going on anyway. */
  fallback: 600,
} as const;

function reducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/** Run after the next paint, so a just-resized webview has laid itself out. */
function afterPaint(run: () => void) {
  if (typeof requestAnimationFrame === "function")
    requestAnimationFrame(() => run());
  else run();
}

export function useCallForm(active: boolean) {
  const [form, setFormState] = useState<CallForm>("bar");
  // Updated with the state rather than on render, so two clicks inside one
  // React batch cannot both see `bar` and queue two resizes.
  const formRef = useRef(form);
  const setForm = useCallback((next: CallForm) => {
    formRef.current = next;
    setFormState(next);
  }, []);
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);

  const clearTimers = useCallback(() => {
    timers.current.forEach(clearTimeout);
    timers.current = [];
  }, []);
  const later = useCallback((ms: number, run: () => void) => {
    timers.current.push(setTimeout(run, ms));
  }, []);

  // The window now has the shape the next form needs: draw it.
  const arrive = useCallback(
    (expanded: boolean) => {
      const waiting = expanded ? "opening" : "shrinking";
      if (formRef.current !== waiting) return;
      clearTimers();
      afterPaint(() => {
        if (formRef.current === waiting) setForm(expanded ? "open" : "bar");
      });
    },
    [clearTimers, setForm],
  );

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen<boolean>("assistant-call-frame", ({ payload }) =>
      arrive(payload === true),
    ).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, [arrive]);

  // A call that ends leaves no form behind: the next one opens as the bar.
  useEffect(() => {
    if (active) return;
    clearTimers();
    setForm("bar");
  }, [active, clearTimers, setForm]);

  useEffect(() => clearTimers, [clearTimers]);

  const resize = useCallback(
    (expanded: boolean) => {
      setForm(expanded ? "opening" : "shrinking");
      void commands.assistantConversationSetExpanded(expanded);
      later(CALL_FORM_MS.fallback, () => arrive(expanded));
    },
    [arrive, later, setForm],
  );

  const expand = useCallback(() => {
    if (formRef.current !== "bar") return;
    setForm("leaving");
    later(reducedMotion() ? 0 : CALL_FORM_MS.leave, () => resize(true));
  }, [later, resize, setForm]);

  const collapse = useCallback(() => {
    if (formRef.current !== "open") return;
    setForm("closing");
    later(reducedMotion() ? 0 : CALL_FORM_MS.close, () => resize(false));
  }, [later, resize, setForm]);

  const toggle = useCallback(() => {
    if (formRef.current === "open") collapse();
    else expand();
  }, [collapse, expand]);

  /** Back to the bar at once, with no window change (a call starting). */
  const reset = useCallback(() => {
    clearTimers();
    setForm("bar");
  }, [clearTimers, setForm]);

  return {
    form,
    /** The conversation panel is mounted (open, or folding away). */
    expanded: form === "open" || form === "closing",
    expand,
    collapse,
    toggle,
    reset,
  };
}
