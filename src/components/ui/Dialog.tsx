import React, { useEffect, useId, useLayoutEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { usePortalTarget } from "./portal";

/**
 * Modal dialog: a scrim, a centred panel, Escape and click-outside to close.
 *
 * Used for focused edits that should not cost the user their place on a page —
 * a writing style, the settings window, one assistant feature — which is the
 * pattern the reference apps use and the one this app was missing: everything
 * used to be a row on an ever-longer scrolling page.
 *
 * Only the top-most open dialog reacts to Escape, so a dialog opened from
 * inside another one closes first.
 */

type DialogSize = "sm" | "md" | "lg" | "xl" | "settings";

const SIZE_CLASSES: Record<DialogSize, string> = {
  sm: "w-[min(26rem,calc(100vw-2rem))]",
  md: "w-[min(34rem,calc(100vw-2rem))]",
  lg: "w-[min(44rem,calc(100vw-2rem))]",
  xl: "w-[min(56rem,calc(100vw-2rem))]",
  settings:
    "w-[min(60rem,calc(100vw-2.5rem))] h-[min(44rem,calc(100vh-4.5rem))]",
};

/** Stack of open dialog ids; the last one owns Escape. */
const openStack: string[] = [];

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export interface DialogProps {
  open: boolean;
  onClose: () => void;
  /** Heading shown in the default header. Also names the dialog for screen readers. */
  title?: React.ReactNode;
  description?: React.ReactNode;
  size?: DialogSize;
  children: React.ReactNode;
  /** Optional sticky footer (actions). */
  footer?: React.ReactNode;
  /** Render without the default header; supply your own and pass `labelledBy`. */
  bare?: boolean;
  /** Id of an element that labels the dialog when `bare` is set. */
  labelledBy?: string;
  /** Close when the scrim is clicked. Defaults to true. */
  closeOnScrim?: boolean;
  /** Extra classes for the panel. */
  className?: string;
  /** Classes for the scrolling body. */
  bodyClassName?: string;
}

export const Dialog: React.FC<DialogProps> = ({
  open,
  onClose,
  title,
  description,
  size = "md",
  children,
  footer,
  bare = false,
  labelledBy,
  closeOnScrim = true,
  className = "",
  bodyClassName = "px-6 pb-6",
}) => {
  const { t } = useTranslation();
  const id = useId();
  const portalTarget = usePortalTarget();
  const titleId = `${id}-title`;
  const panelRef = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  // A press that starts inside the panel and is released on the scrim (a text
  // selection dragged too far) must not count as a click outside.
  const pressStartedOnScrim = useRef(false);

  // Stack membership + Escape.
  useEffect(() => {
    if (!open) return;
    openStack.push(id);
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (openStack[openStack.length - 1] !== id) return;
      // Decide after every other listener has seen the key. A shortcut field
      // that is recording claims Escape (it calls preventDefault, or marks
      // itself with data-shortcut-recording for the backend-captured engine),
      // and closing the whole dialog under it would throw the edit away.
      window.setTimeout(() => {
        if (event.defaultPrevented) return;
        if (document.querySelector("[data-shortcut-recording]")) return;
        if (openStack[openStack.length - 1] !== id) return;
        onCloseRef.current();
      }, 0);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      const index = openStack.lastIndexOf(id);
      if (index >= 0) openStack.splice(index, 1);
    };
  }, [open, id]);

  // Move focus in on open, and give it back on close. Focus goes to an element
  // marked `data-autofocus`, else to the panel itself: landing on the first
  // focusable (usually the close button) drew a focus ring on it every time.
  useLayoutEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    const panel = panelRef.current;
    if (panel) {
      const preferred = panel.querySelector<HTMLElement>("[data-autofocus]");
      (preferred ?? panel).focus({ preventScroll: true });
    }
    return () => {
      if (previous && document.contains(previous)) {
        previous.focus({ preventScroll: true });
      }
    };
  }, [open]);

  if (!open || !portalTarget) return null;

  // Keep Tab inside the panel.
  const onPanelKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Tab" || !panelRef.current) return;
    const focusable = Array.from(
      panelRef.current.querySelectorAll<HTMLElement>(FOCUSABLE),
    ).filter((element) => element.offsetParent !== null);
    if (focusable.length === 0) {
      event.preventDefault();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  const isSettings = size === "settings";

  return createPortal(
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center p-4"
      onMouseDown={(event) => {
        pressStartedOnScrim.current = event.target === event.currentTarget;
      }}
      onClick={(event) => {
        if (
          closeOnScrim &&
          pressStartedOnScrim.current &&
          event.target === event.currentTarget
        ) {
          onClose();
        }
        pressStartedOnScrim.current = false;
      }}
    >
      <div
        aria-hidden="true"
        className="dialog-scrim pointer-events-none absolute inset-0 bg-[rgb(20_18_16/0.42)] backdrop-blur-[3px]"
      />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={bare ? labelledBy : title ? titleId : undefined}
        tabIndex={-1}
        onKeyDown={onPanelKeyDown}
        className={`dialog-panel relative flex max-h-[calc(100vh-3rem)] flex-col overflow-hidden rounded-[1.25rem] border border-hairline bg-surface text-ink shadow-[0_32px_80px_-28px_rgba(20,18,16,0.5),0_2px_8px_rgba(20,18,16,0.08)] outline-none ${SIZE_CLASSES[size]} ${className}`}
      >
        {!bare && (
          <div className="flex shrink-0 items-start gap-4 px-6 pt-6 pb-4">
            <div className="min-w-0 flex-1">
              {title && (
                <h2
                  id={titleId}
                  className="font-display text-[1.375rem] text-ink"
                >
                  {title}
                </h2>
              )}
              {description && (
                <p className="mt-1.5 max-w-xl text-sm leading-relaxed text-muted text-pretty">
                  {description}
                </p>
              )}
            </div>
            <DialogCloseButton onClick={onClose} label={t("common.close")} />
          </div>
        )}
        <div
          className={`min-h-0 flex-1 ${isSettings ? "flex" : "overflow-y-auto"} ${bodyClassName}`}
        >
          {children}
        </div>
        {footer && (
          <div className="flex shrink-0 items-center gap-2 border-t border-hairline bg-canvas/60 px-6 py-3">
            {footer}
          </div>
        )}
      </div>
    </div>,
    portalTarget,
  );
};

export const DialogCloseButton: React.FC<{
  onClick: () => void;
  label: string;
  className?: string;
}> = ({ onClick, label, className = "" }) => (
  <button
    type="button"
    onClick={onClick}
    aria-label={label}
    title={label}
    className={`-me-2 -mt-1 grid h-8 w-8 shrink-0 cursor-pointer place-items-center rounded-lg text-muted transition-colors hover:bg-ink/6 hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${className}`}
  >
    <X className="h-4 w-4" aria-hidden="true" />
  </button>
);
