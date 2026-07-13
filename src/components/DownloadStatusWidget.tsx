import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import type { ModelInfo } from "@/bindings";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { useModelStore } from "../stores/modelStore";

/**
 * A quiet, floating bottom-right card that shows any model download still
 * running once the user is inside the main app — so onboarding can hand off
 * immediately and the "your assistant is still moving in" story continues
 * here. Renders nothing when no download is in flight.
 */
export const DownloadStatusWidget: React.FC = () => {
  const { t } = useTranslation();
  const models = useModelStore((s) => s.models);
  const downloadingModels = useModelStore((s) => s.downloadingModels);
  const verifyingModels = useModelStore((s) => s.verifyingModels);
  const extractingModels = useModelStore((s) => s.extractingModels);
  const downloadProgress = useModelStore((s) => s.downloadProgress);

  const activeIds = useMemo(
    () =>
      Array.from(
        new Set([
          ...Object.keys(downloadingModels),
          ...Object.keys(verifyingModels),
          ...Object.keys(extractingModels),
        ]),
      ),
    [downloadingModels, verifyingModels, extractingModels],
  );

  if (activeIds.length === 0) return null;

  const statusLineFor = (id: string): string => {
    const model = models.find((m: ModelInfo) => m.id === id);
    const pct = Math.max(
      0,
      Math.min(100, Math.round(downloadProgress[id]?.percentage ?? 0)),
    );
    const category = model ? getModelCategory(model) : "stt";
    if (category === "llm") {
      return t("onboarding.ready.downloadingAssistant", { percentage: pct });
    }
    if (category === "stt") {
      return t("onboarding.ready.downloadingVoice", { percentage: pct });
    }
    return t("onboarding.ready.downloadingGeneric", { percentage: pct });
  };

  return (
    <div
      role="status"
      aria-live="polite"
      className="fixed bottom-11 end-4 z-50 w-72 rounded-xl border border-hairline bg-surface elev-card shadow-lg p-3 space-y-2.5"
    >
      {activeIds.map((id) => {
        const pct = Math.max(
          0,
          Math.min(100, Math.round(downloadProgress[id]?.percentage ?? 0)),
        );
        return (
          <div key={id}>
            <p className="flex items-center gap-1.5 text-xs text-text/70">
              <Loader2 className="w-3.5 h-3.5 animate-spin text-accent shrink-0" />
              <span>{statusLineFor(id)}</span>
            </p>
            <div className="mt-1.5 w-full h-1.5 bg-mid-gray/20 rounded-full overflow-hidden">
              <div
                className="h-full bg-logo-primary rounded-full transition-all duration-300"
                style={{ width: `${pct}%` }}
              />
            </div>
          </div>
        );
      })}
    </div>
  );
};
