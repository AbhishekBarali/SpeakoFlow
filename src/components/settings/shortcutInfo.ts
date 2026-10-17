import type { AppSettings } from "@/bindings";

type T = (key: string, options?: Record<string, unknown>) => string;

/**
 * What a shortcut's (i) says: how it is pressed, in one sentence. The same on
 * Home and in Settings → Shortcuts, so the two never explain one shortcut two
 * ways. It follows Hold to talk, which is the one thing about pressing a
 * shortcut that the keys alone do not show.
 */
export const shortcutInfo = (
  t: T,
  settings: AppSettings | null | undefined,
  shortcutId: string,
): string | undefined => {
  const dictationHeld = settings?.push_to_talk ?? true;
  const askHeld = settings?.assistant_push_to_talk ?? true;
  const cleanupOnDictation =
    !!settings?.post_process_enabled && !!settings?.post_process_on_dictation;
  switch (shortcutId) {
    case "transcribe":
      if (cleanupOnDictation) return t("home.shortcuts.cleanup.what");
      return dictationHeld
        ? t("home.shortcuts.dictate.whatHold")
        : t("home.shortcuts.dictate.whatTap");
    case "transcribe_with_post_process":
      return t("home.shortcuts.cleanup.what");
    case "assistant":
      return askHeld
        ? t("home.shortcuts.ask.whatHold")
        : t("home.shortcuts.ask.whatTap");
    case "assistant_call":
      return t("home.shortcuts.call.what");
    case "cancel":
      return t("home.shortcuts.cancel.what");
    default:
      return undefined;
  }
};
