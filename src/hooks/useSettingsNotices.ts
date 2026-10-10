import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import {
  commands,
  type SettingsNotice,
  type SettingsNoticeKind,
} from "@/bindings";
import {
  describeSettingsNotice,
  settingsNoticeToastId,
  settingsNoticesToTakeDown,
} from "@/lib/settingsNotices";

/** How long the restore notice stays up. It lost nothing, so it goes away. */
const RESTORED_TOAST_MS = 10_000;

/**
 * Tell the user what happened to their settings file (see
 * `settings_file.rs`): a restore from the backup, settings that could not be
 * recovered, or changes that are not being saved. The file is read before this
 * window exists, so the current notices are fetched on mount; a notice a later
 * save raises or clears arrives as `settings-notices`.
 */
export const useSettingsNotices = () => {
  const { t } = useTranslation();

  useEffect(() => {
    const shown = new Set<SettingsNoticeKind>();
    let heardEvent = false;
    let active = true;

    const dismissed = (kind: SettingsNoticeKind) => {
      if (!shown.delete(kind)) return;
      commands.dismissSettingsNotice(kind).catch((error) => {
        console.warn("Failed to dismiss the settings notice:", error);
      });
    };

    const show = (notice: SettingsNotice) => {
      const { title, description, warning } = describeSettingsNotice(notice, t);
      const id = settingsNoticeToastId(notice.kind);
      const onDismiss = () => dismissed(notice.kind);
      if (!warning) {
        toast(title, {
          id,
          description,
          duration: RESTORED_TOAST_MS,
          onDismiss,
          onAutoClose: onDismiss,
        });
        return;
      }
      toast.warning(title, {
        id,
        description,
        duration: Infinity,
        action: { label: t("settingsNotice.dismiss"), onClick: onDismiss },
        onDismiss,
      });
    };

    const apply = (notices: SettingsNotice[]) => {
      if (!active) return;
      for (const kind of settingsNoticesToTakeDown(shown, notices)) {
        shown.delete(kind);
        toast.dismiss(settingsNoticeToastId(kind));
      }
      for (const notice of notices) {
        if (shown.has(notice.kind)) continue;
        shown.add(notice.kind);
        show(notice);
      }
    };

    const unlisten = listen<SettingsNotice[]>("settings-notices", (event) => {
      heardEvent = true;
      apply(event.payload ?? []);
    });
    commands
      .getSettingsNotices()
      .then((notices) => {
        // An event that already arrived is newer than this answer.
        if (!heardEvent) apply(notices);
      })
      .catch((error) => {
        console.warn("Failed to read the settings notices:", error);
      });

    return () => {
      active = false;
      unlisten.then((fn) => fn());
    };
  }, [t]);
};
