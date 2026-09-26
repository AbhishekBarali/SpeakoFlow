import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal, flushSync } from "react-dom";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, Search } from "lucide-react";
import { usePortalTarget } from "./portal";

/**
 * A dropdown whose options carry a logo, for choosing a provider or engine.
 *
 * The plain `Dropdown` shows a list of names, which is fine for "Small /
 * Medium / Large" and poor for "which AI company runs this": a logo is read
 * faster than a word and tells you at a glance that OpenRouter and OpenAI are
 * different things. The menu renders in a portal with fixed positioning so it
 * is never clipped by a card or a scrolling dialog body.
 */

export interface LogoSelectOption {
  value: string;
  label: string;
  icon?: React.ReactNode;
  /** Short secondary line under the label. */
  hint?: string;
  /** Options sharing a group are listed under that heading. */
  group?: string;
  disabled?: boolean;
  /** Native tooltip, e.g. the exact id behind a friendly label. */
  title?: string;
}

interface LogoSelectProps {
  options: LogoSelectOption[];
  value: string | null;
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  /** Accessible name when there is no visible label next to the control. */
  ariaLabel?: string;
  /** Show a filter box above the options. Defaults to on for long lists. */
  searchable?: boolean;
  /** A link pinned under the options ("Browse models…"). Closes the menu. */
  footerAction?: { label: string; onClick: () => void; icon?: React.ReactNode };
}

interface MenuPosition {
  top: number;
  left: number;
  width: number;
  maxHeight: number;
  placement: "below" | "above";
}

const MENU_GAP = 6;
const MENU_MAX = 320;

