import React from "react";
import { useTranslation } from "react-i18next";
import { Slider } from "../ui/Slider";
import { useSettings } from "../../hooks/useSettings";

export const VolumeSlider: React.FC<{ disabled?: boolean }> = ({
  disabled = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  // Matches `default_audio_feedback_volume()`. The old 0.5 fallback disagreed
  // with the backend's 1.0, so a store that had not loaded yet showed 50% while
  // the sounds played at full volume.
  const audioFeedbackVolume = getSetting("audio_feedback_volume") ?? 1;

  return (
    <Slider
      value={audioFeedbackVolume}
      onChange={(value: number) =>
        updateSetting("audio_feedback_volume", value)
      }
      min={0}
      max={1}
      label={t("settings.sound.volume.title")}
      grouped
      formatValue={(value) => `${Math.round(value * 100)}%`}
      disabled={disabled}
    />
  );
};
