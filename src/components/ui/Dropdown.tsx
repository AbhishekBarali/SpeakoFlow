import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { usePortalTarget } from "./portal";

export interface DropdownOption {
  value: string;
  label: string;
  disabled?: boolean;
}

interface DropdownProps {
  options: DropdownOption[];
  className?: string;
  selectedValue: string | null;
  onSelect: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  onRefresh?: () => void;
}

interface MenuPosition {
  top: number;
  left: number;
  width: number;
  maxHeight: number;
  placement: "below" | "above";
}

const MENU_GAP = 6;
const MENU_MAX = 256;

/**
 * Plain select. The menu renders in a portal with fixed positioning and flips
 * above the button when there is no room below, so it is never clipped by a
 * card or trapped at the bottom of a scrolling dialog.
 */
export const Dropdown: React.FC<DropdownProps> = ({
  options,
  selectedValue,
  onSelect,
  className = "",
  placeholder,
  disabled = false,
  onRefresh,
}) => {
  const { t } = useTranslation();
  const portalTarget = usePortalTarget();
  const placeholderText = placeholder ?? t("common.select");
  const [isOpen, setIsOpen] = useState(false);
  const [position, setPosition] = useState<MenuPosition | null>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const place = useCallback(() => {
    const button = buttonRef.current;
    if (!button) return;
    const rect = button.getBoundingClientRect();
    const spaceBelow = window.innerHeight - rect.bottom - MENU_GAP - 8;
    const spaceAbove = rect.top - MENU_GAP - 8;
    const placement =
      spaceBelow >= Math.min(MENU_MAX, 180) || spaceBelow >= spaceAbove
        ? "below"
        : "above";
    const maxHeight = Math.max(
      120,
      Math.min(MENU_MAX, placement === "below" ? spaceBelow : spaceAbove),
    );
    const width = Math.max(rect.width, 160);
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
    if (!isOpen) {
      setPosition(null);
      return;
    }
    place();
  }, [isOpen, place]);

  useEffect(() => {
    if (!isOpen) return;
    const handleClickOutside = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        dropdownRef.current?.contains(target) ||
        listRef.current?.contains(target)
      ) {
        return;
      }
      setIsOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        // Claim the key so an enclosing dialog stays open.
        event.preventDefault();
        setIsOpen(false);
        buttonRef.current?.focus();
      }
    };
    const reposition = () => place();
    document.addEventListener("mousedown", handleClickOutside);
    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [isOpen, place]);

  // Open onto the current selection, not onto the top of the list. The menu is
  // height-capped and scrolls, so for a long list (downloaded models, cloud
  // model ids) a selection past the fold looked like the setting had reset to
  // the first entry and forced the user to scroll to find their own choice.
  //
  // `useLayoutEffect` so the scroll lands before the menu is painted — with a
  // plain effect the list is visibly at the top for a frame. `options` is in the
  // deps because a dropdown with `onRefresh` fetches its options *after* opening,
  // and the first pass then has nothing to centre on.
  useLayoutEffect(() => {
    if (!isOpen || !position) return;
    const list = listRef.current;
    const selected = list?.querySelector<HTMLElement>('[data-selected="true"]');
    if (!list || !selected) return;
    list.scrollTop =
      selected.offsetTop - list.clientHeight / 2 + selected.offsetHeight / 2;
  }, [isOpen, options, position]);

  const selectedOption = options.find(
    (option) => option.value === selectedValue,
  );

  const handleSelect = (value: string) => {
    onSelect(value);
    setIsOpen(false);
    buttonRef.current?.focus();
  };

  const handleToggle = () => {
    if (disabled) return;
    if (!isOpen && onRefresh) onRefresh();
    setIsOpen(!isOpen);
  };

  const onListKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const items = Array.from(
      listRef.current?.querySelectorAll<HTMLButtonElement>(
        "button:not([disabled])",
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
    <div className={`relative ${className}`} ref={dropdownRef}>
      <button
        ref={buttonRef}
        type="button"
        aria-haspopup="listbox"
        aria-expanded={isOpen}
        className={`w-full h-9 px-3 text-sm bg-surface border border-hairline-strong rounded-lg min-w-[12.5rem] text-start flex items-center justify-between shadow-[0_1px_1px_rgba(27,26,24,0.04)] transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${
          disabled
            ? "opacity-50 cursor-not-allowed"
            : "hover:border-ink/25 cursor-pointer"
        }`}
        onClick={handleToggle}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" && !isOpen && !disabled) {
            event.preventDefault();
            if (onRefresh) onRefresh();
            setIsOpen(true);
          }
        }}
        disabled={disabled}
        title={selectedOption?.label || placeholderText}
      >
        <span className="truncate min-w-0">
          {selectedOption?.label || placeholderText}
        </span>
        <svg
          className={`w-4 h-4 ms-2 shrink-0 text-muted transition-transform duration-200 ${isOpen ? "transform rotate-180" : ""}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
          aria-hidden="true"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M19 9l-7 7-7-7"
          />
        </svg>
      </button>
      {isOpen &&
        !disabled &&
        position &&
        portalTarget &&
        createPortal(
          <div
            ref={listRef}
            role="listbox"
            onKeyDown={onListKeyDown}
            style={{
              position: "fixed",
              left: position.left,
              width: position.width,
              maxHeight: position.maxHeight,
              ...(position.placement === "below"
                ? { top: position.top }
                : { bottom: window.innerHeight - position.top }),
            }}
            className="glass-menu elev-float z-[70] overflow-y-auto overflow-x-hidden rounded-xl border border-hairline p-1"
          >
            {options.length === 0 ? (
              <div className="px-2.5 py-1.5 text-sm text-muted">
                {t("common.noOptionsFound")}
              </div>
            ) : (
              options.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  role="option"
                  aria-selected={selectedValue === option.value}
                  data-selected={selectedValue === option.value}
                  className={`flex items-center w-full px-2.5 py-1.5 text-sm text-start rounded-lg overflow-hidden hover:bg-surface-strong focus-visible:bg-surface-strong focus-visible:outline-none transition-colors duration-150 cursor-pointer ${
                    selectedValue === option.value
                      ? "bg-surface-strong font-medium"
                      : ""
                  } ${option.disabled ? "opacity-50 cursor-not-allowed" : ""}`}
                  onClick={() => handleSelect(option.value)}
                  disabled={option.disabled}
                  title={option.label}
                >
                  <span className="truncate min-w-0 flex-1">
                    {option.label}
                  </span>
                </button>
              ))
            )}
          </div>,
          portalTarget,
        )}
    </div>
  );
};
