import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useOsType } from "@/hooks/useOsType";
import { SectionTitle } from "@/components/ui/Page";
import { Segmented } from "@/components/ui/Segmented";
import { InfoTip } from "@/components/ui/InfoTip";
import { ShortcutInput } from "@/components/settings/ShortcutInput";
import type { RecordingModeSetting } from "@/components/settings/PushToTalk";
import { useNavigation, type PageId } from "@/components/shell/navigation";
import { HANDS_FREE_ID, handsFree } from "@/lib/handsFree";
import { formatKeyCombination } from "@/lib/utils/keyboard";
import { LinuxShortcutHelp } from "./LinuxShortcutHelp";

interface ShortcutRow {
  id: string;
  title: string;
  /** What pressing it does, behind the row's (i). */
  info: string;
  /** Where to turn the feature on, when it is off. */
  off?: PageId;
  /** Held or tapped, for the shortcuts that record. */
  mode?: RecordingModeSetting;
  /** A key pressed on top of the dictation keys, not a shortcut of its own. */
  addOn?: boolean;
}

/**
 * Every shortcut, one line each: a name, an (i) for what it does, and the
 * keys — which are also the button that changes them. Dictate comes first,
 * because it is the one people press all day. Dictation and the ask are each
 * held or tapped on their own (`settings::holds_to_record`), so each carries
 * its own Hold · Tap; the call has none, because a call is a press to start
 * and a press to hang up. With Hold, the hands-free key sits under Dictate,
 * drawn as what it is: one more key on top of the dictation keys.
 */
