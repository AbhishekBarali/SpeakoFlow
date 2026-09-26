import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/bindings";
import type { ModelStateEvent } from "@/lib/types/events";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { useModelStore } from "@/stores/modelStore";

/**
 * Load state of the on-device speech model, for the sidebar's status line.
 *
 * Lifted out of the old footer `ModelSelector`, which owned it together with a
 * model dropdown. The dropdown is gone — models are chosen on the Models page —
 * but "is my model loaded, loading, or broken" still deserves a permanent spot.
 */
export type SttLoadState = "ready" | "loading" | "unloaded" | "error" | "none";

export const useSttLoadState = (): {
  state: SttLoadState;
  error: string | null;
} => {
  const currentModel = useModelStore((store) => store.currentModel);
  const [state, setState] = useState<SttLoadState>("unloaded");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!currentModel) {
      setState("none");
      return;
    }
    let active = true;
    void commands
      .getTranscriptionModelStatus()
      .then((result) => {
        if (!active || result.status !== "ok") return;
        setState(result.data === currentModel ? "ready" : "unloaded");
      })
      .catch(() => {
        if (active) setState("error");
      });
    return () => {
      active = false;
    };
  }, [currentModel]);

  useEffect(() => {
    const unlisten = listen<ModelStateEvent>("model-state-changed", (event) => {
      const { event_type, error: message } = event.payload;
      switch (event_type) {
        case "loading_started":
          setState("loading");
          setError(null);
          break;
        case "loading_completed":
          setState("ready");
          setError(null);
          break;
        case "loading_failed":
          setState("error");
          setError(message ?? null);
          break;
        case "unloaded":
          setState("unloaded");
          setError(null);
          break;
      }
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  return { state, error };
};

/**
 * Switch to a speech model as soon as its download finishes.
 *
 * The footer model selector used to do this as a side effect of being mounted,
 * which is why it has to be preserved explicitly now that the footer is gone:
 * without it, downloading a model from the catalog would leave the old one
 * active and the user would wonder why nothing changed. Restricted to speech
 * models — an assistant or cleanup model can never be the active dictation
 * model, and asking the backend to make it one only produced an error.
 */
export const useAutoSelectDownloadedModel = (enabled: boolean): void => {
  const selectModel = useModelStore((store) => store.selectModel);

  useEffect(() => {
    if (!enabled) return;
    const unlisten = listen<string>("model-download-complete", (event) => {
      const modelId = event.payload;
      window.setTimeout(async () => {
        try {
          const model = useModelStore
            .getState()
            .models.find((entry) => entry.id === modelId);
          if (model && getModelCategory(model) !== "stt") return;
          if (await commands.isRecording()) return;
          await selectModel(modelId);
        } catch {
          // Best effort: the user can still pick it on the Models page.
        }
      }, 500);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [enabled, selectModel]);
};
