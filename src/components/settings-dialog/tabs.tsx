import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { type } from "@tauri-apps/plugin-os";
import { getVersion } from "@tauri-apps/api/app";
import { emit } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import {
  AudioLines,
  Ban,
  ExternalLink,
  FolderOpen,
  Github,
  MessageCircle,
  Mic,
  PhoneCall,
  Scale,
  SunMoon,
  Type as TypeIcon,
  Wand2,
} from "lucide-react";
import { commands } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { Button } from "@/components/ui/Button";
import Badge from "@/components/ui/Badge";
import { useNavigation } from "@/components/shell/navigation";
import { AppearanceSelector } from "@/components/settings/AppearanceSelector";
import { TextSizeSelector } from "@/components/settings/TextSizeSelector";
import { AppLanguageSelector } from "@/components/settings/AppLanguageSelector";
import { AutostartToggle } from "@/components/settings/AutostartToggle";
import { StartHidden } from "@/components/settings/StartHidden";
import { ShowTrayIcon } from "@/components/settings/ShowTrayIcon";
import { QuitOnClose } from "@/components/settings/QuitOnClose";
import { UpdateChecksToggle } from "@/components/settings/UpdateChecksToggle";
import { ShortcutInput } from "@/components/settings/ShortcutInput";
import { PushToTalk } from "@/components/settings/PushToTalk";
import { MicrophoneSelector } from "@/components/settings/MicrophoneSelector";
import { AlwaysOnMicrophone } from "@/components/settings/AlwaysOnMicrophone";
import { MuteWhileRecording } from "@/components/settings/MuteWhileRecording";
import { AudioFeedback } from "@/components/settings/AudioFeedback";
import { SoundPicker } from "@/components/settings/SoundPicker";
import { OutputDeviceSelector } from "@/components/settings/OutputDeviceSelector";
import { VolumeSlider } from "@/components/settings/VolumeSlider";
import {
  OverlayStyle,
  useResolvedOverlayStyle,
} from "@/components/settings/OverlayStyle";
import { OverlayLinger } from "@/components/settings/OverlayLinger";
import { ShowOverlay } from "@/components/settings/ShowOverlay";
import { PasteMethodSetting } from "@/components/settings/PasteMethod";
import { TypingToolSetting } from "@/components/settings/TypingTool";
import { ClipboardHandlingSetting } from "@/components/settings/ClipboardHandling";
import { AppendTrailingSpace } from "@/components/settings/AppendTrailingSpace";
import { AutoSubmit } from "@/components/settings/AutoSubmit";
import { SpokenEmojiToggle } from "@/components/settings/dictation/SpokenEmojiToggle";
import { GenerateWithFlowGroup } from "@/components/settings/dictation/GenerateWithFlowGroup";
import { RetentionSettings } from "@/components/settings/history/RetentionSettings";
import { AppDataDirectory } from "@/components/settings/AppDataDirectory";
import { LogDirectory } from "@/components/settings/debug";
import { ModelUnloadTimeoutSetting } from "@/components/settings/ModelUnloadTimeout";
import { ExperimentalToggle } from "@/components/settings/ExperimentalToggle";
import { KeyboardImplementationSelector } from "@/components/settings/debug/KeyboardImplementationSelector";
import { AccelerationSelector } from "@/components/settings/AccelerationSelector";
import { LazyStreamClose } from "@/components/settings/LazyStreamClose";
import { LogLevelSelector } from "@/components/settings/debug/LogLevelSelector";
import { WordCorrectionThreshold } from "@/components/settings/debug/WordCorrectionThreshold";
import { PasteDelay } from "@/components/settings/debug/PasteDelay";
import { RecordingBuffer } from "@/components/settings/debug/RecordingBuffer";
import { ClamshellMicrophoneSelector } from "@/components/settings/ClamshellMicrophoneSelector";

/**
 * The contents of each Settings tab. These are the preferences you set once and
 * forget — where the old app put all of them one click from the top. Features
 * (AI cleanup, the assistant, dictionary) and the models behind them live on
 * their own pages in the sidebar; nothing here should be needed to get started.
 */

export const GeneralTab: React.FC = () => {
  const { t } = useTranslation();
  return (
    <>
      <SettingsGroup title={t("settingsDialog.general.appearance")}>
        <AppearanceSelector
          descriptionMode="tooltip"
          grouped={true}
          icon={SunMoon}
          tone="amber"
        />
        <TextSizeSelector grouped={true} icon={TypeIcon} tone="sky" />
        <AppLanguageSelector descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settings.general.groups.startup")}>
        <AutostartToggle descriptionMode="tooltip" grouped={true} />
        <StartHidden descriptionMode="tooltip" grouped={true} />
        <ShowTrayIcon descriptionMode="tooltip" grouped={true} />
        <QuitOnClose descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
    </>
  );
};

