import React, { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useSettings } from "../../hooks/useSettings";
import { useNavigation } from "../shell/navigation";
import { useUpdateStore } from "./updateStore";
import { takeUpdateNotice } from "./updateCommands";
import { FIRST_CHECK_DELAY_MS, RECHECK_INTERVAL_MS } from "./updateLogic";

/**
 * Runs the background update checks and answers the tray's "Check for
 * updates…". Renders nothing: what it finds is shown by `UpdatePill` in the
 * sidebar and `UpdatePanel` in Settings → About, both reading `useUpdateStore`.
 *
 * Checks once shortly after launch and then every twelve hours, because the
 * app lives in the tray and a launch-only check never fires again for someone
 * who leaves it running for weeks. `update_checks_enabled` turns the automatic
 * checks off; asking (tray, About) always works.
 */
const UpdateChecker: React.FC = () => {
  const { t } = useTranslation();
  const { settings, isLoading } = useSettings();
  const { openSettings } = useNavigation();
  const settingsLoaded = !isLoading && settings !== null;
  const automatic = settings?.update_checks_enabled ?? true;

  useEffect(() => {
    void useUpdateStore.getState().loadSupport();
  }, []);

  // The launch right after an in-app update says so; otherwise the app just
  // reappears and nothing confirms the update worked.
  useEffect(() => {
    void takeUpdateNotice()
      .then((finished) => {
        if (finished) {
          toast.success(t("updates.updatedTo", { version: finished.to }));
        }
      })
      .catch((error) => console.warn("take_update_notice failed:", error));
  }, [t]);

  useEffect(() => {
    if (!settingsLoaded || !automatic) return;
    const run = () => void useUpdateStore.getState().check();
    const first = window.setTimeout(run, FIRST_CHECK_DELAY_MS);
    const repeat = window.setInterval(run, RECHECK_INTERVAL_MS);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(repeat);
    };
  }, [settingsLoaded, automatic]);

  useEffect(() => {
    const unlisten = listen("check-for-updates", () => {
      openSettings("about");
      void useUpdateStore.getState().check({ manual: true });
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [openSettings]);

  return null;
};

export default UpdateChecker;
