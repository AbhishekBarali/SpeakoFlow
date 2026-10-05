import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type DesktopShortcutCommand,
  type ShortcutEnvironment,
} from "@/bindings";
import { useSettings } from "@/hooks/useSettings";

/**
 * Linux only: why the shortcuts above may do nothing, and the desktop
 * shortcuts that work instead.
 *
 * On a Wayland session (GNOME, KDE Plasma, Hyprland, Sway, COSMIC) the Tauri
 * engine is only told about a key while an X11 window is in front, and the
 * SpeakoFlow keys engine needs keyboard permissions most systems do not grant.
 * Both used to fail silently, showing the keys as if they worked. A shortcut
 * the desktop owns always works, so this lists the command for each action
 * (see `shortcut::environment`). Renders nothing when shortcuts work.
 */
export const LinuxShortcutHelp: React.FC<{
  /** Display name of each binding id, as the rows above show it. */
  labels: Record<string, string>;
}> = ({ labels }) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const engine = getSetting("keyboard_implementation");
  const assistantOn = getSetting("assistant_enabled");
  const cleanupOn = getSetting("post_process_enabled");
  const cleanupOnDictation = getSetting("post_process_on_dictation");
  const [environment, setEnvironment] = useState<ShortcutEnvironment | null>(
    null,
  );
  const [open, setOpen] = useState(false);

  // Asked again whenever something it depends on changes, and on focus, which
  // is when someone comes back from a terminal or their system settings.
  useEffect(() => {
    let live = true;
    const check = () =>
      void commands
        .getShortcutEnvironment()
        .then((next) => live && setEnvironment(next))
        .catch(() => {});
    check();
    window.addEventListener("focus", check);
    return () => {
      live = false;
      window.removeEventListener("focus", check);
    };
  }, [engine, assistantOn, cleanupOn, cleanupOnDictation]);

  if (!environment || environment.reach === "works") return null;

  return (
    <li className="px-5 py-3">
      <div className="flex min-h-[2rem] items-center justify-between gap-3">
        <span className="min-w-0 text-[0.8125rem] text-muted">
          {environment.reach === "wayland_x11_only"
            ? t("home.shortcuts.linux.wayland")
            : t("home.shortcuts.linux.noAccess")}
        </span>
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
          className="shrink-0 cursor-pointer rounded-md px-2 py-1 text-[0.8125rem] font-medium text-accent transition-colors hover:bg-accent/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
        >
          {open ? t("common.showLess") : t("home.shortcuts.linux.setUp")}
        </button>
      </div>
      {open && (
        <div className="mt-3 flex flex-col gap-3">
          <p className="text-[0.8125rem] text-ink">
            {t(`home.shortcuts.linux.where.${environment.desktop}`)}
          </p>
          <ul className="flex flex-col gap-1.5">
            {environment.commands.map((entry) => (
              <CommandRow
                key={entry.id}
                label={labels[entry.id] ?? entry.id}
                entry={entry}
              />
            ))}
          </ul>
          <p className="text-xs text-muted">
            {t("home.shortcuts.linux.tapOnly")}
          </p>
          {environment.access_command && (
            <div className="flex flex-col gap-1.5">
              <p className="text-[0.8125rem] text-ink">
                {t("home.shortcuts.linux.access")}
              </p>
              <CopyableCommand command={environment.access_command} />
            </div>
          )}
        </div>
      )}
    </li>
  );
};

const CommandRow: React.FC<{
  label: string;
  entry: DesktopShortcutCommand;
}> = ({ label, entry }) => (
  <li className="flex flex-col gap-1 @lg:flex-row @lg:items-center @lg:gap-3">
    <span className="shrink-0 text-[0.8125rem] font-medium text-ink @lg:w-44 @lg:truncate">
      {label}
    </span>
    <CopyableCommand command={entry.command} />
  </li>
);

/** A command in a monospace box, selectable, with a Copy button. */
const CopyableCommand: React.FC<{ command: string }> = ({ command }) => {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const copy = useCallback(() => {
    void navigator.clipboard?.writeText(command).then(
      () => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      },
      () => {},
    );
  }, [command]);
  return (
    <span className="flex min-w-0 flex-1 items-center gap-2">
      <code className="min-w-0 flex-1 select-text break-all rounded-md bg-ink/[0.06] px-2 py-1 font-mono text-xs text-ink">
        {command}
      </code>
      <button
        type="button"
        onClick={copy}
        className="shrink-0 cursor-pointer rounded-md px-2 py-1 text-xs font-medium text-accent transition-colors hover:bg-accent/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
      >
        {copied
          ? t("home.shortcuts.linux.copied")
          : t("home.shortcuts.linux.copy")}
      </button>
    </span>
  );
};
