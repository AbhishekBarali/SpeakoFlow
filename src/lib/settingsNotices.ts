import type { SettingsNotice, SettingsNoticeKind } from "@/bindings";

/** The sonner id for a notice, so each kind is one toast that updates in place. */
export const settingsNoticeToastId = (kind: SettingsNoticeKind) =>
  `settings-notice-${kind}`;

export interface SettingsNoticeText {
  title: string;
  description?: string;
  /**
   * A warning stays until it is dismissed: the user lost something or is about
   * to. A restore from the backup lost nothing, so it is an ordinary toast.
   */
  warning: boolean;
}

/** What a notice says, in the user's language. */
export const describeSettingsNotice = (
  notice: SettingsNotice,
  t: (key: string, options?: Record<string, unknown>) => string,
): SettingsNoticeText => {
  switch (notice.kind) {
    case "restored":
      return { title: t("settingsNotice.restored"), warning: false };
    case "unrecoverable":
      return {
        title: t("settingsNotice.unrecoverableTitle"),
        description: notice.kept_as
          ? t("settingsNotice.unrecoverableDescription", {
              file: notice.kept_as,
            })
          : undefined,
        warning: true,
      };
    case "not_saving":
      return {
        title: t("settingsNotice.notSavingTitle"),
        description: t("settingsNotice.notSavingDescription"),
        warning: true,
      };
    case "save_failing":
      return {
        title: t("settingsNotice.saveFailingTitle"),
        description: t("settingsNotice.saveFailingDescription"),
        warning: true,
      };
  }
};

/**
 * Kinds on screen that the backend no longer reports, so their toast comes
 * down: saving works again, which clears `save_failing`.
 */
export const settingsNoticesToTakeDown = (
  shown: Iterable<SettingsNoticeKind>,
  current: SettingsNotice[],
): SettingsNoticeKind[] => {
  const live = new Set(current.map((notice) => notice.kind));
  return [...shown].filter((kind) => !live.has(kind));
};
