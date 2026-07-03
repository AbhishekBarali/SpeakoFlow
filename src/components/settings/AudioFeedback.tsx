import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import type { SettingIcon, SettingTone } from "../ui/tones";
import { useSettings } from "../../hooks/useSettings";
import { VolumeSlider } from "./VolumeSlider";
import { SoundPicker } from "./SoundPicker";

interface AudioFeedbackProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  icon?: SettingIcon;
  tone?: SettingTone;
}

export const AudioFeedback: React.FC<AudioFeedbackProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false, icon, tone }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const audioFeedbackEnabled = getSetting("audio_feedback") || false;

    return (
      <div className="flex flex-col">
        <ToggleSwitch
          checked={audioFeedbackEnabled}
          onChange={(enabled) => updateSetting("audio_feedback", enabled)}
          isUpdating={isUpdating("audio_feedback")}
          label={t("settings.sound.audioFeedback.label")}
          description={t("settings.sound.audioFeedback.description")}
          icon={icon}
          tone={tone}
          descriptionMode={descriptionMode}
          grouped={grouped}
        />
      </div>
    );
  },
);
