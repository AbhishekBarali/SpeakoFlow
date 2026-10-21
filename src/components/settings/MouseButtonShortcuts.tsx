import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "../../hooks/useOsType";

interface MouseButtonShortcutsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * The middle, Back and Forward mouse buttons as shortcut keys. In Settings →
 * Shortcuts on Windows and macOS; on Linux it stays under Experimental, since
 * there it needs the SpeakoFlow Keys engine and only the middle button works.
 *
 * A button is only ever a shortcut's other keys, never its own (the backend
 * refuses that), so turning this off changes no shortcut: its button is kept
 * but not listened for, and works again once this is back on.
 */
export const MouseButtonShortcuts: React.FC<MouseButtonShortcutsProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const os = useOsType();

    const enabled = getSetting("mouse_button_shortcuts") ?? false;
    const engine = getSetting("keyboard_implementation") ?? "tauri";

    const description = [
      t("settings.advanced.mouseButtonShortcuts.description"),
      // Back and Forward never reach the Linux key listener.
      os === "linux" ? t("settings.advanced.mouseButtonShortcuts.linux") : "",
      engine === "handy_keys"
        ? ""
        : t("settings.advanced.mouseButtonShortcuts.needsEngine"),
    ]
      .filter(Boolean)
      .join(" ");

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(on) => void updateSetting("mouse_button_shortcuts", on)}
        isUpdating={isUpdating("mouse_button_shortcuts")}
        label={t("settings.advanced.mouseButtonShortcuts.label")}
        description={description}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  });