export const ShortcutsCard: React.FC = () => {
  const { t } = useTranslation();
  const { settings, getSetting, updateSetting, isUpdating } = useSettings();
  const { navigate } = useNavigation();
  const os = useOsType();
  const dictationHeld = getSetting("push_to_talk") ?? true;
  const askHeld = getSetting("assistant_push_to_talk") ?? true;
  const cleanupOn = getSetting("post_process_enabled") ?? false;
  // Cleanup that rides on the dictation shortcut has no keys of its own, so
  // the dictation row says "Dictate and clean up" instead of listing both.
  const cleanupOnDictation =
    cleanupOn && (getSetting("post_process_on_dictation") ?? false);
  const assistantOn = getSetting("assistant_enabled") ?? true;

  const bindings = getSetting("bindings") ?? {};
  const keysOf = (id: string) =>
    formatKeyCombination(bindings[id]?.current_binding ?? "", os);

  // Only where it works: removed, or impossible with these keys or this
  // engine, it is Settings' to explain, not a row here that does nothing.
  const hands = handsFree(settings);
  const handsFreeRow: ShortcutRow[] =
    hands.state === "ready" || hands.state === "taken"
      ? [
          {
            id: HANDS_FREE_ID,
            title: t("settings.general.shortcut.bindings.hands_free.name"),
            info:
              hands.state === "taken"
                ? t("settings.general.handsFree.taken", {
                    name: t(
                      `settings.general.shortcut.bindings.${hands.by}.name`,
                    ),
                    keys: formatKeyCombination(hands.keys, os),
                  })
                : t("home.shortcuts.handsFree.what", {
                    keys: keysOf("transcribe"),
                    key: formatKeyCombination(hands.key, os),
                  }),
            addOn: true,
          },
        ]
      : [];

  const rows: ShortcutRow[] = [
    {
      id: "transcribe",
      title: cleanupOnDictation
        ? t("home.shortcuts.cleanup.title")
        : t("home.shortcuts.dictate.title"),
      info: cleanupOnDictation
        ? t("home.shortcuts.cleanup.what")
        : dictationHeld
          ? t("home.shortcuts.dictate.whatHold")
          : t("home.shortcuts.dictate.whatTap"),
      mode: "push_to_talk",
    },
    ...handsFreeRow,
    ...(cleanupOnDictation
      ? []
      : [
          {
            id: "transcribe_with_post_process",
            title: t("home.shortcuts.cleanup.title"),
            info: t("home.shortcuts.cleanup.what"),
            off: cleanupOn ? undefined : ("cleanup" as PageId),
          },
        ]),
    {
      id: "assistant",
      title: t("home.shortcuts.ask.title"),
      info: askHeld
        ? t("home.shortcuts.ask.whatHold")
        : t("home.shortcuts.ask.whatTap"),
      off: assistantOn ? undefined : "assistant",
      mode: "assistant_push_to_talk",
    },
    {
      id: "assistant_call",
      title: t("home.shortcuts.call.title"),
      info: t("home.shortcuts.call.what"),
      off: assistantOn ? undefined : "assistant",
    },
  ];
  // Cancel is not a global shortcut on Linux (see the Settings dialog).
  if (os !== "linux") {
    rows.push({
      id: "cancel",
      title: t("home.shortcuts.cancel.title"),
      info: t("home.shortcuts.cancel.what"),
    });
  }

  // Linux names the actions in its desktop-shortcut list the way these rows
  // do, Cancel included although it has no row there. The hands-free key is
  // not an action a desktop shortcut can run.
  const labels: Record<string, string> = Object.fromEntries([
    ...rows.filter((row) => !row.addOn).map((row) => [row.id, row.title]),
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

  const modeControl = (row: ShortcutRow, setting: RecordingModeSetting) => {
    const held = setting === "push_to_talk" ? dictationHeld : askHeld;
    return (
      <Segmented
        size="sm"
        label={`${row.title}: ${t("settings.general.pushToTalk.label")}`}
        value={held ? "hold" : "tap"}
        onChange={(mode) => void updateSetting(setting, mode === "hold")}
        disabled={isUpdating(setting)}
        options={[
          {
            value: "hold",
            label: t("settings.general.pushToTalk.hold"),
            title: t("home.shortcuts.mode.hold"),
          },
          {
            value: "tap",
            label: t("settings.general.pushToTalk.tap"),
            title: t("home.shortcuts.mode.tap"),
          },
        ]}
      />
    );
  };

  return (
    <section>
      <SectionTitle
        title={t("home.shortcuts.title")}
        description={t("home.shortcuts.info")}
      />
      <ul className="divide-y divide-hairline rounded-2xl border border-hairline bg-surface elev-card">
        {rows.map((row) => (
          <li
            key={row.id}
            className="flex min-h-[3.5rem] items-center justify-between gap-3 px-5 py-3"
          >
            <span className="flex min-w-0 items-center gap-1">
              <span
                className={`truncate text-[0.9375rem] ${row.off ? "text-muted" : "font-medium text-ink"}`}
              >
                {row.title}
              </span>
              <InfoTip text={row.info} />
            </span>
            {row.off ? (
              <button
                type="button"
                onClick={() => navigate(row.off as PageId)}
                className="shrink-0 cursor-pointer rounded-md px-2 py-1 text-[0.8125rem] font-medium text-accent transition-colors hover:bg-accent/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
              >
                {t("home.shortcuts.turnOn")}
              </button>
            ) : (
              <span className="flex shrink-0 items-center gap-3">
                {row.mode && modeControl(row, row.mode)}
                {/* The keys, right-aligned in a column wide enough for two
                    keycaps, so the Hold · Tap of every row lines up. */}
                <span
                  className={`flex items-center justify-end gap-1.5 ${row.mode ? "min-w-[8rem]" : ""}`}
                >
                  {row.addOn && (
                    <span
                      aria-hidden="true"
                      className="text-[0.9375rem] text-muted"
                    >
                      +
                    </span>
                  )}
                  <ShortcutInput shortcutId={row.id} bare size="md" />
                </span>
              </span>
            )}
          </li>
        ))}
        {globeBusy && (
          <li className="flex min-h-[3.5rem] items-center justify-between gap-3 px-5 py-3">
            <span className="min-w-0 text-[0.8125rem] text-muted">
              {t("home.shortcuts.globe.hint")}
            </span>
            <button
              type="button"
              onClick={() => void commands.openKeyboardSettings()}
              className="shrink-0 cursor-pointer rounded-md px-2 py-1 text-[0.8125rem] font-medium text-accent transition-colors hover:bg-accent/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
            >
              {t("home.shortcuts.globe.open")}
            </button>
          </li>
        )}
        {os === "linux" && <LinuxShortcutHelp labels={labels} />}
      </ul>
    </section>
  );
};
