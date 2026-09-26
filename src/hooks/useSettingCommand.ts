import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useSettings } from "./useSettings";

type Settled = { status: "ok" | "error"; error?: unknown } | null | void;

/**
 * Run a settings command, then pull the authoritative settings back.
 *
 * Most commands answer with a `Result`; a rejected one used to be awaited and
 * ignored, so a control could look changed and quietly snap back on the next
 * refresh. This checks the result, says so in a toast when it failed, and
 * refreshes either way so the control always ends up showing the truth.
 * Resolves to whether the command succeeded.
 */
export const useSettingCommand = () => {
  const { t } = useTranslation();
  const { refreshSettings } = useSettings();
  return useCallback(
    async (command: Promise<Settled>): Promise<boolean> => {
      let ok = true;
      try {
        const result = await command;
        if (result && result.status === "error") {
          throw new Error(String(result.error));
        }
      } catch (error) {
        ok = false;
        console.error("Settings command failed:", error);
        toast.error(t("common.saveFailed"));
      }
      await refreshSettings();
      return ok;
    },
    [refreshSettings, t],
  );
};
