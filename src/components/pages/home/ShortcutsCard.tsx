import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { commands } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useOsType } from "@/hooks/useOsType";
import { SectionTitle, TextLink } from "@/components/ui/Page";
import { InfoTip } from "@/components/ui/InfoTip";
import { ShortcutInput } from "@/components/settings/ShortcutInput";
import { KeysPrefix, RowAction } from "@/components/settings/ShortcutRow";
import { handsFreeInfo } from "@/components/settings/HandsFree";
import { shortcutInfo } from "@/components/settings/shortcutInfo";
import { useNavigation, type PageId } from "@/components/shell/navigation";
import { HANDS_FREE_ID, handsFree } from "@/lib/handsFree";
import { LinuxShortcutHelp } from "./LinuxShortcutHelp";

interface ShortcutRow {
  id: string;
  title: string;
  /** What pressing it does, behind the row's (i). */
  info?: string;
  /** Where to turn the feature on, when it is off. */
  off?: PageId;
}

/** Set once the hands-free hint is closed, so it never comes back. */
const HINT_CLOSED_KEY = "speakoflow.home.handsFreeHintClosed";

const hintClosed = () => {
  try {
    return window.localStorage.getItem(HINT_CLOSED_KEY) === "1";
  } catch {
    return false;
  }
};

/**
 * Every shortcut, one line each: a name, an (i) for how it is pressed, and the
 * keys, which are also the button that changes them. Nothing else sits on a
 * row: Hold to talk is under Manage, and the (i) follows it.
 *
 * Hands-free is the one row whose keys need words in front of them, because
 * its key is pressed while dictating, not on its own: "While dictating, press
 * [Space]". Where there can be no hands-free key (Linux's Ctrl + Space, the
 * other keyboard engine), a one-line hint says what to do instead, with a
 * one-click fix and an X that closes it for good.
 */
