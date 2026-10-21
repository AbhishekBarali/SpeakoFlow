import React from "react";
import { useTranslation } from "react-i18next";
import type { ChineseScript as ChineseScriptValue } from "@/bindings";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import type { SettingIcon, SettingTone } from "../ui/tones";
import { useSettings } from "../../hooks/useSettings";
import { offersChineseScript } from "./chineseScriptRule";

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
 * picker, only where it can change something (`showsChineseScript`) and only
 * for someone it concerns (`offersChineseScript`).
 */
export const ChineseScript: React.FC<ChineseScriptProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
  supportedLanguages,
  supportsLanguageDetection = true,
  icon,
  tone,
}) => {
  const { t, i18n } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();

  const intent = getSetting("selected_language") || "auto";
  const selected = getSetting("chinese_script") ?? "auto";
  if (
    !offersChineseScript(
      intent,
      supportedLanguages,
      supportsLanguageDetection,
      {
        uiLanguage: i18n.language,
        scriptChosen: selected !== "auto",
      },
    )
  ) {
    return null;
  }

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
