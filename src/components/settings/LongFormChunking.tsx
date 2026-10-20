import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface LongFormChunkingProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Cut recordings over 40 seconds at pauses for models not built for long
 * audio (`managers::transcription::policy`). Whisper and streaming models are
 * never cut, so the switch does nothing for them.
 */
export const LongFormChunking: React.FC<LongFormChunkingProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const enabled = getSetting("long_form_chunking") ?? true;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(value) => updateSetting("long_form_chunking", value)}
        isUpdating={isUpdating("long_form_chunking")}
        label={t("settings.advanced.longFormChunking.title")}
        description={t("settings.advanced.longFormChunking.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
