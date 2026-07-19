import React from "react";
import { useTranslation } from "react-i18next";
import { Feather, Sparkles } from "lucide-react";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { Dropdown } from "@/components/ui/Dropdown";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
import { useSettings } from "@/hooks/useSettings";
import { type PostProcessCleanupStrength } from "@/bindings";
import {
  PostProcessingSettingsApi,
  PostProcessingSettingsPrompts,
  PostProcessingTone,
} from "../post-processing/PostProcessingSettings";
import { PostProcessTimeout } from "../PostProcessTimeout";
import { ShortcutInput } from "../ShortcutInput";

/**
 * "AI cleanup" — dedicated dictation cleanup controls with clear separation:
 * the shortcut, how the writing should sound, where the model runs, and what
 * cleanup instructions it follows.
 */
export const AiCleanupGroup: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();

  const enabled = getSetting("post_process_enabled") ?? false;
  const fixMisheard = getSetting("post_process_fix_misheard") ?? false;
  const cleanupStrength = getSetting("post_process_cleanup_strength") ?? "balanced";

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
        {enabled && (
          <>
            <ShortcutInput
              shortcutId="transcribe_with_post_process"
              grouped={true}
            />
            <SettingContainer
              title={t("settings.dictation.aiCleanup.strengthLabel")}
              info={t("settings.dictation.aiCleanup.strengthDescription")}
              layout="horizontal"
              grouped={true}
            >
              <Dropdown
                options={[
                  {
                    value: "light",
                    label: t("settings.dictation.aiCleanup.strengthOptions.light"),
                  },
                  {
                    value: "balanced",
                    label: t(
                      "settings.dictation.aiCleanup.strengthOptions.balanced",
                    ),
                  },
                  {
                    value: "aggressive",
                    label: t(
                      "settings.dictation.aiCleanup.strengthOptions.aggressive",
                    ),
                  },
                ]}
                selectedValue={cleanupStrength}
                onSelect={(value) =>
                  updateSetting(
                    "post_process_cleanup_strength",
                    value as PostProcessCleanupStrength,
                  )
                }
                disabled={isUpdating("post_process_cleanup_strength")}
                className="min-w-[150px]"
              />
            </SettingContainer>
            <ToggleSwitch
              checked={fixMisheard}
              onChange={(value) =>
                updateSetting("post_process_fix_misheard", value)
              }
              isUpdating={isUpdating("post_process_fix_misheard")}
              label={t("settings.dictation.aiCleanup.fixMisheardLabel")}
              description={t(
                "settings.dictation.aiCleanup.fixMisheardDescription",
              )}
              grouped={true}
            />
          </>
        )}
      </SettingsGroup>

      {enabled && (
        <>
          <SettingsGroup
            title={t("settings.dictation.aiCleanup.styleGroupTitle")}
            description={t(
              "settings.dictation.aiCleanup.styleGroupDescription",
            )}
            icon={Feather}
          >
            <PostProcessingTone />
          </SettingsGroup>

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
