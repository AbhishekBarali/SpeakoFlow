import React from "react";
import { useTranslation } from "react-i18next";
import { type } from "@tauri-apps/plugin-os";
import {
  Keyboard,
  AudioLines,
  Ban,
  Palette,
  SunMoon,
  Type,
  Mic,
} from "lucide-react";
import { MicrophoneSelector } from "../MicrophoneSelector";
import { ShortcutInput } from "../ShortcutInput";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SectionHeader } from "../../ui/SectionHeader";
import { OutputDeviceSelector } from "../OutputDeviceSelector";
import { AudioFeedback } from "../AudioFeedback";
import { SoundPicker } from "../SoundPicker";
import { AppearanceSelector } from "../AppearanceSelector";
import { TextSizeSelector } from "../TextSizeSelector";
import { PushToTalk } from "../PushToTalk";
import { useSettings } from "../../../hooks/useSettings";
import { VolumeSlider } from "../VolumeSlider";
import { MuteWhileRecording } from "../MuteWhileRecording";
import { AutostartToggle } from "../AutostartToggle";
import { StartHidden } from "../StartHidden";
import { ShowTrayIcon } from "../ShowTrayIcon";
import { OverlayStyle } from "../OverlayStyle";
import { ShowOverlay } from "../ShowOverlay";
import { UpdateChecksToggle } from "../UpdateChecksToggle";
import { ModelUnloadTimeoutSetting } from "../ModelUnloadTimeout";
import { ExperimentalToggle } from "../ExperimentalToggle";
import { KeyboardImplementationSelector } from "../debug/KeyboardImplementationSelector";
import { AccelerationSelector } from "../AccelerationSelector";
import { LazyStreamClose } from "../LazyStreamClose";

/**
 * General — app-wide basics, all visible in titled groups (no folds):
 * recording shortcuts + microphone, appearance, sounds, overlay, startup,
 * updates, and a small System group at the end. Dictation-specific rows
 * (output, custom words, model language) live on the Dictation page.
 */
export const GeneralSettings: React.FC = () => {
  const { t } = useTranslation();
  const { audioFeedbackEnabled, getSetting } = useSettings();
  const isLinux = type() === "linux";
  const experimentalEnabled = getSetting("experimental_enabled") || false;

  return (
    <div className="max-w-3xl w-full mx-auto space-y-8">
      <SectionHeader
        title={t("sidebar.general")}
        description={t("sectionSubtitles.general")}
      />
      <SettingsGroup
        title={t("settings.general.recording.title")}
        icon={Keyboard}
      >
        <ShortcutInput
          shortcutId="transcribe"
          grouped={true}
          icon={AudioLines}
          tone="teal"
        />
        {/* Cancel shortcut is hidden on Linux (dynamic shortcut instability). */}
        {!isLinux && (
          <ShortcutInput
            shortcutId="cancel"
            grouped={true}
            icon={Ban}
            tone="rose"
          />
        )}
        <PushToTalk descriptionMode="tooltip" grouped={true} />
        <MicrophoneSelector
          descriptionMode="tooltip"
          grouped={true}
          icon={Mic}
          tone="teal"
        />
      </SettingsGroup>

      <SettingsGroup title={t("appearance.title")} icon={Palette}>
        <AppearanceSelector
          descriptionMode="tooltip"
          grouped={true}
          icon={SunMoon}
          tone="amber"
        />
        <TextSizeSelector grouped={true} icon={Type} tone="sky" />
      </SettingsGroup>

      <SettingsGroup title={t("settings.general.groups.sounds")}>
        <AudioFeedback descriptionMode="tooltip" grouped={true} />
        {audioFeedbackEnabled && (
          <SoundPicker label={t("settings.sound.soundTheme.label")} />
        )}
        <OutputDeviceSelector
          descriptionMode="tooltip"
          grouped={true}
          disabled={!audioFeedbackEnabled}
        />
        <VolumeSlider disabled={!audioFeedbackEnabled} />
        <MuteWhileRecording descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settings.general.groups.overlay")}>
        <OverlayStyle descriptionMode="tooltip" grouped={true} />
        <ShowOverlay descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settings.general.groups.startup")}>
        <AutostartToggle descriptionMode="tooltip" grouped={true} />
        <StartHidden descriptionMode="tooltip" grouped={true} />
        <ShowTrayIcon descriptionMode="tooltip" grouped={true} />
        <UpdateChecksToggle descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settings.general.groups.system")}>
        <ModelUnloadTimeoutSetting descriptionMode="tooltip" grouped={true} />
        <ExperimentalToggle descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

      {experimentalEnabled && (
        <SettingsGroup title={t("settings.advanced.groups.experimental")}>
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
