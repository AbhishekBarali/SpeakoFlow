import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import type { SettingIcon, SettingTone } from "../ui/tones";
import { useSettings } from "../../hooks/useSettings";

interface TranslateToEnglishProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  icon?: SettingIcon;
  tone?: SettingTone;
  /**
   * Shown under the label. Used by the cloud group to say what this engine can
   * and cannot do, since "Translate to English" is a Whisper task on the local
   * side and a separate `/audio/translations` route on the cloud side — and
   * several cloud providers have neither.
   */
  description?: string;
  /**
   * Locked off. The row still renders on purpose: hiding it would leave a user
   * who knows the feature exists hunting for a switch that is simply not
   * available for the provider they picked.
   */
  disabled?: boolean;
}

export const TranslateToEnglish: React.FC<TranslateToEnglishProps> = React.memo(
  ({
    descriptionMode = "tooltip",
    grouped = false,
    icon,
    tone,
    description,
    disabled = false,
  }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const translateToEnglish = getSetting("translate_to_english") || false;

    return (
      <ToggleSwitch
        checked={!disabled && translateToEnglish}
        onChange={(enabled) => updateSetting("translate_to_english", enabled)}
        isUpdating={isUpdating("translate_to_english")}
        disabled={disabled}
        label={t("settings.advanced.translateToEnglish.label")}
        description={description}
        icon={icon}
        tone={tone}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
