import React from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";

interface PostProcessTimeoutProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

/**
 * How long AI Correction waits for the LLM before giving up and pasting the raw
 * transcription instead. Keeps a slow or stuck model from ever holding up the
 * paste.
 */
export const PostProcessTimeout: React.FC<PostProcessTimeoutProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();

  const current = getSetting("post_process_timeout_secs") ?? 10;

  const options = [5, 10, 15, 20, 30, 60].map((seconds) => ({
    value: String(seconds),
    label: t("settings.advanced.postProcessTimeout.seconds", { seconds }),
  }));

  return (
    <SettingContainer
      title={t("settings.advanced.postProcessTimeout.title")}
      description={t("settings.advanced.postProcessTimeout.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
    >
      <Dropdown
        options={options}
        selectedValue={String(current)}
        onSelect={(value) =>
          updateSetting("post_process_timeout_secs", Number(value))
        }
      />
    </SettingContainer>
  );
};
