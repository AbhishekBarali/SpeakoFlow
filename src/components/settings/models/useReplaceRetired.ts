import { useCallback, useMemo, useState } from "react";
import type { ModelInfo } from "@/bindings";
import { useModelStore } from "@/stores/modelStore";
import {
  replacementPlan,
  replacementView,
  type ReplacementView,
} from "./retiredReplacement";

/**
 * The store wiring for moving off a retired model (the rules themselves are
 * in `retiredReplacement.ts`). Home's prompt, the prompt at the top of the
 * Models page's speech tab and the catalog's retired rows all call this, so
 * pressing "new version" in any of them does the same thing.
 */
export interface ReplaceRetired {
  /** Download `retired`'s new version (selected when it lands, if `retired`
   *  is the selection), or switch to it when it is already here. Resolves to
   *  whether that started. */
  replace: (retired: ModelInfo) => Promise<boolean>;
  /** Where `retired`'s new version stands. */
  viewFor: (retired: ModelInfo) => ReplacementView | null;
  /** Stop a replacement's download. */
  cancel: (replacementId: string) => Promise<void>;
}

export const useReplaceRetired = (): ReplaceRetired => {
  const models = useModelStore((state) => state.models);
  const currentModel = useModelStore((state) => state.currentModel);
  const downloading = useModelStore((state) => state.downloadingModels);
  const verifying = useModelStore((state) => state.verifyingModels);
  const extracting = useModelStore((state) => state.extractingModels);
  const progress = useModelStore((state) => state.downloadProgress);
  const stats = useModelStore((state) => state.downloadStats);
  const errors = useModelStore((state) => state.downloadErrors);
  const downloadModel = useModelStore((state) => state.downloadModel);
  const selectModel = useModelStore((state) => state.selectModel);
  const cancelDownload = useModelStore((state) => state.cancelDownload);
  const setPendingSttSelection = useModelStore(
    (state) => state.setPendingSttSelection,
  );
  const [switchingId, setSwitchingId] = useState<string | null>(null);

  const modelsById = useMemo(
    () => new Map(models.map((model) => [model.id, model])),
    [models],
  );

  const replace = useCallback(
    async (retired: ModelInfo) => {
      const plan = replacementPlan(retired, modelsById, currentModel);
      if (!plan) return false;
      if (plan.kind === "select") {
        setSwitchingId(plan.id);
        try {
          return await selectModel(plan.id);
        } finally {
          setSwitchingId(null);
        }
      }
      if (plan.selectWhenDone) setPendingSttSelection(plan.id);
      return downloadModel(plan.id);
    },
    [
      modelsById,
      currentModel,
      selectModel,
      setPendingSttSelection,
      downloadModel,
    ],
  );

  const viewFor = useCallback(
    (retired: ModelInfo) =>
      replacementView(
        retired,
        modelsById,
        { downloading, verifying, extracting, progress, stats, errors },
        switchingId,
      ),
    [
      modelsById,
      downloading,
      verifying,
      extracting,
      progress,
      stats,
      errors,
      switchingId,
    ],
  );

  const cancel = useCallback(
    async (replacementId: string) => {
      await cancelDownload(replacementId);
    },
    [cancelDownload],
  );

  return { replace, viewFor, cancel };
};
