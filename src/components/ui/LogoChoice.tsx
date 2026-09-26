import React, { useRef } from "react";
import { Check } from "lucide-react";

export interface LogoChoiceOption {
  value: string;
  label: string;
  icon: React.ReactNode;
  /** One short line under the name ("Free", "On this computer"). */
  hint?: string;
  /** A green dot for a choice that is ready to use (its key is saved). */
  ready?: boolean;
  disabled?: boolean;
  /** Native tooltip for a truncated name. */
  title?: string;
}

/**
 * Choose a company or engine by its logo.
 *
 * A dropdown hides every option but one behind a click, and "which AI company
 * runs this" is exactly the choice people make by recognising a mark. This
 * lays the options out as a grid of logo tiles: the current one is ringed, the
 * ones that are already set up carry a dot, and the whole set is visible at
 * once. Arrow keys move between tiles (roving tab stop), as in a radio group.
 */
export const LogoChoice: React.FC<{
  options: LogoChoiceOption[];
  value: string | null;
  onChange: (value: string) => void;
  /** Accessible name for the group. */
  label: string;
  disabled?: boolean;
  /** Narrowest a tile may get before the grid wraps. */
  minTile?: string;
  className?: string;
  /** Accessible text for the ready dot. */
  readyLabel?: string;
}> = ({
  options,
  value,
  onChange,
  label,
  disabled = false,
  minTile = "10rem",
  className = "",
  readyLabel,
}) => {
  const refs = useRef<Record<string, HTMLButtonElement | null>>({});
  const enabled = options.filter((option) => !option.disabled);
  const focusValue =
    options.find((option) => option.value === value && !option.disabled)
      ?.value ?? enabled[0]?.value;

  const onKeyDown = (event: React.KeyboardEvent, current: string) => {
    const rtl =
      typeof document !== "undefined" && document.documentElement.dir === "rtl";
    const steps: Record<string, number> = {
      ArrowDown: 1,
      ArrowUp: -1,
      ArrowRight: rtl ? -1 : 1,
      ArrowLeft: rtl ? 1 : -1,
    };
    const step = steps[event.key];
    if (!step || enabled.length === 0) return;
    event.preventDefault();
    const index = enabled.findIndex((option) => option.value === current);
    const next = enabled[(index + step + enabled.length) % enabled.length];
    refs.current[next.value]?.focus();
  };

  return (
    <div
      role="radiogroup"
      aria-label={label}
      aria-disabled={disabled || undefined}
      className={`grid gap-2 ${disabled ? "opacity-55" : ""} ${className}`}
      style={{
        gridTemplateColumns: `repeat(auto-fill, minmax(min(${minTile}, 100%), 1fr))`,
      }}
    >
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <button
            key={option.value}
            ref={(element) => {
              refs.current[option.value] = element;
            }}
            type="button"
            role="radio"
            aria-checked={selected}
            title={option.title ?? option.label}
            tabIndex={option.value === focusValue ? 0 : -1}
            disabled={disabled || option.disabled}
            onClick={() => {
              if (!selected) onChange(option.value);
            }}
            onKeyDown={(event) => onKeyDown(event, option.value)}
            className={`group relative flex min-w-0 cursor-pointer items-center gap-2.5 rounded-xl border px-2.5 py-2 text-start transition-[border-color,background-color,box-shadow] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed ${
              selected
                ? "border-accent/55 bg-accent/[0.06] shadow-[0_0_0_1px_color-mix(in_srgb,var(--color-accent)_28%,transparent)]"
                : "border-hairline bg-surface hover:border-hairline-strong hover:bg-surface-muted"
            }`}
          >
            <span className="shrink-0">{option.icon}</span>
            <span className="min-w-0 flex-1 pe-1">
              <span className="block truncate text-[0.8125rem] font-medium text-ink">
                {option.label}
              </span>
              {option.hint && (
                <span className="block truncate text-[0.6875rem] leading-snug text-muted">
                  {option.hint}
                </span>
              )}
            </span>
            {/* Corner marks, so neither costs the name any width. */}
            {selected ? (
              <span className="absolute -top-1.5 -end-1.5 grid h-[1.125rem] w-[1.125rem] place-items-center rounded-full bg-accent text-on-primary ring-2 ring-surface">
                <Check className="h-3 w-3" strokeWidth={3} aria-hidden="true" />
              </span>
            ) : (
              option.ready && (
                <span
                  className="absolute top-2 end-2 h-1.5 w-1.5 rounded-full bg-success"
                  role="img"
                  aria-label={readyLabel}
                  title={readyLabel}
                />
              )
            )}
          </button>
        );
      })}
    </div>
  );
};