export const LogoSelect: React.FC<LogoSelectProps> = ({
  options,
  value,
  onChange,
  placeholder,
  disabled = false,
  className = "",
  ariaLabel,
  searchable,
  footerAction,
}) => {
  const { t } = useTranslation();
  const portalTarget = usePortalTarget();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [position, setPosition] = useState<MenuPosition | null>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const showSearch = searchable ?? options.length > 8;
  const selected = options.find((option) => option.value === value);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return options;
    return options.filter(
      (option) =>
        option.label.toLowerCase().includes(needle) ||
        option.value.toLowerCase().includes(needle),
    );
  }, [options, query]);

  const place = useCallback(() => {
    const button = buttonRef.current;
    if (!button) return;
    const rect = button.getBoundingClientRect();
    const spaceBelow = window.innerHeight - rect.bottom - MENU_GAP - 8;
    const spaceAbove = rect.top - MENU_GAP - 8;
    const placement =
      spaceBelow >= Math.min(MENU_MAX, 220) || spaceBelow >= spaceAbove
        ? "below"
        : "above";
    const maxHeight = Math.max(
      140,
      Math.min(MENU_MAX, placement === "below" ? spaceBelow : spaceAbove),
    );
    const width = Math.max(rect.width, 240);
    const left = Math.min(
      Math.max(8, rect.left),
      window.innerWidth - width - 8,
    );
    setPosition({
      top: placement === "below" ? rect.bottom + MENU_GAP : rect.top - MENU_GAP,
      left,
      width,
      maxHeight,
      placement,
    });
  }, []);

  useLayoutEffect(() => {
    if (!open) return;
    place();
  }, [open, place]);

  useEffect(() => {
    if (!open) return;
    const reposition = () => place();
    const onPointerDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        buttonRef.current?.contains(target) ||
        menuRef.current?.contains(target)
      ) {
        return;
      }
      setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        // Claim the key so an enclosing dialog stays open.
        event.preventDefault();
        setOpen(false);
        buttonRef.current?.focus();
      }
    };
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    document.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
      document.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open, place]);

  // Focus the filter (or the selected option) when the menu opens.
  useEffect(() => {
    if (!open || !position) return;
    if (showSearch) {
      searchRef.current?.focus();
      return;
    }
    const current = menuRef.current?.querySelector<HTMLElement>(
      '[data-selected="true"]',
    );
    (current ?? menuRef.current?.querySelector<HTMLElement>("button"))?.focus();
  }, [open, position, showSearch]);

  useEffect(() => {
    if (!open) setQuery("");
  }, [open]);

  const choose = (next: string) => {
    setOpen(false);
    buttonRef.current?.focus();
    if (next !== value) onChange(next);
  };

  const onMenuKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const items = Array.from(
      menuRef.current?.querySelectorAll<HTMLButtonElement>(
        "button[data-option]:not([disabled])",
      ) ?? [],
    );
    if (items.length === 0) return;
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === "ArrowDown"
        ? items[(index + 1) % items.length]
        : items[(index - 1 + items.length) % items.length];
    next.focus();
  };

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        title={selected?.title}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" && !open) {
            event.preventDefault();
            setOpen(true);
          }
        }}
        className={`flex h-10 min-w-[13rem] cursor-pointer items-center gap-2.5 rounded-lg border border-hairline-strong bg-surface ps-2 pe-2.5 text-start text-sm text-ink shadow-[0_1px_1px_rgba(27,26,24,0.04)] transition-colors hover:border-ink/25 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-50 ${className}`}
      >
        {selected?.icon && <span className="shrink-0">{selected.icon}</span>}
        <span
          className={`min-w-0 flex-1 truncate ${selected ? "" : "ps-1 text-muted"}`}
        >
          {selected?.label ?? placeholder ?? t("common.select")}
        </span>
        <ChevronDown
          className={`h-4 w-4 shrink-0 text-muted transition-transform duration-200 ${open ? "rotate-180" : ""}`}
          aria-hidden="true"
        />
      </button>
      {open &&
        position &&
        portalTarget &&
        createPortal(
          <div
            ref={menuRef}
            role="listbox"
            onKeyDown={onMenuKeyDown}
            style={{
              position: "fixed",
              left: position.left,
              width: position.width,
              maxHeight: position.maxHeight,
              ...(position.placement === "below"
                ? { top: position.top }
                : { bottom: window.innerHeight - position.top }),
            }}
            className="elev-float z-[70] flex flex-col overflow-hidden rounded-xl border border-hairline bg-surface"
          >
            {showSearch && (
              <div className="relative shrink-0 border-b border-hairline p-1.5">
                <Search
                  className="pointer-events-none absolute start-4 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-soft"
                  aria-hidden="true"
                />
                <input
                  ref={searchRef}
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "ArrowDown") {
                      event.preventDefault();
                      menuRef.current
                        ?.querySelector<HTMLButtonElement>(
                          "button[data-option]:not([disabled])",
                        )
                        ?.focus();
                    } else if (event.key === "Enter" && filtered[0]) {
                      event.preventDefault();
                      choose(filtered[0].value);
                    }
                  }}
                  placeholder={t("common.search")}
                  aria-label={t("common.search")}
                  className="h-8 w-full rounded-lg bg-surface-strong/70 ps-8 pe-2 text-sm text-ink placeholder:text-muted-soft focus:outline-none focus:ring-2 focus:ring-accent/30"
                />
              </div>
            )}
            <div className="min-h-0 flex-1 overflow-y-auto p-1">
              {filtered.length === 0 ? (
                <p className="px-3 py-2 text-sm text-muted">
                  {t("common.noOptionsFound")}
                </p>
              ) : (
                filtered.map((option, index) => {
                  const isSelected = option.value === value;
                  const heading =
                    option.group && option.group !== filtered[index - 1]?.group
                      ? option.group
                      : null;
                  return (
                    <React.Fragment key={option.value}>
                      {heading && (
                        <p
                          role="presentation"
                          className={`px-2 pb-1 text-xs font-medium text-muted ${index === 0 ? "pt-1.5" : "mt-1 border-t border-hairline pt-2.5"}`}
                        >
                          {heading}
                        </p>
                      )}
                      <button
                        type="button"
                        role="option"
                        aria-selected={isSelected}
                        data-option="true"
                        data-selected={isSelected}
                        disabled={option.disabled}
                        title={option.title}
                        onClick={() => choose(option.value)}
                        className={`flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-2 py-1.5 text-start text-sm transition-colors hover:bg-surface-strong focus-visible:bg-surface-strong focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50 ${isSelected ? "bg-surface-strong/70" : ""}`}
                      >
                        {option.icon && (
                          <span className="shrink-0">{option.icon}</span>
                        )}
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-ink">
                            {option.label}
                          </span>
                          {option.hint && (
                            <span className="block truncate text-xs text-muted">
                              {option.hint}
                            </span>
                          )}
                        </span>
                        {isSelected && (
                          <Check
                            className="h-4 w-4 shrink-0 text-accent"
                            aria-hidden="true"
                          />
                        )}
                      </button>
                    </React.Fragment>
                  );
                })
              )}
            </div>
            {footerAction && (
              <div className="shrink-0 border-t border-hairline p-1">
                <button
                  type="button"
                  onClick={() => {
                    // Close first, synchronously: the action usually opens
                    // another page, and a page that is being hidden cannot
                    // commit its own menu closing.
                    flushSync(() => setOpen(false));
                    footerAction.onClick();
                  }}
                  className="flex w-full cursor-pointer items-center gap-2 rounded-lg px-2 py-2 text-start text-sm font-medium text-ink transition-colors hover:bg-surface-strong focus-visible:bg-surface-strong focus-visible:outline-none"
                >
                  {footerAction.icon}
                  <span className="min-w-0 flex-1 truncate">
                    {footerAction.label}
                  </span>
                </button>
              </div>
            )}
          </div>,
          portalTarget,
        )}
    </>
  );
};
