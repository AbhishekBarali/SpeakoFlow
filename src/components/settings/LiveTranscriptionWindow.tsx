import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface LiveTranscriptionWindowProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const LiveTranscriptionWindow: React.FC<LiveTranscriptionWindowProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    // The live-transcription window only shows text while live transcription is
    // actually running, so it's paired with (and disabled without) that
    // setting. The toggle itself stays visible for discoverability.
    const liveEnabled = getSetting("live_transcription_enabled") ?? false;
    const enabled = getSetting("live_transcription_window_enabled") ?? false;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(value) =>
          updateSetting("live_transcription_window_enabled", value)
        }
        isUpdating={isUpdating("live_transcription_window_enabled")}
        disabled={!liveEnabled}
        label={t("settings.advanced.liveTranscriptionWindow.label")}
        description={t("settings.advanced.liveTranscriptionWindow.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  });
