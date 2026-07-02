import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface TapToLockProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Toggle for the "tap Shift to lock a hold recording hands-free" gesture.
 * While a push-to-talk recording is held, a quick Shift tap converts it to
 * hands-free so you can release the hotkey and keep talking. Turning this off
 * stops a stray Shift tap from locking recordings. Only relevant when
 * Push To Talk is on.
 */
export const TapToLock: React.FC<TapToLockProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    // Defaults to on to match the backend default when the field is absent.
    const tapToLockEnabled = getSetting("tap_to_lock") ?? true;

    return (
      <ToggleSwitch
        checked={tapToLockEnabled}
        onChange={(enabled) => updateSetting("tap_to_lock", enabled)}
        isUpdating={isUpdating("tap_to_lock")}
        label={t("settings.general.tapToLock.label")}
        description={t("settings.general.tapToLock.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
