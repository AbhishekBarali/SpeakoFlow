import React from "react";
import { useTranslation } from "react-i18next";
import { Sparkles } from "lucide-react";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
import { useSettings } from "@/hooks/useSettings";
import {
  PostProcessingSettingsApi,
  PostProcessingSettingsPrompts,
  PostProcessingTone,
} from "../post-processing/PostProcessingSettings";
import { PostProcessTimeout } from "../PostProcessTimeout";
import { ShortcutInput } from "../ShortcutInput";

/**
 * "AI cleanup" — the dictation post-processing feature, renamed for humans.
 *
 * Off: a single toggle row. On: three clean groups, all visible — the basics
 * (tone + its own hotkey), the model that does the cleanup, and the cleanup
 * prompt. No folds; the page just grows when the feature is on.
 */
export const AiCleanupGroup: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();

  const enabled = getSetting("post_process_enabled") ?? false;

  return (
    <>
      <SettingsGroup
        title={t("settings.dictation.aiCleanup.groupTitle")}
        icon={Sparkles}
      >
        <ToggleSwitch
          checked={enabled}
          onChange={(value) => updateSetting("post_process_enabled", value)}
          isUpdating={isUpdating("post_process_enabled")}
          label={t("settings.dictation.aiCleanup.title")}
          description={t("settings.dictation.aiCleanup.caption")}
          grouped={true}
        />
        {enabled && <PostProcessingTone />}
        {enabled && (
          <ShortcutInput
            shortcutId="transcribe_with_post_process"
            grouped={true}
          />
        )}
      </SettingsGroup>

      {enabled && (
        <>
          <SettingsGroup
            title={t("settings.dictation.aiCleanup.modelGroupTitle")}
          >
            <PostProcessingSettingsApi />
            <PostProcessTimeout grouped={true} />
          </SettingsGroup>

          <SettingsGroup
            title={t("settings.dictation.aiCleanup.promptGroupTitle")}
          >
            <PostProcessingSettingsPrompts />
          </SettingsGroup>
        </>
      )}
    </>
  );
};
