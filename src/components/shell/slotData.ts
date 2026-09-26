import { useEffect } from "react";
import { create } from "zustand";
import { commands, type CloudSttProvider } from "@/bindings";
import { useSettingsStore } from "@/stores/settingsStore";
import { useModelStore } from "@/stores/modelStore";

/**
 * Backend facts the model-slot summaries need that are not in settings: the
 * cloud speech-to-text registry and which of its providers have a key.
 *
 * These used to be fetched by every `useModelSlots()` caller on mount — the
 * sidebar, Home, each Models tab, AI cleanup, the assistant — and each of them
 * also asked for cleanup readiness, which flips a loading flag in the settings
 * store and re-renders every settings consumer twice. Opening one page fired
 * the same three requests three or four times over. Now there is one copy here,
 * kept fresh by `useSlotDataSync`, mounted once by the shell.
 */
interface SlotDataState {
  cloudProviders: CloudSttProvider[];
  cloudKeys: Record<string, boolean>;
  loaded: boolean;
  refreshCloud: () => Promise<void>;
}

let cloudGeneration = 0;

export const useSlotDataStore = create<SlotDataState>((set) => ({
  cloudProviders: [],
  cloudKeys: {},
  loaded: false,
  refreshCloud: async () => {
    const generation = ++cloudGeneration;
    try {
      const [providers, keys] = await Promise.all([
        commands.getCloudSttProviders(),
        commands.getCloudSttKeyStatus(),
      ]);
      if (generation !== cloudGeneration) return;
      set({
        cloudProviders: providers,
        cloudKeys: Object.fromEntries(keys),
        loaded: true,
      });
    } catch (error) {
      console.warn("Failed to load cloud speech providers:", error);
      if (generation === cloudGeneration) set({ loaded: true });
    }
  },
}));

/**
 * Keep the slot data and the cleanup readiness current. Mount exactly once.
 *
 * Reads narrow slices of the settings store so it re-runs only when an input
 * that can change an answer actually moves, not on every settings write.
 */
export const useSlotDataSync = (): void => {
  const refreshCloud = useSlotDataStore((state) => state.refreshCloud);
  const refreshReadiness = useSettingsStore(
    (state) => state.refreshPostProcessReadiness,
  );
  const settingsLoaded = useSettingsStore((state) => state.settings !== null);

  const cloudKey = useSettingsStore((state) => {
    const s = state.settings;
    if (!s) return "";
    return [
      s.stt_engine_mode,
      s.cloud_stt_provider_id,
      JSON.stringify(s.cloud_stt_models ?? {}),
      // Presence, not value: the key itself never needs to reach this string.
      Object.entries(s.cloud_stt_api_keys ?? {})
        .map(([id, key]) => `${id}:${key ? 1 : 0}`)
        .join(","),
    ].join("|");
  });

  const cleanupKey = useSettingsStore((state) => {
    const s = state.settings;
    if (!s) return "";
    return [
      s.post_process_enabled,
      s.post_process_provider_id,
      s.assistant_provider_id,
      s.post_process_selected_prompt_id,
      JSON.stringify(s.post_process_models ?? {}),
      JSON.stringify(s.assistant_models ?? {}),
      Object.entries(s.post_process_api_keys ?? {})
        .map(([id, key]) => `${id}:${key ? 1 : 0}`)
        .join(","),
    ].join("|");
  });
  // Readiness also checks that a local model is still on disk, so a download
  // finishing or a file being removed has to re-ask.
  const downloadedKey = useModelStore((state) =>
    state.models
      .filter((model) => model.is_downloaded)
      .map((model) => model.id)
      .join(","),
  );

  useEffect(() => {
    if (!settingsLoaded) return;
    void refreshCloud();
  }, [cloudKey, settingsLoaded, refreshCloud]);

  useEffect(() => {
    if (!settingsLoaded) return;
    void refreshReadiness();
  }, [cleanupKey, downloadedKey, settingsLoaded, refreshReadiness]);
};
