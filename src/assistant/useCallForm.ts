import { useCallback, useEffect, useRef, useState } from "react";
import { commands } from "@/bindings";

/**
 * Which form the live call is in, including the fold in between.
 *
 * Both forms are drawn in one window that never changes size (see
 * `conversation_size` in `assistant.rs`): the bar floats at the bottom of a
 * transparent frame, and the conversation panel grows up out of it in the same
 * frame. Opening and closing are therefore pure CSS, with nothing on the native
 * side to wait for:
 *
 *   open    bar ─► open (the panel grows up out of the bar)
 *   close   open ─► closing (the panel folds down into the bar) ─► bar
 *
 * It used to resize the window, and a WebView2 window that changes shape shows
 * its old picture at the new size for a frame or two. Drawing through that made
 * the bar (and its "Message …" field) flash at the wrong height; blanking the
 * window to hide it made the call vanish for a beat on every open and close.
 *
 * Rust is still told which form is showing (`assistantConversationSetExpanded`),
 * because it remembers a drag-resize of the expanded panel and keeps the panel's
 * header on screen.
 */
export type CallForm = "bar" | "open" | "closing";

/** Keep in step with the `call-panel-fold` animation in `CallBar.css`. */
export const CALL_FORM_MS = {
  /** The panel folding down into the bar. */
  close: 220,
} as const;

function reducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

export function useCallForm(active: boolean) {
  const [form, setFormState] = useState<CallForm>("bar");
  // Updated with the state rather than on render, so two clicks inside one
  // React batch cannot both see `bar` and report the change twice.
  const formRef = useRef(form);
  const setForm = useCallback((next: CallForm) => {
    formRef.current = next;
    setFormState(next);
  }, []);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearTimer = useCallback(() => {
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
  }, []);

  // A call that ends leaves no form behind: the next one opens as the bar.
  useEffect(() => {
    if (active) return;
    clearTimer();
    setForm("bar");
  }, [active, clearTimer, setForm]);

  useEffect(() => clearTimer, [clearTimer]);

  const expand = useCallback(() => {
    if (formRef.current !== "bar") return;
    setForm("open");
    void commands.assistantConversationSetExpanded(true);
  }, [setForm]);

  const collapse = useCallback(() => {
    if (formRef.current !== "open") return;
    setForm("closing");
    clearTimer();
    timer.current = setTimeout(
      () => {
        timer.current = null;
        if (formRef.current !== "closing") return;
        setForm("bar");
        void commands.assistantConversationSetExpanded(false);
      },
      reducedMotion() ? 0 : CALL_FORM_MS.close,
    );
  }, [clearTimer, setForm]);

  const toggle = useCallback(() => {
    if (formRef.current === "open") collapse();
    else expand();
  }, [collapse, expand]);

  /** Back to the bar at once (a call starting). */
  const reset = useCallback(() => {
    clearTimer();
    setForm("bar");
  }, [clearTimer, setForm]);

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
