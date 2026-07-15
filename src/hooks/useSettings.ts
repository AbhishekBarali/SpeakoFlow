import { useEffect } from "react";
import { useSettingsStore } from "../stores/settingsStore";
import type {
  AppSettings as Settings,
  AudioDevice,
  PostProcessReadiness,
} from "@/bindings";

interface UseSettingsReturn {
  // State
  settings: Settings | null;
  isLoading: boolean;
  isUpdating: (key: string) => boolean;
  audioDevices: AudioDevice[];
  outputDevices: AudioDevice[];
  audioFeedbackEnabled: boolean;
  postProcessModelOptions: Record<string, string[]>;
  postProcessReadiness: PostProcessReadiness | null;
  isPostProcessReadinessLoading: boolean;
  postProcessReadinessError: boolean;

  // Actions
  updateSetting: <K extends keyof Settings>(
    key: K,
    value: Settings[K],
  ) => Promise<boolean>;
  resetSetting: (key: keyof Settings) => Promise<void>;
  refreshSettings: () => Promise<void>;
  refreshPostProcessReadiness: () => Promise<void>;
  refreshAudioDevices: () => Promise<void>;
  refreshOutputDevices: () => Promise<void>;

  // Binding-specific actions
  updateBinding: (id: string, binding: string) => Promise<void>;
  resetBinding: (id: string) => Promise<void>;

  // Convenience getters
  getSetting: <K extends keyof Settings>(key: K) => Settings[K] | undefined;

  // Post-processing helpers
  setPostProcessProvider: (providerId: string) => Promise<boolean>;
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
}

export const useSettings = (): UseSettingsReturn => {
  const store = useSettingsStore();

  // Initialize on first mount
  useEffect(() => {
    if (store.isLoading) {
      store.initialize();
    }
  }, [store.initialize, store.isLoading]);

  return {
    settings: store.settings,
    isLoading: store.isLoading,
    isUpdating: store.isUpdatingKey,
    audioDevices: store.audioDevices,
    outputDevices: store.outputDevices,
    audioFeedbackEnabled: store.settings?.audio_feedback || false,
    postProcessModelOptions: store.postProcessModelOptions,
    postProcessReadiness: store.postProcessReadiness,
    isPostProcessReadinessLoading: store.isPostProcessReadinessLoading,
    postProcessReadinessError: store.postProcessReadinessError,
    updateSetting: store.updateSetting,
    resetSetting: store.resetSetting,
    refreshSettings: store.refreshSettings,
    refreshPostProcessReadiness: store.refreshPostProcessReadiness,
    refreshAudioDevices: store.refreshAudioDevices,
    refreshOutputDevices: store.refreshOutputDevices,
    updateBinding: store.updateBinding,
    resetBinding: store.resetBinding,
    getSetting: store.getSetting,
    setPostProcessProvider: store.setPostProcessProvider,
    updatePostProcessBaseUrl: store.updatePostProcessBaseUrl,
    updatePostProcessApiKey: store.updatePostProcessApiKey,
    updatePostProcessModel: store.updatePostProcessModel,
    fetchPostProcessModels: store.fetchPostProcessModels,
  };
};
