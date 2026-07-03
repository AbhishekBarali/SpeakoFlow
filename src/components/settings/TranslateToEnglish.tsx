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
}

export const TranslateToEnglish: React.FC<TranslateToEnglishProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false, icon, tone }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const translateToEnglish = getSetting("translate_to_english") || false;

    return (
      <ToggleSwitch
        checked={translateToEnglish}
        onChange={(enabled) => updateSetting("translate_to_english", enabled)}
        isUpdating={isUpdating("translate_to_english")}
        label={t("settings.advanced.translateToEnglish.label")}
        icon={icon}
        tone={tone}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
