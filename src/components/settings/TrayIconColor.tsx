import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "../../hooks/useOsType";

interface TrayIconColorProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Linux only: the tray icon is a white glyph like the panel's own icons, and
 * this puts the colour logo back for a light panel, where white cannot be
 * seen. The other platforms always draw the colour logo, so they get no row,
 * and neither does a hidden tray.
 */
export const TrayIconColor: React.FC<TrayIconColorProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const osType = useOsType();

    if (osType !== "linux" || !(getSetting("show_tray_icon") ?? true)) {
      return null;
    }

    return (
      <ToggleSwitch
        checked={getSetting("tray_icon_color") ?? false}
        onChange={(enabled) => updateSetting("tray_icon_color", enabled)}
        isUpdating={isUpdating("tray_icon_color")}
        label={t("settings.advanced.trayIconColor.label")}
        description={t("settings.advanced.trayIconColor.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
        tooltipPosition="bottom"
      />
    );
  },
);
