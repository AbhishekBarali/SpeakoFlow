import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import { listen } from "@tauri-apps/api/event";
import type {
  AppSettings as Settings,
  AudioDevice,
  PostProcessReadiness,
  Replacement,
  WhisperAcceleratorSetting,
  OrtAcceleratorSetting,
  PostProcessCleanupStrength,
} from "@/bindings";
import { commands } from "@/bindings";
import { toast } from "sonner";
import i18n from "@/i18n";

let settingsRefreshGeneration = 0;
let readinessRefreshGeneration = 0;
const mutationGenerations = new Map<string, number>();
const modelFetchGenerations = new Map<string, number>();
let initializePromise: Promise<void> | null = null;

const nextGeneration = (generations: Map<string, number>, key: string) => {
  const next = (generations.get(key) ?? 0) + 1;
  generations.set(key, next);
  return next;
};

const commandSucceeded = (result: unknown): void => {
  if (
    typeof result === "object" &&
    result !== null &&
    "status" in result &&
    result.status === "error"
  ) {
    const error = "error" in result ? String(result.error) : "Command failed";
    throw new Error(error);
  }
};

const isCleanupSetting = (key: keyof Settings): boolean =>
  key === "post_process_enabled" ||
  key === "post_process_selected_prompt_id" ||
  key === "post_process_tone" ||
  key === "post_process_timeout_secs";

const showCleanupSaveError = () =>
  toast.error(
    i18n.t("settings.postProcessing.errors.saveFailed", {
      defaultValue: "Couldn’t save the AI cleanup setting.",
    }),
  );

interface SettingsStore {
  settings: Settings | null;
  defaultSettings: Settings | null;
  isLoading: boolean;
  isUpdating: Record<string, boolean>;
  audioDevices: AudioDevice[];
  outputDevices: AudioDevice[];
  customSounds: { start: boolean; stop: boolean };
  postProcessModelOptions: Record<string, string[]>;
  postProcessReadiness: PostProcessReadiness | null;
  isPostProcessReadinessLoading: boolean;
  postProcessReadinessError: boolean;

  // Actions
  initialize: () => Promise<void>;
  loadDefaultSettings: () => Promise<void>;
  updateSetting: <K extends keyof Settings>(
    key: K,
    value: Settings[K],
  ) => Promise<boolean>;
  resetSetting: (key: keyof Settings) => Promise<void>;
  refreshSettings: () => Promise<void>;
  refreshPostProcessReadiness: () => Promise<void>;
  refreshAudioDevices: () => Promise<void>;
  refreshOutputDevices: () => Promise<void>;
  updateBinding: (id: string, binding: string) => Promise<void>;
  resetBinding: (id: string) => Promise<void>;
  getSetting: <K extends keyof Settings>(key: K) => Settings[K] | undefined;
  isUpdatingKey: (key: string) => boolean;
  playTestSound: (soundType: "start" | "stop") => Promise<void>;
  checkCustomSounds: () => Promise<void>;
  setPostProcessProvider: (providerId: string) => Promise<boolean>;
  updatePostProcessSetting: (
    settingType: "base_url" | "api_key" | "model",
    providerId: string,
    value: string,
  ) => Promise<boolean>;
  updatePostProcessBaseUrl: (
    providerId: string,
    baseUrl: string,
  ) => Promise<boolean>;
  updatePostProcessApiKey: (
    providerId: string,
    apiKey: string,
  ) => Promise<boolean>;
  updatePostProcessModel: (
    providerId: string,
    model: string,
  ) => Promise<boolean>;
  fetchPostProcessModels: (providerId: string) => Promise<string[] | null>;
  setPostProcessModelOptions: (providerId: string, models: string[]) => void;

  // Internal state setters
  setSettings: (settings: Settings | null) => void;
  setDefaultSettings: (defaultSettings: Settings | null) => void;
  setLoading: (loading: boolean) => void;
  setUpdating: (key: string, updating: boolean) => void;
  setAudioDevices: (devices: AudioDevice[]) => void;
  setOutputDevices: (devices: AudioDevice[]) => void;
  setCustomSounds: (sounds: { start: boolean; stop: boolean }) => void;
}

// Note: Default settings are now fetched from Rust via commands.getDefaultSettings()
// This ensures platform-specific defaults (like overlay_position, shortcuts, paste_method) work correctly

