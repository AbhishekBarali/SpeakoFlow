import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import type { SettingIcon, SettingTone } from "../ui/tones";
import { useSettings } from "@/hooks/useSettings";
import { applyThemePreference, type ThemePreference } from "@/lib/theme";

interface AppearanceSelectorProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  icon?: SettingIcon;
  tone?: SettingTone;
}

export const AppearanceSelector: React.FC<AppearanceSelectorProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false, icon, tone }) => {
    const { t } = useTranslation();
    const { settings, updateSetting } = useSettings();

    const current = (settings?.theme ?? "light") as ThemePreference;

    const options = [
      { value: "system", label: t("appearance.system") },
      { value: "light", label: t("appearance.light") },
      { value: "dark", label: t("appearance.dark") },
    ];

    const handleChange = (value: string) => {
      const preference = value as ThemePreference;
      // Apply immediately for instant feedback, then persist to the backend.
      applyThemePreference(preference);
      updateSetting("theme", preference);
    };

    return (
      <SettingContainer
        title={t("appearance.title")}
        icon={icon}
        tone={tone}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <Dropdown
          options={options}
          selectedValue={current}
          onSelect={handleChange}
        />
      </SettingContainer>
    );
  },
);

AppearanceSelector.displayName = "AppearanceSelector";
