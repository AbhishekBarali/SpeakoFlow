import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface QuitOnCloseProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Lets the user choose what closing the main window does (GitHub issue #6).
 *
 * The backing setting is a string enum ("minimize_to_tray" | "quit"). The
 * default stays "minimize_to_tray" so existing behavior is preserved for
 * everyone; this toggle maps ON -> quit, OFF -> minimize to tray.
 */
export const QuitOnClose: React.FC<QuitOnCloseProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const closeBehavior = getSetting("close_behavior") ?? "minimize_to_tray";
    const quitOnClose = closeBehavior === "quit";

    return (
      <ToggleSwitch
        checked={quitOnClose}
        onChange={(enabled) =>
          updateSetting(
            "close_behavior",
            enabled ? "quit" : "minimize_to_tray",
          )
        }
        isUpdating={isUpdating("close_behavior")}
        label={t("settings.advanced.closeBehavior.label")}
        description={t("settings.advanced.closeBehavior.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
        tooltipPosition="bottom"
      />
    );
  },
);