const DEFAULT_AUDIO_DEVICE: AudioDevice = {
  index: "default",
  name: "Default",
  is_default: true,
};

const settingUpdaters: {
  [K in keyof Settings]?: (value: Settings[K]) => Promise<unknown>;
} = {
  always_on_microphone: (value) =>
    commands.updateMicrophoneMode(value as boolean),
  audio_feedback: (value) =>
    commands.changeAudioFeedbackSetting(value as boolean),
  audio_feedback_volume: (value) =>
    commands.changeAudioFeedbackVolumeSetting(value as number),
  sound_theme: (value) => commands.changeSoundThemeSetting(value as string),
  theme: (value) => commands.changeThemeSetting(value as string),
  ui_text_size: (value) => commands.changeUiTextSizeSetting(value as string),
  start_hidden: (value) => commands.changeStartHiddenSetting(value as boolean),
  autostart_enabled: (value) =>
    commands.changeAutostartSetting(value as boolean),
  update_checks_enabled: (value) =>
    commands.changeUpdateChecksSetting(value as boolean),
  push_to_talk: (value) => commands.changePttSetting(value as boolean),
  tap_to_lock: (value) => commands.changeTapToLockSetting(value as boolean),
  tap_to_lock_key: (value) =>
    commands.changeTapToLockKeySetting(value as string),
  assistant_tap_to_lock_key: (value) =>
    commands.changeAssistantTapToLockKeySetting(value as string),
  selected_microphone: (value) =>
    commands.setSelectedMicrophone(
      (value as string) === "Default" || value === null
        ? "default"
        : (value as string),
    ),
  clamshell_microphone: (value) =>
    commands.setClamshellMicrophone(
      (value as string) === "Default" ? "default" : (value as string),
    ),
  selected_output_device: (value) =>
    commands.setSelectedOutputDevice(
      (value as string) === "Default" || value === null
        ? "default"
        : (value as string),
    ),
  recording_retention_period: (value) =>
    commands.updateRecordingRetentionPeriod(value as string),
  translate_to_english: (value) =>
    commands.changeTranslateToEnglishSetting(value as boolean),
  selected_language: (value) =>
    commands.changeSelectedLanguageSetting(value as string),
  overlay_position: (value) =>
    commands.changeOverlayPositionSetting(value as string),
  overlay_style: (value) => commands.changeOverlayStyleSetting(value as string),
  assistant_overlay_style: (value) =>
    commands.changeAssistantOverlayStyleSetting(value as string),
  debug_mode: (value) => commands.changeDebugModeSetting(value as boolean),
  custom_words: (value) => commands.updateCustomWords(value as string[]),
  spoken_emojis_enabled: (value) =>
    commands.changeSpokenEmojisEnabledSetting(value as boolean),
  replacements_enabled: (value) =>
    commands.changeReplacementsEnabledSetting(value as boolean),
  text_replacements: (value) =>
    commands.updateTextReplacements(value as Replacement[]),
  word_correction_threshold: (value) =>
    commands.changeWordCorrectionThresholdSetting(value as number),
  paste_delay_ms: (value) =>
    commands.changePasteDelayMsSetting(value as number),
  paste_method: (value) => commands.changePasteMethodSetting(value as string),
  typing_tool: (value) => commands.changeTypingToolSetting(value as string),
  external_script_path: (value) =>
    commands.changeExternalScriptPathSetting(value as string | null),
  clipboard_handling: (value) =>
    commands.changeClipboardHandlingSetting(value as string),
  auto_submit: (value) => commands.changeAutoSubmitSetting(value as boolean),
  auto_submit_key: (value) =>
    commands.changeAutoSubmitKeySetting(value as string),
  history_limit: (value) => commands.updateHistoryLimit(value as number),
  post_process_enabled: (value) =>
    commands.changePostProcessEnabledSetting(value as boolean),
  post_process_fix_misheard: (value) =>
    commands.changePostProcessFixMisheardSetting(value as boolean),
  post_process_cleanup_strength: (value) =>
    commands.changePostProcessCleanupStrengthSetting(
      value as PostProcessCleanupStrength,
    ),
  flow_enabled: (value) => commands.changeFlowEnabledSetting(value as boolean),
  flow_phrase: (value) => commands.changeFlowPhraseSetting(value as string),
  flow_screen_access: (value) =>
    commands.changeFlowScreenAccessSetting(value as boolean),
  post_process_selected_prompt_id: (value) =>
    commands.setPostProcessSelectedPrompt(value as string),
  post_process_tone: (value) =>
    commands.changePostProcessToneSetting(value as string),
  post_process_timeout_secs: (value) =>
    commands.changePostProcessTimeoutSetting(value as number),
  mute_while_recording: (value) =>
    commands.changeMuteWhileRecordingSetting(value as boolean),
  append_trailing_space: (value) =>
    commands.changeAppendTrailingSpaceSetting(value as boolean),
  log_level: (value) => commands.setLogLevel(value as any),
  app_language: (value) => commands.changeAppLanguageSetting(value as string),
  experimental_enabled: (value) =>
    commands.changeExperimentalEnabledSetting(value as boolean),
  lazy_stream_close: (value) =>
    commands.changeLazyStreamCloseSetting(value as boolean),
  live_transcription_enabled: (value) =>
    commands.changeLiveTranscriptionEnabledSetting(value as boolean),
  live_transcription_window_enabled: (value) =>
    commands.changeLiveTranscriptionWindowEnabledSetting(value as boolean),
  show_tray_icon: (value) =>
    commands.changeShowTrayIconSetting(value as boolean),
  close_behavior: (value) =>
    commands.changeCloseBehaviorSetting(value as string),
  whisper_accelerator: (value) =>
    commands.changeWhisperAcceleratorSetting(
      value as WhisperAcceleratorSetting,
    ),
  ort_accelerator: (value) =>
    commands.changeOrtAcceleratorSetting(value as OrtAcceleratorSetting),
  whisper_gpu_device: (value) =>
    commands.changeWhisperGpuDevice(value as number),
  extra_recording_buffer_ms: (value) =>
    commands.changeExtraRecordingBufferSetting(value as number),
};

