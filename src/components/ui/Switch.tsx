import React from "react";

/**
 * A bare on/off switch, for page headers and cards where a full settings row
 * (`ToggleSwitch`) would be too much. Same track and knob as `ToggleSwitch`, so
 * the two read as one control. `onHero` is for a feature banner's dark stage,
 * which is dark in either theme.
 */
export const Switch: React.FC<{
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  /** Accessible name. Required: the switch has no visible label of its own. */
  label: string;
  className?: string;
  tone?: "default" | "onHero";
}> = ({
  checked,
  onChange,
  disabled = false,
  label,
  className = "",
  tone = "default",
}) => (
  <label
    className={`inline-flex shrink-0 items-center ${disabled ? "cursor-not-allowed" : "cursor-pointer"} ${className}`}
  >
    <input
      type="checkbox"
      role="switch"
      className="peer sr-only"
      checked={checked}
      disabled={disabled}
      aria-label={label}
      onChange={(event) => onChange(event.target.checked)}
    />
    <span
      aria-hidden="true"
      className={`relative h-[1.625rem] w-[2.625rem] rounded-full transition-colors duration-200 after:absolute after:start-0.5 after:top-0.5 after:h-[1.375rem] after:w-[1.375rem] after:rounded-full after:shadow-[0_1px_2px_rgba(0,0,0,0.2)] after:transition-[translate,background-color] after:duration-200 after:ease-out after:content-[''] peer-checked:after:translate-x-4 peer-focus-visible:ring-2 peer-disabled:opacity-50 rtl:peer-checked:after:-translate-x-4 ${
        tone === "onHero"
          ? "bg-hero-border after:bg-hero-control peer-checked:bg-hero-ink peer-checked:after:bg-hero-surface peer-focus-visible:ring-hero-ink/40"
          : "bg-toggle-track-off after:bg-white peer-checked:bg-toggle-track-on peer-focus-visible:ring-accent/40"
      }`}
    />
  </label>
);
