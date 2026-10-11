import type { AppSettings, ModelInfo, PostProcessReadiness } from "@/bindings";
import { isCleanupSpecialistModel } from "@/lib/utils/cleanupSpecialist";

/** The cleanup fine-tune cleanup is pointed at, if it is pointed at one. */
export interface CleanupSpecialist {
  /** The exact model string cleanup will be sent. */
  modelId: string;
  /** The catalog entry behind it, when the app has one. */
  model: ModelInfo | null;
}

/**
 * Whether the model that will run cleanup is a cleanup fine-tune.
 *
 * The backend's readiness resolver names the model first, because cleanup
 * falls back to the assistant's selection when its own is incomplete; the
 * stored selection is only consulted when nothing is ready yet. A model is a
 * specialist when its catalog entry says so, or when its name does, which is
 * what covers a copy served from the user's own Ollama or LM Studio endpoint
 * that has no catalog entry at all.
 */
export const findCleanupSpecialist = (
  settings: AppSettings | null | undefined,
  models: ModelInfo[],
  readiness: PostProcessReadiness | null | undefined,
): CleanupSpecialist | null => {
  const modelId =
    readiness?.state === "ready"
      ? readiness.model
      : (settings?.post_process_models?.[
          settings?.post_process_provider_id ?? ""
        ] ?? "");
  if (!modelId.trim()) return null;
  const model = models.find((entry) => entry.id === modelId) ?? null;
  const specialist =
    isCleanupSpecialistModel(modelId) || !!model?.is_cleanup_specialist;
  return specialist ? { modelId, model } : null;
};
