import React from "react";
import { SettingContainer } from "./SettingContainer";

interface SliderProps {
  value: number;
  onChange: (value: number) => void;
  min: number;
  max: number;
  step?: number;
  disabled?: boolean;
  label: string;
  description?: string;
  /** Optional deep-dive help shown behind a small (i) icon, matching the
   *  dropdown rows (e.g. Panel size) so slider rows can carry the same hint. */
  info?: string;
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  showValue?: boolean;
  formatValue?: (value: number) => string;
  /** Tailwind width class for the control column. Defaults to full width. The
   *  assistant opacity row passes a fixed width so the slider lines up with the
   *  dropdown rows above it instead of running wider and starting further left. */
  controlClassName?: string;
}

export const Slider: React.FC<SliderProps> = ({
  value,
  onChange,
  min,
  max,
  step = 0.01,
  disabled = false,
  label,
  description,
  info,
  descriptionMode = "tooltip",
  grouped = false,
  showValue = true,
  formatValue = (v) => v.toFixed(2),
  controlClassName = "w-full",
}) => {
  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    onChange(parseFloat(e.target.value));
  };

  return (
    <SettingContainer
      title={label}
      description={description}
      info={info}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="horizontal"
      disabled={disabled}
    >
      <div className={controlClassName}>
        <div className="flex items-center space-x-1 h-6">
          <input
            type="range"
            min={min}
            max={max}
            step={step}
            value={value}
            onChange={handleChange}
            disabled={disabled}
            className="flex-grow h-2 rounded-full appearance-none cursor-pointer focus:outline-none focus:ring-2 focus:ring-ink/20 disabled:opacity-50 disabled:cursor-not-allowed"
            style={{
              background: `linear-gradient(to right, var(--color-background-ui) ${
                ((value - min) / (max - min)) * 100
              }%, var(--color-hairline) ${
                ((value - min) / (max - min)) * 100
              }%)`,
            }}
          />
          {showValue && (
            <span className="text-sm font-medium text-ink w-12 text-end">
              {formatValue(value)}
            </span>
          )}
        </div>
      </div>
    </SettingContainer>
  );
};
