import React from "react";
import { useTranslation } from "react-i18next";
import { Cloud, Cpu } from "lucide-react";
import { Segmented } from "../../ui/Segmented";

export type ProviderMode = "device" | "cloud";

type ProviderModeToggleProps = {
  mode: ProviderMode;
  onChange: (mode: ProviderMode) => void;
  disabled?: boolean;
};

/** Shared location picker for model-backed features. One component so speech
 * to text, AI cleanup, and the assistant all say "where does this run" the
 * same way — with the same words and the same two pictures. */
export const ProviderModeToggle: React.FC<ProviderModeToggleProps> = ({
  mode,
  onChange,
  disabled = false,
}) => {
  const { t } = useTranslation();
  return (
    <Segmented
      label={t("settings.assistant.brain.whereLabel")}
      value={mode}
      onChange={onChange}
      disabled={disabled}
      options={[
        {
          value: "device",
          label: t("settings.assistant.brain.onDevice"),
          icon: Cpu,
        },
        {
          value: "cloud",
          label: t("settings.assistant.brain.cloud"),
          icon: Cloud,
        },
      ]}
    />
  );
};
