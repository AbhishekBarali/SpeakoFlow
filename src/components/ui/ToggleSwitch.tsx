import React from "react";
import { SettingContainer } from "./SettingContainer";
import type { SettingIcon, SettingTone } from "./tones";

interface ToggleSwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  isUpdating?: boolean;
  label: string;
  description?: string;
  /** Deep-dive help behind the (i) hint. */
  info?: string;
  /** Optional leading icon tile (see SettingContainer). */
  icon?: SettingIcon;
  tone?: SettingTone;
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  tooltipPosition?: "top" | "bottom";
}

export const ToggleSwitch: React.FC<ToggleSwitchProps> = ({
  checked,
  onChange,
  disabled = false,
  isUpdating = false,
  label,
  description,
  info,
  icon,
  tone,
  descriptionMode = "tooltip",
  grouped = false,
  tooltipPosition = "top",
}) => {
  return (
    <SettingContainer
      title={label}
      description={description}
      info={info}
      icon={icon}
      tone={tone}
      descriptionMode={descriptionMode}
      grouped={grouped}
      disabled={disabled}
      tooltipPosition={tooltipPosition}
    >
      <label
        className={`inline-flex items-center ${disabled || isUpdating ? "cursor-not-allowed" : "cursor-pointer"}`}
      >
        <input
          type="checkbox"
          value=""
          className="sr-only peer"
          checked={checked}
          disabled={disabled || isUpdating}
          onChange={(e) => onChange(e.target.checked)}
        />
        <div className="relative w-[42px] h-[26px] bg-hairline-strong peer-focus-visible:outline-none peer-focus-visible:ring-2 peer-focus-visible:ring-accent/40 rounded-full peer peer-checked:after:translate-x-4 rtl:peer-checked:after:-translate-x-4 after:content-[''] after:absolute after:top-0.5 after:start-0.5 after:bg-white after:rounded-full after:h-[22px] after:w-[22px] after:shadow-[0_1px_2px_rgba(0,0,0,0.2)] after:transition-transform after:duration-200 after:ease-out transition-colors duration-200 peer-checked:bg-toggle-track-on peer-checked:after:bg-toggle-knob-on peer-disabled:opacity-50"></div>
      </label>
      {isUpdating && (
        <div className="absolute inset-0 flex items-center justify-center">
          <div className="w-4 h-4 border-2 border-ink border-t-transparent rounded-full animate-spin"></div>
        </div>
      )}
    </SettingContainer>
  );
};
