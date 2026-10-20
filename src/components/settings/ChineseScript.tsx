import React from "react";
import { useTranslation } from "react-i18next";
import type { ChineseScript as ChineseScriptValue } from "@/bindings";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import type { SettingIcon, SettingTone } from "../ui/tones";
import { useSettings } from "../../hooks/useSettings";
import { showsChineseScript } from "./chineseScriptRule";

interface ChineseScriptProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  /** The current model's languages; none for the cloud. */
  supportedLanguages?: string[];
  supportsLanguageDetection?: boolean;
  icon?: SettingIcon;
  tone?: SettingTone;
}

const SCRIPTS: ChineseScriptValue[] = ["auto", "simplified", "traditional"];

/**
 * The script Chinese transcripts are written in. Rendered under the language
 * picker, and only where it can change something (see `showsChineseScript`).
 */
export const ChineseScript: React.FC<ChineseScriptProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
  supportedLanguages,
  supportsLanguageDetection = true,
  icon,
  tone,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();

  const intent = getSetting("selected_language") || "auto";
  if (
    !showsChineseScript(intent, supportedLanguages, supportsLanguageDetection)
  ) {
    return null;
  }

  const selected = getSetting("chinese_script") ?? "auto";
  const options = SCRIPTS.map((value) => ({
    value,
    label: t(`settings.general.chineseScript.options.${value}`),
  }));

  return (
    <SettingContainer
      title={t("settings.general.chineseScript.title")}
      description={t("settings.general.chineseScript.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      icon={icon}
      tone={tone}
    >
      <Dropdown
        options={options}
        selectedValue={selected}
        onSelect={(value) =>
          void updateSetting("chinese_script", value as ChineseScriptValue)
        }
        disabled={isUpdating("chinese_script")}
      />
    </SettingContainer>
  );
};