export const useSettingsStore = create<SettingsStore>()(
  subscribeWithSelector((set, get) => ({
    settings: null,
    defaultSettings: null,
    isLoading: true,
    isUpdating: {},
    audioDevices: [],
    outputDevices: [],
    customSounds: { start: false, stop: false },
    postProcessModelOptions: {},
    postProcessReadiness: null,
    isPostProcessReadinessLoading: false,
    postProcessReadinessError: false,

    // Internal setters
    setSettings: (settings) => set({ settings }),
    setDefaultSettings: (defaultSettings) => set({ defaultSettings }),
    setLoading: (isLoading) => set({ isLoading }),
    setUpdating: (key, updating) =>
      set((state) => ({
        isUpdating: { ...state.isUpdating, [key]: updating },
      })),
    setAudioDevices: (audioDevices) => set({ audioDevices }),
    setOutputDevices: (outputDevices) => set({ outputDevices }),
    setCustomSounds: (customSounds) => set({ customSounds }),

    // Getters
    getSetting: (key) => get().settings?.[key],
    isUpdatingKey: (key) => get().isUpdating[key] || false,

    // Load settings from store
    refreshSettings: async () => {
      const generation = ++settingsRefreshGeneration;
      try {
        const result = await commands.getAppSettings();
        // A newer mutation/refresh started while this read was in flight; drop
        // this stale response so it can't overwrite a fresher optimistic value.
        if (generation !== settingsRefreshGeneration) return;
        if (result.status === "ok") {
          const settings = result.data;
          const normalizedSettings: Settings = {
            ...settings,
            always_on_microphone: settings.always_on_microphone ?? false,
            selected_microphone: settings.selected_microphone ?? "Default",
            clamshell_microphone: settings.clamshell_microphone ?? "Default",
            selected_output_device:
              settings.selected_output_device ?? "Default",
          };
          set({ settings: normalizedSettings, isLoading: false });
        } else {
          console.error("Failed to load settings:", result.error);
          set({ isLoading: false });
        }
      } catch (error) {
        console.error("Failed to load settings:", error);
        if (generation === settingsRefreshGeneration) set({ isLoading: false });
      }
    },

    // Read the backend's single cleanup readiness resolver so the UI never
    // recomputes "ready" rules in TypeScript.
    refreshPostProcessReadiness: async () => {
      const generation = ++readinessRefreshGeneration;
      set({
        isPostProcessReadinessLoading: true,
        postProcessReadinessError: false,
      });
      try {
        const readiness = await commands.getPostProcessReadiness();
        if (generation !== readinessRefreshGeneration) return;
        set({
          postProcessReadiness: readiness,
          isPostProcessReadinessLoading: false,
          postProcessReadinessError: false,
        });
      } catch (error) {
        console.error("Failed to load AI cleanup readiness:", error);
        if (generation === readinessRefreshGeneration) {
          set({
            isPostProcessReadinessLoading: false,
            postProcessReadinessError: true,
          });
        }
      }
    },

    // Load audio devices
    refreshAudioDevices: async () => {
      try {
        const result = await commands.getAvailableMicrophones();
        if (result.status === "ok") {
          const devicesWithDefault = [
            DEFAULT_AUDIO_DEVICE,
            ...result.data.filter(
              (d) => d.name !== "Default" && d.name !== "default",
            ),
          ];
          set({ audioDevices: devicesWithDefault });
        } else {
          set({ audioDevices: [DEFAULT_AUDIO_DEVICE] });
        }
      } catch (error) {
        console.error("Failed to load audio devices:", error);
        set({ audioDevices: [DEFAULT_AUDIO_DEVICE] });
      }
    },

    // Load output devices
    refreshOutputDevices: async () => {
      try {
        const result = await commands.getAvailableOutputDevices();
        if (result.status === "ok") {
          const devicesWithDefault = [
            DEFAULT_AUDIO_DEVICE,
            ...result.data.filter(
              (d) => d.name !== "Default" && d.name !== "default",
            ),
          ];
          set({ outputDevices: devicesWithDefault });
        } else {
          set({ outputDevices: [DEFAULT_AUDIO_DEVICE] });
        }
      } catch (error) {
        console.error("Failed to load output devices:", error);
        set({ outputDevices: [DEFAULT_AUDIO_DEVICE] });
      }
    },

    // Play a test sound
    playTestSound: async (soundType: "start" | "stop") => {
      try {
        await commands.playTestSound(soundType);
      } catch (error) {
        console.error(`Failed to play test sound (${soundType}):`, error);
      }
    },

    checkCustomSounds: async () => {
      try {
        const sounds = await commands.checkCustomSounds();
        get().setCustomSounds(sounds);
      } catch (error) {
        console.error("Failed to check custom sounds:", error);
      }
    },

    // Update a specific setting
    updateSetting: async <K extends keyof Settings>(
      key: K,
      value: Settings[K],
    ) => {
      const { settings, setUpdating } = get();
      const updateKey = String(key);
      const originalValue = settings?.[key];
      const cleanup = isCleanupSetting(key);
      const generation = cleanup
        ? nextGeneration(mutationGenerations, updateKey)
        : 0;

      setUpdating(updateKey, true);

      try {
        set((state) => ({
          settings: state.settings ? { ...state.settings, [key]: value } : null,
        }));

        const updater = settingUpdaters[key];
        if (updater) {
          // Generated commands resolve `{status:"error"}` for non-Error
          // rejections instead of throwing, so inspect the discriminant too.
          commandSucceeded(await updater(value));
        } else if (key !== "bindings" && key !== "selected_model") {
          console.warn(`No handler for setting: ${String(key)}`);
        }

        if (cleanup) {
          void get().refreshPostProcessReadiness();
        }
        return true;
      } catch (error) {
        console.error(`Failed to update setting ${String(key)}:`, error);
        // Roll back only this field on the CURRENT state, and only if a newer
        // mutation for the same key hasn't superseded this one.
        if (!cleanup || generation === mutationGenerations.get(updateKey)) {
          set((state) =>
            state.settings
              ? { settings: { ...state.settings, [key]: originalValue } }
              : { settings: state.settings },
          );
        }
        if (cleanup) {
          showCleanupSaveError();
          void get().refreshSettings();
          void get().refreshPostProcessReadiness();
        }
        return false;
      } finally {
        if (!cleanup || generation === mutationGenerations.get(updateKey)) {
          setUpdating(updateKey, false);
        }
      }
    },

    // Reset a setting to its default value
    resetSetting: async (key) => {
      const { defaultSettings } = get();
      if (defaultSettings) {
        const defaultValue = defaultSettings[key];
        if (defaultValue !== undefined) {
          await get().updateSetting(key, defaultValue as any);
        }
      }
    },

    // Update a specific binding
    updateBinding: async (id, binding) => {
      const { settings, setUpdating } = get();
      const updateKey = `binding_${id}`;
      const originalBinding = settings?.bindings?.[id]?.current_binding;

      setUpdating(updateKey, true);

      try {
        // Optimistic update
        set((state) => ({
          settings: state.settings
            ? {
                ...state.settings,
                bindings: {
                  ...state.settings.bindings,
                  [id]: {
                    ...state.settings.bindings[id]!,
                    current_binding: binding,
                  },
                },
              }
            : null,
        }));

        const result = await commands.changeBinding(id, binding);

        // Check if the command executed successfully
        if (result.status === "error") {
          throw new Error(result.error);
        }

        // Check if the binding change was successful
        if (!result.data.success) {
          throw new Error(result.data.error || "Failed to update binding");
        }
      } catch (error) {
        console.error(`Failed to update binding ${id}:`, error);

        // Rollback on error
        if (originalBinding && get().settings) {
          set((state) => ({
            settings: state.settings
              ? {
                  ...state.settings,
                  bindings: {
                    ...state.settings.bindings,
                    [id]: {
                      ...state.settings.bindings[id]!,
                      current_binding: originalBinding,
                    },
                  },
                }
              : null,
          }));
        }

        // Re-throw to let the caller know it failed
        throw error;
      } finally {
        setUpdating(updateKey, false);
      }
    },

    // Reset a specific binding
    resetBinding: async (id) => {
      const { setUpdating, refreshSettings } = get();
      const updateKey = `binding_${id}`;

      setUpdating(updateKey, true);

      try {
        await commands.resetBinding(id);
        await refreshSettings();
      } catch (error) {
        console.error(`Failed to reset binding ${id}:`, error);
      } finally {
        setUpdating(updateKey, false);
      }
    },

    setPostProcessProvider: async (providerId) => {
      const {
        settings,
        setUpdating,
        refreshSettings,
        setPostProcessModelOptions,
      } = get();
      const updateKey = "post_process_provider_id";
      const previousId = settings?.post_process_provider_id ?? null;
      const fetchGeneration = nextGeneration(modelFetchGenerations, providerId);

      setUpdating(updateKey, true);

      if (settings) {
        set((state) => ({
          settings: state.settings
            ? { ...state.settings, post_process_provider_id: providerId }
            : null,
        }));
      }

      // Clear cached model options for the new provider so the dropdown
      // doesn't show stale models from a previous fetch or base_url.
      setPostProcessModelOptions(providerId, []);

      try {
        commandSucceeded(await commands.setPostProcessProvider(providerId));
        await refreshSettings();
        void get().refreshPostProcessReadiness();
        // Preserve latest-wins semantics for any concurrent model fetch.
        void fetchGeneration;
        return true;
      } catch (error) {
        console.error("Failed to set post-process provider:", error);
        if (previousId !== null) {
          set((state) => ({
            settings: state.settings
              ? { ...state.settings, post_process_provider_id: previousId }
              : null,
          }));
        }
        showCleanupSaveError();
        void refreshSettings();
        void get().refreshPostProcessReadiness();
        return false;
      } finally {
        setUpdating(updateKey, false);
      }
    },

    // Generic updater for post-processing provider settings
    updatePostProcessSetting: async (
      settingType: "base_url" | "api_key" | "model",
      providerId: string,
      value: string,
    ) => {
      const { setUpdating, refreshSettings } = get();
      const updateKey = `post_process_${settingType}:${providerId}`;

      setUpdating(updateKey, true);

      try {
        if (settingType === "base_url") {
          commandSucceeded(
            await commands.changePostProcessBaseUrlSetting(providerId, value),
          );
        } else if (settingType === "api_key") {
          commandSucceeded(
            await commands.changePostProcessApiKeySetting(providerId, value),
          );
        } else if (settingType === "model") {
          commandSucceeded(
            await commands.changePostProcessModelSetting(providerId, value),
          );
        }
        await refreshSettings();
        void get().refreshPostProcessReadiness();
        return true;
      } catch (error) {
        console.error(
          `Failed to update post-process ${settingType.replace("_", " ")}:`,
          error,
        );
        showCleanupSaveError();
        void refreshSettings();
        void get().refreshPostProcessReadiness();
        return false;
      } finally {
        setUpdating(updateKey, false);
      }
    },

    updatePostProcessBaseUrl: async (providerId, baseUrl) => {
      const { setUpdating, refreshSettings } = get();
      const updateKey = `post_process_base_url:${providerId}`;
      // A new endpoint invalidates any in-flight model list for this provider.
      const fetchGeneration = nextGeneration(modelFetchGenerations, providerId);

      setUpdating(updateKey, true);

      // Optimistically clear the provider's model + cached options and mark
      // readiness as checking so a stale "Ready" state disappears at once.
      set((state) => ({
        settings: state.settings
          ? {
              ...state.settings,
              post_process_models: {
                ...state.settings.post_process_models,
                [providerId]: "",
              },
            }
          : state.settings,
        postProcessModelOptions: {
          ...state.postProcessModelOptions,
          [providerId]: [],
        },
        isPostProcessReadinessLoading: true,
      }));

      try {
        // The backend now clears the provider's cleanup model in the SAME
        // write as the base URL, so this is a single atomic mutation.
        commandSucceeded(
          await commands.changePostProcessBaseUrlSetting(providerId, baseUrl),
        );
        await refreshSettings();
        void get().refreshPostProcessReadiness();
        void fetchGeneration;
        return true;
      } catch (error) {
        console.error("Failed to update post-process base URL:", error);
        showCleanupSaveError();
        void refreshSettings();
        void get().refreshPostProcessReadiness();
        return false;
      } finally {
        setUpdating(updateKey, false);
      }
    },

    updatePostProcessApiKey: async (providerId, apiKey) => {
      // Clear cached models when API key changes - user should click refresh after
      nextGeneration(modelFetchGenerations, providerId);
      set((state) => ({
        postProcessModelOptions: {
          ...state.postProcessModelOptions,
          [providerId]: [],
        },
      }));
      return get().updatePostProcessSetting("api_key", providerId, apiKey);
    },

    updatePostProcessModel: async (providerId, model) => {
      return get().updatePostProcessSetting("model", providerId, model);
    },

    fetchPostProcessModels: async (providerId) => {
      const updateKey = `post_process_models_fetch:${providerId}`;
      const { setUpdating, setPostProcessModelOptions } = get();
      const generation = nextGeneration(modelFetchGenerations, providerId);

      setUpdating(updateKey, true);

      try {
        const result = await commands.fetchPostProcessModels(providerId);
        // A newer endpoint/key change (or provider switch) superseded this
        // fetch; drop the stale list so old options can't repopulate.
        if (generation !== modelFetchGenerations.get(providerId)) {
          return null;
        }
        if (result.status === "ok") {
          setPostProcessModelOptions(providerId, result.data);
          return result.data;
        } else {
          console.error("Failed to fetch models:", result.error);
          return null;
        }
      } catch (error) {
        console.error("Failed to fetch models:", error);
        // Distinguish failure (null) from a legitimate empty list.
        return null;
      } finally {
        if (generation === modelFetchGenerations.get(providerId)) {
          setUpdating(updateKey, false);
        }
      }
    },

    setPostProcessModelOptions: (providerId, models) =>
      set((state) => ({
        postProcessModelOptions: {
          ...state.postProcessModelOptions,
          [providerId]: models,
        },
      })),

    // Load default settings from Rust
    loadDefaultSettings: async () => {
      try {
        const result = await commands.getDefaultSettings();
        if (result.status === "ok") {
          set({ defaultSettings: result.data });
        } else {
          console.error("Failed to load default settings:", result.error);
        }
      } catch (error) {
        console.error("Failed to load default settings:", error);
      }
    },

    // Initialize everything
    initialize: async () => {
      // Many components call useSettings(); guard so multiple mounts can't
      // start duplicate initialization/listeners while isLoading is still true.
      if (initializePromise) return initializePromise;

      const run = async () => {
        const { refreshSettings, checkCustomSounds, loadDefaultSettings } =
          get();

        // Note: Audio devices are NOT refreshed here. The frontend (App.tsx)
        // is responsible for calling refreshAudioDevices/refreshOutputDevices
        // after onboarding completes. This avoids triggering permission dialogs
        // on macOS before the user is ready.
        await Promise.all([
          loadDefaultSettings(),
          refreshSettings(),
          checkCustomSounds(),
        ]);

        // Re-fetch settings when the backend changes them (e.g. language
        // reset during model switch). The backend is the source of truth.
        listen("model-state-changed", () => {
          get().refreshSettings();
        });
      };

      initializePromise = run();
      return initializePromise;
    },
  })),
);
