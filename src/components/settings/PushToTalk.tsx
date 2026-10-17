import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

/** Dictation and the ask are each held or tapped on their own. */
export type RecordingModeSetting = "push_to_talk" | "assistant_push_to_talk";

interface PushToTalkProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  /** Whose mode: dictation (the default) or the ask. */
  setting?: RecordingModeSetting;
}

/**
 * Hold to talk (or Hold to ask): a plain switch on a row of its own. On, the
 * keys are held while you speak; off, one press starts and the next one stops.
 * A Hold | Tap control on the same row as the keys made every row a cluster of
 * controls, and "Recording behavior" said nothing about what it changed.
 */
export const PushToTalk: React.FC<PushToTalkProps> = React.memo(
  ({
    descriptionMode = "tooltip",
    grouped = false,
    setting = "push_to_talk",
  }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const ask = setting === "assistant_push_to_talk";
    return (
      <ToggleSwitch
        checked={getSetting(setting) ?? true}
        onChange={(hold) => void updateSetting(setting, hold)}
        isUpdating={isUpdating(setting)}
        label={
          ask
            ? t("settings.general.pushToTalk.assistantLabel")
            : t("settings.general.pushToTalk.dictationLabel")
        }
        description={
          ask
            ? t("settings.general.pushToTalk.assistant")
            : t("settings.general.pushToTalk.dictation")
        }
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);

PushToTalk.displayName = "PushToTalk";
