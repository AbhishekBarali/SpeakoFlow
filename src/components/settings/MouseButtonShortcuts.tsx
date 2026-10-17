import React from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "../../hooks/useOsType";
import { hasMouseButton } from "../../lib/utils/keyboard";

interface MouseButtonShortcutsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Experimental: the middle, Back and Forward mouse buttons as shortcut keys.
 *
 * Turning it off moves any shortcut whose own keys are a mouse button back to
 * its default keys (the backend does that, so dictation never ends up with no
 * key), which is why turning it off reloads the settings and says so.
 */
export const MouseButtonShortcuts: React.FC<MouseButtonShortcutsProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating, refreshSettings } =
      useSettings();
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

    const onChange = async (on: boolean) => {
      const bindings = getSetting("bindings") ?? {};
      const moving = Object.values(bindings).some((binding) =>
        hasMouseButton(binding?.current_binding),
      );
      const saved = await updateSetting("mouse_button_shortcuts", on);
      if (!on && saved && moving) {
        await refreshSettings();
        toast(t("settings.advanced.mouseButtonShortcuts.reset"));
      }
    };

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(on) => void onChange(on)}
        isUpdating={isUpdating("mouse_button_shortcuts")}
        label={t("settings.advanced.mouseButtonShortcuts.label")}
        description={description}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  });
