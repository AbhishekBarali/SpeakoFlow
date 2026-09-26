import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal, flushSync } from "react-dom";
import { Check } from "lucide-react";
import { usePortalTarget } from "./portal";

export interface MenuItem {
  id: string;
  label: string;
  hint?: string;
  /** Present for a choice with an on/off state; omit for a plain action. */
  checked?: boolean;
  icon?: React.ComponentType<{ className?: string }>;
  tone?: "default" | "danger";
  disabled?: boolean;
  /** Draw a rule above this item, to set a group apart (e.g. Delete). */
  separated?: boolean;
  onSelect: () => void;
}

interface MenuPosition {
  top: number;
  left: number;
  placement: "below" | "above";
}

const GAP = 6;

/**
 * A button that opens a short list of actions. The list renders in a portal
 * with fixed positioning, flips above the trigger near the bottom of the
 * window, and closes on Escape, outside click, scroll, or a choice.
 */
export const MenuButton: React.FC<{
  items: MenuItem[];
  /** Trigger contents. */
  children: React.ReactNode;
  className?: string;
  ariaLabel?: string;
  title?: string;
  disabled?: boolean;
  /** Menu width in px. */
  width?: number;
}> = ({
  items,
  children,
  className = "",
  ariaLabel,
  title,
  disabled = false,
  width = 248,
}) => {
  const portalTarget = usePortalTarget();
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<MenuPosition | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const place = useCallback(() => {
    const rect = buttonRef.current?.getBoundingClientRect();
    if (!rect) return;
    const estimated =
      items.reduce((sum, item) => sum + (item.hint ? 52 : 38), 0) + 12;
    const below = window.innerHeight - rect.bottom - GAP - 8 >= estimated;
    setPosition({
      top: below ? rect.bottom + GAP : rect.top - GAP,
      left: Math.min(
        Math.max(8, rect.right - width),
        window.innerWidth - width - 8,
      ),
      placement: below ? "below" : "above",
    });
  }, [items, width]);

  useLayoutEffect(() => {
    if (open) place();
    else setPosition(null);
  }, [open, place]);

  useEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    const onPointer = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        buttonRef.current?.contains(target) ||
        menuRef.current?.contains(target)
      ) {
        return;
      }
      close();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close();
        buttonRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [open]);

  useEffect(() => {
    if (!open || !position) return;
    menuRef.current
      ?.querySelector<HTMLButtonElement>("button:not([disabled])")
      ?.focus();
  }, [open, position]);

  const onMenuKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const buttons = Array.from(
      menuRef.current?.querySelectorAll<HTMLButtonElement>(
        "button:not([disabled])",
      ) ?? [],
    );
    if (buttons.length === 0) return;
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === "ArrowDown"
        ? buttons[(index + 1) % buttons.length]
        : buttons[(index - 1 + buttons.length) % buttons.length];
    next.focus();
  };

  const hasChecks = items.some((item) => item.checked !== undefined);

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={ariaLabel}
        title={title}
        onClick={() => setOpen((value) => !value)}
        className={className}
      >
        {children}
      </button>
      {open &&
        position &&
        portalTarget &&
        createPortal(
          <div
            ref={menuRef}
            role="menu"
            onKeyDown={onMenuKeyDown}
            style={{
              position: "fixed",
              left: position.left,
              width,
              ...(position.placement === "below"
                ? { top: position.top }
                : { bottom: window.innerHeight - position.top }),
            }}
            className="elev-float menu-in z-[70] rounded-xl border border-hairline bg-surface p-1"
          >
            {items.map((item) => {
              const Icon = item.icon;
              const danger = item.tone === "danger";
              return (
                <React.Fragment key={item.id}>
                  {item.separated && (
                    <div
                      role="separator"
                      className="mx-1.5 my-1 h-px bg-hairline"
                    />
                  )}
                  <button
                    type="button"
                    role={
                      item.checked !== undefined
                        ? "menuitemcheckbox"
                        : "menuitem"
                    }
                    aria-checked={item.checked}
                    disabled={item.disabled}
                    onClick={() => {
                      // Close before acting: an action that opens another page
                      // would otherwise freeze this one with the menu still up.
                      flushSync(() => setOpen(false));
                      item.onSelect();
                    }}
                    className={`flex w-full cursor-pointer items-start gap-2.5 rounded-lg px-2.5 py-2 text-start transition-colors focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-45 disabled:hover:bg-transparent ${
                      danger
                        ? "text-error hover:bg-error/10 focus-visible:bg-error/10"
                        : "text-ink hover:bg-surface-strong focus-visible:bg-surface-strong"
                    }`}
                  >
                    {(Icon || hasChecks) && (
                      <span className="grid h-5 w-4 shrink-0 place-items-center">
                        {Icon ? (
                          <Icon
                            className={`h-4 w-4 ${danger ? "" : "text-muted"}`}
                          />
                        ) : (
                          item.checked && (
                            <Check
                              className="h-4 w-4 text-accent"
                              aria-hidden="true"
                            />
                          )
                        )}
                      </span>
                    )}
                    <span className="min-w-0 flex-1">
                      <span className="block text-sm">{item.label}</span>
                      {item.hint && (
                        <span className="mt-0.5 block text-xs leading-snug text-muted">
                          {item.hint}
                        </span>
                      )}
                    </span>
                    {Icon && item.checked && (
                      <Check
                        className="mt-0.5 h-4 w-4 shrink-0 text-accent"
                        aria-hidden="true"
                      />
                    )}
                  </button>
                </React.Fragment>
              );
            })}
          </div>,
          portalTarget,
        )}
    </>
  );
};
