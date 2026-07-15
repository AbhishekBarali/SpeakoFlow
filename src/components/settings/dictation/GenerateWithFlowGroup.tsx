import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Wand2 } from "lucide-react";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
import { Input } from "@/components/ui/Input";
import { useSettings } from "@/hooks/useSettings";

/**
 * "Generate with Flow" — dictation that begins with the activation phrase
 * (default "Hey Flow") becomes a one-shot AI generation command: the finished
 * result is pasted instead of the spoken words. Uses the assistant's provider
 * and model; screen access is permissioned separately from the assistant's.
 */
export const GenerateWithFlowGroup: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();

  const enabled = getSetting("flow_enabled") ?? false;
  const screenAccess = getSetting("flow_screen_access") ?? false;
  const savedPhrase = getSetting("flow_phrase") ?? "Hey Flow";

  // Local draft so typing doesn't write settings per keystroke; committed on
  // blur or Enter. An empty phrase resets to the default in the backend.
  const [phrase, setPhrase] = useState(savedPhrase);
  useEffect(() => setPhrase(savedPhrase), [savedPhrase]);

  const commitPhrase = () => {
    const next = phrase.trim();
    if (next === savedPhrase) return;
    updateSetting("flow_phrase", next === "" ? "Hey Flow" : next);
  };

  return (
    <SettingsGroup title={t("settings.dictation.flow.groupTitle")} icon={Wand2}>
      <ToggleSwitch
        checked={enabled}
        onChange={(value) => updateSetting("flow_enabled", value)}
        isUpdating={isUpdating("flow_enabled")}
        label={t("settings.dictation.flow.enableLabel")}
        description={t("settings.dictation.flow.enableDescription")}
        grouped={true}
      />
      {enabled && (
        <>
          <SettingContainer
            title={t("settings.dictation.flow.phraseLabel")}
            info={t("settings.dictation.flow.phraseDescription")}
            layout="horizontal"
            grouped={true}
          >
            <Input
              value={phrase}
              onChange={(e) => setPhrase(e.target.value)}
              onBlur={commitPhrase}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  (e.target as HTMLInputElement).blur();
                }
              }}
              placeholder={t("settings.dictation.flow.phrasePlaceholder")}
              variant="compact"
              className="w-44"
            />
          </SettingContainer>
          <ToggleSwitch
            checked={screenAccess}
            onChange={(value) => updateSetting("flow_screen_access", value)}
            isUpdating={isUpdating("flow_screen_access")}
            label={t("settings.dictation.flow.screenLabel")}
            description={t("settings.dictation.flow.screenDescription")}
            grouped={true}
          />
        </>
      )}
    </SettingsGroup>
  );
};