/** A shortcut that only exists while its feature is on, with a way to turn it on. */
const UnavailableShortcut: React.FC<{
  title: string;
  hint: string;
  action: string;
  onAction: () => void;
}> = ({ title, hint, action, onAction }) => (
  <SettingContainer
    title={title}
    description={hint}
    descriptionMode="inline"
    grouped={true}
  >
    <Button variant="secondary" size="sm" onClick={onAction}>
      {action}
    </Button>
  </SettingContainer>
);

export const ShortcutsTab: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const { navigate, closeSettings } = useNavigation();
  const isLinux = type() === "linux";
  const cleanupEnabled = getSetting("post_process_enabled") ?? false;
  const assistantEnabled = getSetting("assistant_enabled") ?? true;

  const goTo = (page: "cleanup" | "assistant") => {
    closeSettings();
    navigate(page);
  };

  return (
    <>
      <SettingsGroup title={t("settingsDialog.shortcuts.dictation")}>
        <ShortcutInput
          shortcutId="transcribe"
          grouped={true}
          icon={AudioLines}
          tone="teal"
        />
        {cleanupEnabled ? (
          <ShortcutInput
            shortcutId="transcribe_with_post_process"
            grouped={true}
            icon={Wand2}
            tone="violet"
          />
        ) : (
          <UnavailableShortcut
            title={t(
              "settings.general.shortcut.bindings.transcribe_with_post_process.name",
            )}
            hint={t("settingsDialog.shortcuts.cleanupOff")}
            action={t("settingsDialog.shortcuts.openCleanup")}
            onAction={() => goTo("cleanup")}
          />
        )}
        {/* Cancel is hidden on Linux (dynamic shortcut instability). */}
        {!isLinux && (
          <ShortcutInput
            shortcutId="cancel"
            grouped={true}
            icon={Ban}
            tone="rose"
          />
        )}
        <PushToTalk descriptionMode="inline" grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settingsDialog.shortcuts.assistant")}>
        {assistantEnabled ? (
          <>
            <ShortcutInput
              shortcutId="assistant"
              grouped={true}
              icon={MessageCircle}
              tone="sky"
            />
            <ShortcutInput
              shortcutId="assistant_call"
              grouped={true}
              icon={PhoneCall}
              tone="indigo"
            />
          </>
        ) : (
          <UnavailableShortcut
            title={t("settings.general.shortcut.bindings.assistant.name")}
            hint={t("settingsDialog.shortcuts.assistantOff")}
            action={t("settingsDialog.shortcuts.openAssistant")}
            onAction={() => goTo("assistant")}
          />
        )}
      </SettingsGroup>
    </>
  );
};

export const AudioTab: React.FC = () => {
  const { t } = useTranslation();
  const { audioFeedbackEnabled } = useSettings();
  // Only the Live card lingers after a dictation, so its linger is only a
  // choice while Live is the overlay in use.
  const liveOverlay = useResolvedOverlayStyle() === "live";
  return (
    <>
      <SettingsGroup title={t("settingsDialog.audio.microphone")}>
        <MicrophoneSelector
          descriptionMode="tooltip"
          grouped={true}
          icon={Mic}
          tone="teal"
        />
        <AlwaysOnMicrophone descriptionMode="tooltip" grouped={true} />
        <MuteWhileRecording descriptionMode="tooltip" grouped={true} />
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
      </SettingsGroup>

      <SettingsGroup
        title={t("settings.general.groups.overlay")}
        description={t("settingsDialog.audio.overlayHint")}
      >
        <OverlayStyle descriptionMode="tooltip" grouped={true} />
        {liveOverlay && (
          <OverlayLinger descriptionMode="tooltip" grouped={true} />
        )}
        <ShowOverlay descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
    </>
  );
};

export const DictationTab: React.FC = () => {
  const { t } = useTranslation();
  return (
    <>
      <SettingsGroup title={t("settingsDialog.dictation.output")}>
        <PasteMethodSetting grouped={true} />
        <TypingToolSetting grouped={true} />
        <ClipboardHandlingSetting grouped={true} />
        <AppendTrailingSpace grouped={true} />
        <AutoSubmit grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settingsDialog.dictation.extras")}>
        <SpokenEmojiToggle grouped={true} />
      </SettingsGroup>

      {/* Tucked at the end on purpose: this feature is being folded into the
          assistant, so it keeps working but is no longer something to set up. */}
      <GenerateWithFlowGroup />
    </>
  );
};

