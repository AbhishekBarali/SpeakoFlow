import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "@/hooks/useSettings";
import { useModelStore } from "@/stores/modelStore";
import { localModelLabel } from "@/components/shell/useModelSlots";
import { prettyModelName } from "@/lib/utils/prettyModelName";
import { findCleanupSpecialist } from "./cleanupSpecialist";

/**
 * The cleanup fine-tune cleanup will run on, with the name to call it by, or
 * null when cleanup runs on a general-purpose model.
 *
 * Every place on the Cleanup page that warns about a fine-tune reads this, so
 * the prompt editor and the writing styles cannot disagree about which model
 * is in use.
 */
export const useCleanupSpecialist = (): { name: string } | null => {
  const { t } = useTranslation();
  const { settings, postProcessReadiness } = useSettings();
  const models = useModelStore((state) => state.models);

  return useMemo(() => {
    const found = findCleanupSpecialist(settings, models, postProcessReadiness);
    if (!found) return null;
    const name = found.model
      ? localModelLabel(found.model, t)
      : prettyModelName(found.modelId) || found.modelId;
    return { name };
  }, [settings, models, postProcessReadiness, t]);
};
