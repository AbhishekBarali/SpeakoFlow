import React from "react";
import { useTranslation } from "react-i18next";
import type { ThinkingLevel } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { THINKING_LEVELS, thinkingUnsupportedReason } from "@/lib/thinking";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";

interface ThinkingLevelSettingProps {
  /** Which job the dial belongs to. Each keeps its own level. */
  job: "cleanup" | "assistant";
  /** The provider the job runs on, which decides whether the dial applies. */
  providerId: string | null | undefined;
  grouped?: boolean;
}

const SETTING_KEY = {
  cleanup: "post_process_thinking",
  assistant: "assistant_thinking",
} as const;

/**
 * How much a reasoning model may think before it answers. Off is the fast path
 * the app always used; a provider with no request-level control gets a
 * disabled dial and the reason, rather than a setting that does nothing.
 */
export const ThinkingLevelSetting: React.FC<ThinkingLevelSettingProps> = ({
  job,
  providerId,
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const key = SETTING_KEY[job];
  const current: ThinkingLevel = getSetting(key) ?? "off";
  const unsupported = thinkingUnsupportedReason(providerId);

  const options = THINKING_LEVELS.map((level) => ({
    value: level,
    label: t(`settings.thinking.options.${level}`),
  }));

  return (
    <SettingContainer
      title={t("settings.thinking.title")}
      description={
        unsupported
          ? t(`settings.thinking.unsupported.${unsupported}`)
          : t(`settings.thinking.description.${job}`)
      }
      // A disabled control needs its reason in sight, not behind the (i).
      descriptionMode={unsupported ? "caption" : "tooltip"}
      layout="horizontal"
      grouped={grouped}
    >
      <Dropdown
        options={options}
        selectedValue={unsupported ? "off" : current}
        disabled={unsupported !== null || isUpdating(key)}
        onSelect={(value) => void updateSetting(key, value as ThinkingLevel)}
      />
    </SettingContainer>
  );
};