export const PrivacyTab: React.FC = () => {
  const { t } = useTranslation();

  const openRecordings = async () => {
    try {
      const result = await commands.openRecordingsFolder();
      if (result.status !== "ok") throw new Error(result.error);
    } catch (error) {
      console.error("Failed to open recordings folder:", error);
      toast.error(t("settings.history.openFolderError"));
    }
  };

  return (
    <>
      <SettingsGroup
        title={t("settings.history.storage.title")}
        description={t("settings.history.storage.description")}
      >
        <RetentionSettings grouped={true} />
        <SettingContainer
          title={t("settingsDialog.privacy.recordingsFolder")}
          grouped={true}
        >
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void openRecordings()}
          >
            <FolderOpen className="h-3.5 w-3.5" aria-hidden="true" />
            {t("settings.history.openFolder")}
          </Button>
        </SettingContainer>
      </SettingsGroup>

      <SettingsGroup title={t("settings.about.folders.title")}>
        <AppDataDirectory descriptionMode="tooltip" grouped={true} />
        <LogDirectory grouped={true} />
      </SettingsGroup>
    </>
  );
};

export const AdvancedTab: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const experimentalEnabled = getSetting("experimental_enabled") || false;
  return (
    <>
      <SettingsGroup title={t("settingsDialog.advanced.performance")}>
        <ModelUnloadTimeoutSetting descriptionMode="tooltip" grouped={true} />
        <ExperimentalToggle descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

      {experimentalEnabled && (
        <SettingsGroup
          title={t("settings.advanced.groups.experimental")}
          action={<Badge variant="outline">{t("common.beta")}</Badge>}
        >
          <KeyboardImplementationSelector
            descriptionMode="tooltip"
            grouped={true}
          />
          <AccelerationSelector descriptionMode="tooltip" grouped={true} />
          <LazyStreamClose descriptionMode="tooltip" grouped={true} />
        </SettingsGroup>
      )}
    </>
  );
};

const REPO_URL = "https://github.com/AbhishekBarali/SpeakoFlow";

export const AboutTab: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const [version, setVersion] = useState("");
  const updateChecksEnabled =
    (getSetting("update_checks_enabled") as boolean | undefined) ?? true;

  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => setVersion(""));
  }, []);

  return (
    <>
      <div className="flex items-center gap-4 rounded-xl border border-hairline bg-surface px-5 py-4 elev-card">
        <img
          src="/favicon.svg"
          alt=""
          aria-hidden="true"
          className="h-11 w-11 shrink-0 rounded-xl"
        />
        <div className="min-w-0 flex-1">
          <p className="font-display text-lg leading-tight text-ink">
            {t("settingsDialog.about.appName")}
          </p>
          <p className="mt-0.5 text-sm text-muted">
            {version
              ? t("settingsDialog.about.version", { version })
              : t("common.loading")}
          </p>
        </div>
        <Button
          variant="secondary"
          size="sm"
          disabled={!updateChecksEnabled}
          onClick={() => void emit("check-for-updates")}
        >
          {t("settings.about.updates.button")}
        </Button>
      </div>

      <SettingsGroup title={t("settings.general.groups.updates")}>
        <UpdateChecksToggle descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settingsDialog.about.project")}>
        <SettingContainer
          title={t("settings.about.sourceCode.title")}
          description={t("settings.about.sourceCode.description")}
          descriptionMode="inline"
          grouped={true}
        >
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void openUrl(REPO_URL)}
          >
            <Github className="h-3.5 w-3.5" aria-hidden="true" />
            {t("settings.about.sourceCode.button")}
          </Button>
        </SettingContainer>
        <SettingContainer
          title={t("settings.about.license.title")}
          description={t("settings.about.license.description")}
          descriptionMode="inline"
          grouped={true}
        >
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void openUrl(`${REPO_URL}/blob/main/LICENSE`)}
          >
            <Scale className="h-3.5 w-3.5" aria-hidden="true" />
            {t("settings.about.license.button")}
          </Button>
        </SettingContainer>
        <SettingContainer title={t("settingsDialog.about.docs")} grouped={true}>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void openUrl("https://www.speakoflow.com/docs")}
          >
            {t("settingsDialog.about.openDocs")}
            <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
          </Button>
        </SettingContainer>
      </SettingsGroup>
    </>
  );
};

export const DebugTab: React.FC = () => {
  const { t } = useTranslation();
  return (
    <>
      <SettingsGroup>
        <LogLevelSelector grouped={true} />
        <SoundPicker
          label={t("settings.debug.soundTheme.label")}
          description={t("settings.debug.soundTheme.description")}
        />
        <WordCorrectionThreshold descriptionMode="tooltip" grouped={true} />
        <PasteDelay descriptionMode="tooltip" grouped={true} />
        <RecordingBuffer descriptionMode="tooltip" grouped={true} />
        <ClamshellMicrophoneSelector descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
    </>
  );
};
