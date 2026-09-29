import React from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "@/hooks/useSettings";
import { useOsType } from "@/hooks/useOsType";
import { SectionTitle } from "@/components/ui/Page";
import { Segmented } from "@/components/ui/Segmented";
import { InfoTip } from "@/components/ui/InfoTip";
import { ShortcutInput } from "@/components/settings/ShortcutInput";
import { useNavigation, type PageId } from "@/components/shell/navigation";

interface ShortcutRow {
  id: string;
  title: string;
  /** What pressing it does, behind the row's (i). */
  info: string;
  /** Where to turn the feature on, when it is off. */
  off?: PageId;
}

/**
 * The other shortcuts, one line each: a name, an (i) for what it does, and the
 * keys — which are also the button that changes them. Dictation has the banner
 * above to itself. Hold-or-tap applies to every recording shortcut at once
 * (that is how the backend treats `push_to_talk`), so it is one switch in the
 * header rather than a setting repeated per row.
 */
export const ShortcutsCard: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const { navigate } = useNavigation();
  const os = useOsType();
  const holdToTalk = getSetting("push_to_talk") ?? true;
  const cleanupOn = getSetting("post_process_enabled") ?? false;
  // Cleanup that rides on the dictation shortcut has no keys of its own; the
  // hero above already says "Dictate and clean up".
  const cleanupOnDictation =
    cleanupOn && (getSetting("post_process_on_dictation") ?? false);
  const assistantOn = getSetting("assistant_enabled") ?? true;

  const rows: ShortcutRow[] = [
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
      info: t("home.shortcuts.ask.what"),
      off: assistantOn ? undefined : "assistant",
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

  return (
    <section>
      <SectionTitle
        title={t("home.shortcuts.title")}
        description={t("home.shortcuts.info")}
        action={
          <Segmented
            size="sm"
            label={t("settings.general.pushToTalk.label")}
            value={holdToTalk ? "hold" : "tap"}
            onChange={(mode) =>
              void updateSetting("push_to_talk", mode === "hold")
            }
            disabled={isUpdating("push_to_talk")}
            options={[
              { value: "hold", label: t("home.shortcuts.mode.hold") },
              { value: "tap", label: t("home.shortcuts.mode.tap") },
            ]}
          />
        }
      />
      <ul className="divide-y divide-hairline rounded-2xl border border-hairline bg-surface elev-card">
        {rows.map((row) => (
          <li
            key={row.id}
            className="flex min-h-[3.25rem] items-center justify-between gap-3 px-4 py-2"
          >
            <span className="flex min-w-0 items-center gap-1">
              <span
                className={`truncate text-sm ${row.off ? "text-muted" : "font-medium text-ink"}`}
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
              <ShortcutInput
                shortcutId={row.id}
                bare
                size="sm"
                showReset="never"
              />
            )}
          </li>
        ))}
      </ul>
    </section>
  );
};
