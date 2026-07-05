import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import type { SettingIcon, SettingTone } from "../ui/tones";
import { useSettings } from "@/hooks/useSettings";
import type { UiTextSize } from "@/bindings";

interface TextSizeSelectorProps {
  grouped?: boolean;
  icon?: SettingIcon;
  tone?: SettingTone;
}

/**
 * UI text size preference. The backend applies the choice as a webview zoom
 * factor on the main window, so every surface (px and rem alike) scales
 * together — no per-component font juggling.
 */
export const TextSizeSelector: React.FC<TextSizeSelectorProps> = React.memo(
  ({ grouped = false, icon, tone }) => {
    const { t } = useTranslation();
    const { settings, updateSetting } = useSettings();

    const current = (settings?.ui_text_size ?? "default") as UiTextSize;

    const options = [
      { value: "small", label: t("appearance.textSizeSmall") },
      { value: "default", label: t("appearance.textSizeDefault") },
      { value: "large", label: t("appearance.textSizeLarge") },
      { value: "extra_large", label: t("appearance.textSizeExtraLarge") },
    ];

    return (
      <SettingContainer
        title={t("appearance.textSize")}
        info={t("appearance.textSizeDescription")}
        icon={icon}
        tone={tone}
        grouped={grouped}
      >
        <Dropdown
          options={options}
          selectedValue={current}
          onSelect={(value) =>
            updateSetting("ui_text_size", value as UiTextSize)
          }
        />
      </SettingContainer>
    );
  },
);

TextSizeSelector.displayName = "TextSizeSelector";
