import React, { useId, useRef } from "react";

export interface TabItem<T extends string> {
  id: T;
  label: string;
  /** Small glyph before the label. */
  icon?: React.ComponentType<{ className?: string }>;
  /** Small trailing element (a count, a dot). */
  badge?: React.ReactNode;
}

/**
 * Underlined text tabs: the lightest way to split one page into views. Used
 * where the old design drilled into a sub-page and needed a Back button to get
 * out again — a tab is always one click from its siblings.
 *
 * Arrow keys move between tabs (roving tabindex), per the WAI-ARIA tabs pattern.
 */
export function Tabs<T extends string>({
  items,
  value,
  onChange,
  label,
  className = "",
  trailing,
}: {
  items: TabItem<T>[];
  value: T;
  onChange: (id: T) => void;
  /** Accessible name for the tab list. */
  label: string;
  className?: string;
  /** Controls at the right end of the tab row. */
  trailing?: React.ReactNode;
}) {
  const baseId = useId();
  const refs = useRef<Record<string, HTMLButtonElement | null>>({});

  const onKeyDown = (event: React.KeyboardEvent, index: number) => {
    if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
    event.preventDefault();
    const rtl =
      typeof document !== "undefined" && document.documentElement.dir === "rtl";
    const forward = (event.key === "ArrowRight") !== rtl;
    const next =
      items[(index + (forward ? 1 : items.length - 1)) % items.length];
    onChange(next.id);
    refs.current[next.id]?.focus();
  };

  return (
    <div
      className={`flex items-end justify-between gap-4 border-b border-hairline ${className}`}
    >
      <div role="tablist" aria-label={label} className="-mb-px flex gap-7">
        {items.map((item, index) => {
          const selected = item.id === value;
          const Icon = item.icon;
          return (
            <button
              key={item.id}
              ref={(element) => {
                refs.current[item.id] = element;
              }}
              id={`${baseId}-${item.id}`}
              type="button"
              role="tab"
              aria-selected={selected}
              tabIndex={selected ? 0 : -1}
              onClick={() => onChange(item.id)}
              onKeyDown={(event) => onKeyDown(event, index)}
              className={`relative inline-flex cursor-pointer items-center gap-1.5 pb-3 text-[0.9375rem] transition-colors focus-visible:outline-none focus-visible:after:bg-accent ${
                selected
                  ? "font-medium text-ink after:absolute after:inset-x-0 after:bottom-0 after:h-[2px] after:rounded-full after:bg-ink"
                  : "text-muted hover:text-ink"
              }`}
            >
              {Icon && (
                <Icon
                  className={`h-4 w-4 shrink-0 ${selected ? "text-accent" : "text-muted-soft"}`}
                />
              )}
              {item.label}
              {item.badge}
            </button>
          );
        })}
      </div>
      {trailing && (
        <div className="flex shrink-0 items-center gap-1 pb-2">{trailing}</div>
      )}
    </div>
  );
}