export const ShortcutsCard: React.FC = () => {
  const { t } = useTranslation();
  const { settings, getSetting, updateSetting, isUpdating } = useSettings();
  const { navigate, openSettings } = useNavigation();
  const os = useOsType();
  const cleanupOn = getSetting("post_process_enabled") ?? false;
  // Cleanup that rides on the dictation shortcut has no keys of its own, so
  // the dictation row says "Dictate and clean up" instead of listing both.
  const cleanupOnDictation =
    cleanupOn && (getSetting("post_process_on_dictation") ?? false);
  const assistantOn = getSetting("assistant_enabled") ?? true;
  const bindings = getSetting("bindings") ?? {};
  const info = (id: string) => shortcutInfo(t, settings, id);

  const hands = handsFree(settings);
  const handsInfo = handsFreeInfo(
    t,
    hands,
    bindings.transcribe?.current_binding,
    os,
  );
  const [hintHidden, setHintHidden] = useState(hintClosed);
  const closeHint = () => {
    setHintHidden(true);
    try {
      window.localStorage.setItem(HINT_CLOSED_KEY, "1");
    } catch {
      // Closed for this session, then.
    }
  };

  const rows: ShortcutRow[] = [
    {
      id: "transcribe",
      title: cleanupOnDictation
        ? t("home.shortcuts.cleanup.title")
        : t("home.shortcuts.dictate.title"),
      info: info("transcribe"),
    },
    // With Hold to talk off every dictation already is hands-free, and where
    // there can be no key the hint below takes its place.
    ...(hands.state === "tapped" || hands.state === "unavailable"
      ? []
      : [
          {
            id: HANDS_FREE_ID,
            title: t("settings.general.shortcut.bindings.hands_free.name"),
            info: handsInfo,
          },
        ]),
    ...(cleanupOnDictation
      ? []
      : [
          {
            id: "transcribe_with_post_process",
            title: t("home.shortcuts.cleanup.title"),
            info: info("transcribe_with_post_process"),
            off: cleanupOn ? undefined : ("cleanup" as PageId),
          },
        ]),
    {
      id: "assistant",
      title: t("home.shortcuts.ask.title"),
      info: info("assistant"),
      off: assistantOn ? undefined : "assistant",
    },
    {
      id: "assistant_call",
      title: t("home.shortcuts.call.title"),
      info: info("assistant_call"),
      off: assistantOn ? undefined : "assistant",
    },
  ];
  // Cancel is not a global shortcut on Linux (see the Settings dialog).
  if (os !== "linux") {
    rows.push({
      id: "cancel",
      title: t("home.shortcuts.cancel.title"),
      info: info("cancel"),
    });
  }

  // Linux names the actions in its desktop-shortcut list the way these rows
  // do, Cancel included although it has no row there. The hands-free key is
  // not an action a desktop shortcut can run.
  const labels: Record<string, string> = Object.fromEntries([
    ...rows
      .filter((row) => row.id !== HANDS_FREE_ID)
      .map((row) => [row.id, row.title]),
    ["cancel", t("home.shortcuts.cancel.title")],
  ]);

  // On a Mac the dictation default is the globe key, which macOS also acts on
  // unless "Press 🌐 key to" is Do Nothing. Checked again when the window
  // regains focus, which is when someone comes back from System Settings.
  const usesGlobe =
    os === "macos" &&
    rows.some(
      (row) =>
        !row.off &&
        (bindings[row.id]?.current_binding ?? "")
          .split("+")
          .some((part) => part.trim().toLowerCase() === "fn"),
    );
  const [globeBusy, setGlobeBusy] = useState(false);
  useEffect(() => {
    if (!usesGlobe) {
      setGlobeBusy(false);
      return;
    }
    let live = true;
    const check = () =>
      void commands
        .globeKeyHasOwnAction()
        .then((busy) => live && setGlobeBusy(busy))
        .catch(() => {});
    check();
    window.addEventListener("focus", check);
    return () => {
      live = false;
      window.removeEventListener("focus", check);
    };
  }, [usesGlobe]);

  const hint =
    hands.state === "unavailable" && !hintHidden
      ? os === "linux"
        ? t("home.shortcuts.handsFree.linux")
        : handsInfo
      : null;

  const rowClass =
    "flex min-h-[3.5rem] items-center justify-between gap-3 px-5 py-3";

  return (
    <section>
      <SectionTitle
        title={t("home.shortcuts.title")}
        description={t("home.shortcuts.info")}
        action={
          <TextLink onClick={() => openSettings("shortcuts")}>
            {t("home.setup.manage")}
          </TextLink>
        }
      />
      <ul className="divide-y divide-hairline rounded-2xl border border-hairline bg-surface elev-card">
        {rows.map((row) => (
          <React.Fragment key={row.id}>
            <li className={rowClass}>
              <span className="flex min-w-0 items-center gap-1">
                <span
                  className={`truncate text-[0.9375rem] ${row.off ? "text-muted" : "font-medium text-ink"}`}
                >
                  {row.title}
                </span>
                {row.info && <InfoTip text={row.info} />}
              </span>
              {row.off ? (
                <RowAction onClick={() => navigate(row.off as PageId)}>
                  {t("home.shortcuts.turnOn")}
                </RowAction>
              ) : (
                <span className="flex shrink-0 items-center gap-2">
                  {row.id === HANDS_FREE_ID && hands.state !== "off" && (
                    <KeysPrefix>
                      {t("settings.general.handsFree.whileDictating")}
                    </KeysPrefix>
                  )}
                  <ShortcutInput shortcutId={row.id} bare size="md" />
                </span>
              )}
            </li>
            {row.id === "transcribe" && hint && (
              <li className="flex items-center gap-2 px-5 py-2.5">
                <span className="min-w-0 flex-1 text-[0.8125rem] leading-snug text-muted">
                  {hint}
                </span>
                <RowAction
                  onClick={() => void updateSetting("push_to_talk", false)}
                  disabled={isUpdating("push_to_talk")}
                >
                  {t("shortcutEditor.turnOff")}
                </RowAction>
                <button
                  type="button"
                  onClick={closeHint}
                  aria-label={t("common.close")}
                  title={t("common.close")}
                  className="inline-flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-md text-muted transition-colors hover:bg-ink/[0.06] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
                >
                  <X className="h-3.5 w-3.5" aria-hidden="true" />
                </button>
              </li>
            )}
          </React.Fragment>
        ))}
        {globeBusy && (
          <li className={rowClass}>
            <span className="min-w-0 text-[0.8125rem] text-muted">
              {t("home.shortcuts.globe.hint")}
            </span>
            <RowAction onClick={() => void commands.openKeyboardSettings()}>
              {t("home.shortcuts.globe.open")}
            </RowAction>
          </li>
        )}
        {os === "linux" && <LinuxShortcutHelp labels={labels} />}
      </ul>
    </section>
  );
};
