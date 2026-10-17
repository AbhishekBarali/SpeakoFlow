import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface SmartSpacingProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Smart spacing: fit each dictation onto the text already in the field, so
 * stopping and continuing never runs words together. The two refinements only
 * appear once it is on, because they mean nothing without it.
 */
export const SmartSpacing: React.FC<SmartSpacingProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const enabled = getSetting("smart_spacing") ?? true;
    const fixCase = getSetting("smart_spacing_fix_case") ?? true;
    const joinSentences = getSetting("smart_spacing_join_sentences") ?? true;

    return (
      <>
        <ToggleSwitch
          checked={enabled}
          onChange={(value) => updateSetting("smart_spacing", value)}
          isUpdating={isUpdating("smart_spacing")}
          label={t("settings.debug.smartSpacing.label")}
          description={t("settings.debug.smartSpacing.description")}
          info={t("settings.debug.smartSpacing.info")}
          descriptionMode={descriptionMode}
          grouped={grouped}
        />
        {enabled && (
          <>
            <ToggleSwitch
              checked={fixCase}
              onChange={(value) =>
                updateSetting("smart_spacing_fix_case", value)
              }
              isUpdating={isUpdating("smart_spacing_fix_case")}
              label={t("settings.debug.smartSpacingFixCase.label")}
              description={t("settings.debug.smartSpacingFixCase.description")}
              descriptionMode={descriptionMode}
              grouped={grouped}
            />
            <ToggleSwitch
              checked={joinSentences}
              onChange={(value) =>
                updateSetting("smart_spacing_join_sentences", value)
              }
              isUpdating={isUpdating("smart_spacing_join_sentences")}
              label={t("settings.debug.smartSpacingJoin.label")}
              description={t("settings.debug.smartSpacingJoin.description")}
              descriptionMode={descriptionMode}
              grouped={grouped}
            />
          </>
        )}
      </>
    );
  },
);
