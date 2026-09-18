// Settings section components
export { GeneralSettings } from "./general/GeneralSettings";
export { DebugSettings } from "./debug/DebugSettings";
export { HistorySettings } from "./history/HistorySettings";
export { MeetingsSection } from "./meetings/MeetingsSection";
export { AboutSettings } from "./about/AboutSettings";
export { ModelsSettings } from "./models/ModelsSettings";
export { DictationSettings } from "./dictation/DictationSettings";
export { AssistantSettings } from "./assistant/AssistantSettings";
export { AssistantSection } from "./assistant/AssistantSection";
export { CharactersSettings } from "./assistant/CharactersSettings";
export { MemorySettings } from "./assistant/MemorySettings";

// Individual setting components
export { MicrophoneSelector } from "./MicrophoneSelector";
export { ClamshellMicrophoneSelector } from "./ClamshellMicrophoneSelector";
export { OutputDeviceSelector } from "./OutputDeviceSelector";
export { AlwaysOnMicrophone } from "./AlwaysOnMicrophone";
export { PushToTalk } from "./PushToTalk";
export { TapToLock } from "./TapToLock";
export { AudioFeedback } from "./AudioFeedback";
export { ShowOverlay } from "./ShowOverlay";
export { GlobalShortcutInput } from "./GlobalShortcutInput";
export { HandyKeysShortcutInput } from "./HandyKeysShortcutInput";
export { ShortcutInput } from "./ShortcutInput";
export { TranslateToEnglish } from "./TranslateToEnglish";
export { CustomWords } from "./CustomWords";
export { TextReplacements } from "./TextReplacements";
export { PostProcessingSettingsApi } from "./PostProcessingSettingsApi";
export { PostProcessingSettingsPrompts } from "./PostProcessingSettingsPrompts";
export { AppDataDirectory } from "./AppDataDirectory";
export { ModelUnloadTimeoutSetting } from "./ModelUnloadTimeout";
export { StartHidden } from "./StartHidden";
// The History page owns retention now (`history/RetentionSettings.tsx`): the
// period, the count, and the day window share a preview/confirm step, so they
// can no longer be dropped in as three independent rows.
export { AutostartToggle } from "./AutostartToggle";
export { UpdateChecksToggle } from "./UpdateChecksToggle";
