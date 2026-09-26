import React, { useCallback, useLayoutEffect, useRef, useState } from "react";

export interface SegmentedOption<T extends string> {
  value: T;
  label: React.ReactNode;
  /** Accessible name when `label` is not plain text. */
  ariaLabel?: string;
  icon?: React.ComponentType<{ className?: string }>;
  title?: string;
  disabled?: boolean;
}

interface SegmentedProps<T extends string> {
  options: SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
  /** Accessible name for the group. */
  label: string;
  disabled?: boolean;
  size?: "sm" | "md";
  /** Stretch to the container, one equal column per option. */
  fill?: boolean;
  /** `glass` for the gradient hero. */
  tone?: "default" | "glass";
  className?: string;
}

const SIZES = {
  sm: "h-7 px-2.5 text-xs",
  md: "h-8 px-3 text-[0.8125rem]",
};

/**
 * A short, mutually exclusive choice shown in full: "Hold · Tap",
 * "Off · When I attach it · Assistant decides". Every option is visible, so
 * the setting explains itself without being opened — which is what a
 * dropdown with three entries could never do.
 *
 * The selected pill slides between options. It is measured from the buttons
 * rather than computed, so labels of any length (and any language) work; until
 * the first measurement the selected button paints its own background, so
 * nothing is ever shown unselected.
 */
export function Segmented<T extends string>({
  options,
  value,
  onChange,
  label,
  disabled = false,
  size = "md",
  fill = false,
  tone = "default",
  className = "",
}: SegmentedProps<T>) {
  const trackRef = useRef<HTMLDivElement>(null);
  const buttons = useRef<Record<string, HTMLButtonElement | null>>({});
  const [thumb, setThumb] = useState<{ left: number; width: number } | null>(
    null,
  );
  const [animated, setAnimated] = useState(false);

  const measure = useCallback(() => {
    const element = buttons.current[value];
    if (!element || element.offsetWidth === 0) {
      setThumb(null);
      return;
    }
    setThumb((current) =>
      current &&
      current.left === element.offsetLeft &&
      current.width === element.offsetWidth
        ? current
        : { left: element.offsetLeft, width: element.offsetWidth },
    );
  }, [value]);

  useLayoutEffect(() => {
    measure();
  }, [measure, options.length]);

  useLayoutEffect(() => {
    const track = trackRef.current;
    if (!track || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => measure());
    observer.observe(track);
    return () => observer.disconnect();
  }, [measure]);

  // Slide only once there is a position to slide from; the first placement is
  // instant.
  useLayoutEffect(() => {
    if (!thumb || animated) return;
    const frame = window.requestAnimationFrame(() => setAnimated(true));
    return () => window.cancelAnimationFrame(frame);
  }, [thumb, animated]);

  const enabled = options.filter((option) => !option.disabled);

  const onKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
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
    const index = enabled.findIndex((option) => option.value === value);
    const next = enabled[(index + step + enabled.length) % enabled.length];
    onChange(next.value);
    buttons.current[next.value]?.focus();
  };

  const glass = tone === "glass";
  const track = glass
    ? "border border-white/20 bg-white/10"
    : "border border-hairline bg-surface-strong";

  return (
    <div
      ref={trackRef}
      role="radiogroup"
      aria-label={label}
      aria-disabled={disabled || undefined}
      className={`relative isolate ${fill ? "flex w-full" : "inline-flex"} rounded-lg p-0.5 ${track} ${disabled ? "opacity-50" : ""} ${className}`}
    >
      {thumb && (
        <span
          aria-hidden="true"
          className={`absolute top-0.5 bottom-0.5 -z-10 rounded-md ${
            glass
              ? "bg-white/95 shadow-[0_2px_8px_rgba(0,0,0,0.18)]"
              : "segmented-pill"
          } ${animated ? "segmented-thumb" : ""}`}
          style={{ left: thumb.left, width: thumb.width }}
        />
      )}
      {options.map((option) => {
        const selected = option.value === value;
        const Icon = option.icon;
        const text = selected
          ? glass
            ? "text-[#0b3f3a]"
            : "text-ink"
          : glass
            ? "text-white/80 hover:text-white"
            : "text-muted hover:text-ink";
        const ownBackground =
          selected && !thumb ? (glass ? "bg-white/95" : "segmented-pill") : "";
        return (
          <button
            key={option.value}
            ref={(element) => {
              buttons.current[option.value] = element;
            }}
            type="button"
            role="radio"
            aria-checked={selected}
            aria-label={option.ariaLabel}
            title={option.title}
            tabIndex={selected ? 0 : -1}
            disabled={disabled || option.disabled}
            onClick={() => {
              if (!selected) onChange(option.value);
            }}
            onKeyDown={onKeyDown}
            className={`inline-flex cursor-pointer items-center justify-center gap-1.5 rounded-md font-medium whitespace-nowrap transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 disabled:cursor-not-allowed ${
              glass
                ? "focus-visible:ring-white/60"
                : "focus-visible:ring-accent/40"
            } ${fill ? "min-w-0 flex-1" : ""} ${SIZES[size]} ${text} ${ownBackground}`}
          >
            {Icon && (
              <Icon
                className={`h-3.5 w-3.5 shrink-0 ${selected && !glass ? "text-accent" : ""}`}
              />
            )}
            <span className={fill ? "truncate" : undefined}>
              {option.label}
            </span>
          </button>
        );
      })}
    </div>
  );
}
