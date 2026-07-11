import React from "react";
import { useTranslation } from "react-i18next";
import { ShowOverlay } from "../ShowOverlay";
import { ModelUnloadTimeoutSetting } from "../ModelUnloadTimeout";
import { CustomWords } from "../CustomWords";
import { TextReplacements } from "../TextReplacements";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { MoreOptions } from "../../ui/MoreOptions";
import { StartHidden } from "../StartHidden";
import { AutostartToggle } from "../AutostartToggle";
import { ShowTrayIcon } from "../ShowTrayIcon";
import { PasteMethodSetting } from "../PasteMethod";
import { TypingToolSetting } from "../TypingTool";
import { ClipboardHandlingSetting } from "../ClipboardHandling";
import { AutoSubmit } from "../AutoSubmit";
import { PostProcessingToggle } from "../PostProcessingToggle";
import { PostProcessTimeout } from "../PostProcessTimeout";
import { AppendTrailingSpace } from "../AppendTrailingSpace";
import { HistoryLimit } from "../HistoryLimit";
import { RecordingRetentionPeriodSelector } from "../RecordingRetentionPeriod";
import { ExperimentalToggle } from "../ExperimentalToggle";
import { useSettings } from "../../../hooks/useSettings";
import { KeyboardImplementationSelector } from "../debug/KeyboardImplementationSelector";
import { AccelerationSelector } from "../AccelerationSelector";
import { LazyStreamClose } from "../LazyStreamClose";
import { AlwaysOnMicrophone } from "../AlwaysOnMicrophone";
import { OverlayStyle } from "../OverlayStyle";

export const AdvancedSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const experimentalEnabled = getSetting("experimental_enabled") || false;

  return (
    <div className="max-w-2xl w-full mx-auto space-y-8">
      <SettingsGroup title={t("settings.advanced.groups.app")}>
        <AutostartToggle descriptionMode="tooltip" grouped={true} />
        <StartHidden descriptionMode="tooltip" grouped={true} />
        <ShowTrayIcon descriptionMode="tooltip" grouped={true} />
        <OverlayStyle descriptionMode="tooltip" grouped={true} />
        <MoreOptions>
          <ShowOverlay descriptionMode="tooltip" grouped={true} />
          <ModelUnloadTimeoutSetting descriptionMode="tooltip" grouped={true} />
          <ExperimentalToggle descriptionMode="tooltip" grouped={true} />
        </MoreOptions>
      </SettingsGroup>

      <SettingsGroup title={t("settings.advanced.groups.output")}>
        <PasteMethodSetting descriptionMode="tooltip" grouped={true} />
        <MoreOptions>
          <TypingToolSetting descriptionMode="tooltip" grouped={true} />
          <ClipboardHandlingSetting descriptionMode="tooltip" grouped={true} />
          <AutoSubmit descriptionMode="tooltip" grouped={true} />
        </MoreOptions>
      </SettingsGroup>

      <SettingsGroup title={t("settings.advanced.groups.transcription")}>
        <AlwaysOnMicrophone descriptionMode="tooltip" grouped={true} />
        <CustomWords descriptionMode="tooltip" grouped />
        <MoreOptions>
          <AppendTrailingSpace descriptionMode="tooltip" grouped={true} />
        </MoreOptions>
      </SettingsGroup>

      <SettingsGroup title={t("settings.advanced.groups.textReplacements")}>
        <TextReplacements descriptionMode="tooltip" grouped />
      </SettingsGroup>

      <SettingsGroup title={t("settings.advanced.groups.history")}>
        <RecordingRetentionPeriodSelector
          descriptionMode="tooltip"
          grouped={true}
        />
        {getSetting("recording_retention_period") === "preserve_limit" && (
          <HistoryLimit descriptionMode="tooltip" grouped={true} />
        )}
      </SettingsGroup>

      {experimentalEnabled && (
        <SettingsGroup title={t("settings.advanced.groups.experimental")}>
          <PostProcessingToggle descriptionMode="tooltip" grouped={true} />
          {getSetting("post_process_enabled") && (
            <PostProcessTimeout descriptionMode="tooltip" grouped={true} />
          )}
          <KeyboardImplementationSelector
            descriptionMode="tooltip"
            grouped={true}
          />
          <AccelerationSelector descriptionMode="tooltip" grouped={true} />
          <LazyStreamClose descriptionMode="tooltip" grouped={true} />
        </SettingsGroup>
      )}
    </div>
  );
};
