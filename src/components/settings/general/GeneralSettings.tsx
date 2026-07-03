import React from "react";
import { useTranslation } from "react-i18next";
import { type } from "@tauri-apps/plugin-os";
import {
  Keyboard,
  AudioLines,
  Ban,
  Lock,
  Palette,
  SunMoon,
  Type,
  Volume2,
  Mic,
  MicOff,
  Headphones,
} from "lucide-react";
import { MicrophoneSelector } from "../MicrophoneSelector";
import { ShortcutInput } from "../ShortcutInput";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { MoreOptions } from "../../ui/MoreOptions";
import { OutputDeviceSelector } from "../OutputDeviceSelector";
import { AudioFeedback } from "../AudioFeedback";
import { AppearanceSelector } from "../AppearanceSelector";
import { TextSizeSelector } from "../TextSizeSelector";
import { TapToLock } from "../TapToLock";
import { useSettings } from "../../../hooks/useSettings";
import { VolumeSlider } from "../VolumeSlider";
import { MuteWhileRecording } from "../MuteWhileRecording";
import { ModelSettingsCard } from "./ModelSettingsCard";

export const GeneralSettings: React.FC = () => {
  const { t } = useTranslation();
  const { audioFeedbackEnabled } = useSettings();
  const isLinux = type() === "linux";
  return (
    <div className="max-w-2xl w-full mx-auto space-y-8">
      <SettingsGroup title={t("settings.general.shortcut.title")} icon={Keyboard}>
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
        <TapToLock
          descriptionMode="tooltip"
          grouped={true}
          icon={Lock}
          tone="violet"
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
      <ModelSettingsCard />
      <SettingsGroup title={t("settings.sound.title")} icon={Volume2}>
        <MicrophoneSelector
          descriptionMode="tooltip"
          grouped={true}
          icon={Mic}
          tone="teal"
        />
        <AudioFeedback
          descriptionMode="tooltip"
          grouped={true}
          icon={Volume2}
          tone="amber"
        />
        <MoreOptions>
          <MuteWhileRecording
            descriptionMode="tooltip"
            grouped={true}
            icon={MicOff}
            tone="rose"
          />
          <OutputDeviceSelector
            descriptionMode="tooltip"
            grouped={true}
            disabled={!audioFeedbackEnabled}
            icon={Headphones}
            tone="sky"
          />
          <VolumeSlider disabled={!audioFeedbackEnabled} />
        </MoreOptions>
      </SettingsGroup>
    </div>
  );
};
