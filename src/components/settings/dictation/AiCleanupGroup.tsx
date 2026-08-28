import React from "react";
import { useTranslation } from "react-i18next";
import { Layers, Sparkles } from "lucide-react";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
import { Alert } from "@/components/ui/Alert";
import { useSettings } from "@/hooks/useSettings";
import { useModelStore } from "@/stores/modelStore";
import { isCleanupSpecialistModel } from "@/lib/utils/cleanupSpecialist";
import {
  PostProcessingSettingsApi,
  PostProcessingSettingsPrompts,
  PostProcessingTone,
} from "../post-processing/PostProcessingSettings";
import { PostProcessTimeout } from "../PostProcessTimeout";
import { PostProcessUnloadTimeout } from "../PostProcessUnloadTimeout";
import { ShortcutInput } from "../ShortcutInput";

interface AiCleanupGroupProps {
  /** Opens the cleanup-model catalog, which the page above owns as a sub-page. */
  onBrowseCleanupModels: () => void;
}

/**
 * "AI cleanup" — three groups answering three questions in order: is it on,
 * which model does it, and what is that model told.
 *
 * The instruction part is a fixed two-layer hierarchy, which is the whole point
 * of this layout. Layer 1 (the cleanup system prompt) decides what corrections
 * happen; layer 2 (the writing style) sits on top and decides how the result
 * reads. Anything else the app used to add — a strength dial, a misheard-word
 * toggle, and a "use my prompt exactly" switch — is gone: the first two were
 * extra prose competing with the prompt for the model's attention, and the third
 * asked the user to know something the app can work out itself, since whether a
 * model needs the app's scaffolding is a property of the model.
 */
export const AiCleanupGroup: React.FC<AiCleanupGroupProps> = ({
  onBrowseCleanupModels,
}) => {
  const { t } = useTranslation();
  const {
    getSetting,
    updateSetting,
    isUpdating,
    settings,
    postProcessReadiness,
  } = useSettings();
  const { models } = useModelStore();

  const enabled = getSetting("post_process_enabled") ?? false;

  // Is the model doing cleanup one that was trained for it? If so the app sends
  // only the layers the user chose, and the copy says so — otherwise a user would
  // reasonably assume the long default prompt is helping when it is not.
  //
  // Read from readiness first, because that is the model the backend actually
  // resolved: cleanup falls back to the assistant's provider when the dedicated
  // selection is incomplete, so the stored selection alone can disagree with what
  // will really run. The stored selection is the fallback for the moment before
  // the first readiness check lands.
  const resolvedModel =
    postProcessReadiness?.state === "ready"
      ? postProcessReadiness.model
      : (settings?.post_process_models?.[
          settings?.post_process_provider_id ?? ""
        ] ?? "");
  const specialistActive =
    isCleanupSpecialistModel(resolvedModel) ||
    models.some(
      (model) => model.id === resolvedModel && model.is_cleanup_specialist,
    );

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
          <ShortcutInput
            shortcutId="transcribe_with_post_process"
            grouped={true}
          />
        )}
      </SettingsGroup>

      {enabled && (
        <>
          {/* No group description here on purpose: the segmented "On my device /
              Cloud provider" control below says it faster than a sentence can,
              and the row keeps its tooltip for the detail. */}
          <SettingsGroup
            title={t("settings.dictation.aiCleanup.modelGroupTitle")}
          >
            <PostProcessingSettingsApi onBrowseModels={onBrowseCleanupModels} />
            <PostProcessTimeout grouped={true} />
            <PostProcessUnloadTimeout grouped={true} />
          </SettingsGroup>

          {/* Title only. "1 ·" and "2 ·" on the rows already say which layer runs
              first, and each row keeps its own detail in a tooltip. */}
          <SettingsGroup
            title={t("settings.dictation.aiCleanup.promptGroupTitle")}
            icon={Layers}
          >
            {specialistActive && (
              <Alert variant="info" contained>
                {t("settings.dictation.aiCleanup.tunedModelNotice")}
              </Alert>
            )}
            <PostProcessingSettingsPrompts />
            <PostProcessingTone />
          </SettingsGroup>
        </>
      )}
    </>
  );
};
