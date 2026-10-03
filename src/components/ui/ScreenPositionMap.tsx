import React from "react";

/** One place on the pictured screen, in percent of it. */
export interface ScreenSpot<T extends string> {
  value: T;
  label: string;
  box: { left: number; top: number; width: number; height: number };
}

interface ScreenPositionMapProps<T extends string> {
  spots: ReadonlyArray<ScreenSpot<T>>;
  /** The chosen spot, or null when none is (e.g. the thing is hidden). */
  value: T | null;
  onChange: (value: T) => void;
  label: string;
  /** `panel` for a window-sized spot, `pill` for a small lozenge. */
  shape?: "panel" | "pill";
  className?: string;
}

/**
 * "Where it opens" as a picture of a screen: click the spot. Shared by the
 * assistant's panel and the dictation overlay, so the two pickers are one
 * control rather than two that look alike. A list of the same choices belongs
 * beside it: the map is the quick way, the list names every place and works
 * from the keyboard.
 */
export function ScreenPositionMap<T extends string>({
  spots,
  value,
  onChange,
  label,
  shape = "panel",
  className = "",
}: ScreenPositionMapProps<T>) {
  const radius = shape === "pill" ? "rounded-full" : "rounded-[5px]";
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={`relative aspect-video overflow-hidden rounded-xl border border-hairline-strong bg-surface-muted ${className}`}
    >
      {/* The top edge of a window, so it reads as a screen, not a grid. */}
      <span
        aria-hidden="true"
        className="absolute inset-x-0 top-0 h-[6%] bg-ink/[0.06]"
      />
      {spots.map((spot) => {
        const active = spot.value === value;
        return (
          <button
            key={spot.value}
            type="button"
            role="radio"
            aria-checked={active}
            aria-label={spot.label}
            title={spot.label}
            onClick={() => {
              if (!active) onChange(spot.value);
            }}
            style={{
              left: `${spot.box.left}%`,
              top: `${spot.box.top}%`,
              width: `${spot.box.width}%`,
              height: `${spot.box.height}%`,
            }}
            className={`absolute cursor-pointer ${radius} border transition-[background-color,border-color,box-shadow] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${
              active
                ? "border-accent-fill bg-accent-fill shadow-[0_4px_12px_-4px_rgb(0_150_132/0.55)]"
                : "border-dashed border-ink/20 bg-surface/50 hover:border-accent/60 hover:bg-accent/10"
            }`}
          />
        );
      })}
    </div>
  );
}
