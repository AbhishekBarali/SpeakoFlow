import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import type { SettingIcon, SettingTone } from "../ui/tones";
import { useSettings } from "../../hooks/useSettings";

interface MuteWhileRecordingToggleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  icon?: SettingIcon;
  tone?: SettingTone;
}

export const MuteWhileRecording: React.FC<MuteWhileRecordingToggleProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false, icon, tone }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const muteEnabled = getSetting("mute_while_recording") ?? false;

    return (
      <ToggleSwitch
        checked={muteEnabled}
        onChange={(enabled) => updateSetting("mute_while_recording", enabled)}
        isUpdating={isUpdating("mute_while_recording")}
        label={t("settings.debug.muteWhileRecording.label")}
        icon={icon}
        tone={tone}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  });
