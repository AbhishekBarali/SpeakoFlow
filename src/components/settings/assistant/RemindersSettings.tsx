import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { BellRing, Trash2 } from "lucide-react";
import { SettingContainer, SettingsGroup } from "@/components/ui";

/** Mirrors the Rust `Reminder`. Timestamps are RFC 3339 UTC strings. */
interface Reminder {
  id: string;
  text: string;
  note?: string | null;
  due_at: string;
  created_at: string;
  snoozes: number;
  fired: boolean;
}

/**
 * The reminders on the books, with a way to cancel one.
 *
 * A reminder is created by talking to the assistant, which is the point — but a
 * list you can only reach by asking is a list you cannot trust. This is the
 * answer to "did that actually get set?" and the only way to cancel one without
 * a conversation.
 */
export const RemindersSettings: React.FC = () => {
  const { t } = useTranslation();
  const [reminders, setReminders] = useState<Reminder[]>([]);

  const refresh = useCallback(() => {
    void invoke<Reminder[]>("list_reminders")
      .then(setReminders)
      .catch(() => setReminders([]));
  }, []);

  useEffect(() => {
    refresh();
    // The list changes from outside this window — a reminder set by voice, one
    // that just came due, one dismissed in the popup — so it follows the same
    // event the popup does rather than polling.
    const unlisten = listen<Reminder[]>("reminders-changed", (event) => {
      setReminders(event.payload ?? []);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refresh]);

  const cancel = (id: string) => {
    void invoke("complete_reminder", { id }).then(refresh).catch(refresh);
  };

  const when = (reminder: Reminder) => {
    const due = new Date(reminder.due_at);
    if (Number.isNaN(due.getTime())) return reminder.due_at;
    const sameDay = due.toDateString() === new Date().toDateString();
    return due.toLocaleString(undefined, {
      hour: "numeric",
      minute: "2-digit",
      ...(sameDay ? {} : { weekday: "short", day: "numeric", month: "short" }),
    });
  };

  return (
    <SettingsGroup title={t("reminders.settings.label")} icon={BellRing}>
      <SettingContainer
        title={t("reminders.settings.label")}
        description={t("reminders.settings.description")}
        descriptionMode="inline"
        layout="stacked"
        grouped={true}
      >
        {reminders.length === 0 ? (
          <p className="text-[13px] text-muted">
            {t("reminders.settings.empty")}
          </p>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {reminders.map((reminder) => (
              <li
                key={reminder.id}
                className="flex items-start gap-3 rounded-xl border border-hairline bg-surface-strong/45 px-3 py-2.5"
              >
                <div className="min-w-0 flex-1">
                  <p className="text-[13px] text-ink break-words">
                    {reminder.text}
                  </p>
                  {reminder.note && (
                    <p className="mt-0.5 text-[12px] text-muted break-words">
                      {reminder.note}
                    </p>
                  )}
                  <p className="mt-1 text-[12px] text-muted">
                    {reminder.fired
                      ? t("reminders.settings.waiting")
                      : t("reminders.settings.dueAt", { when: when(reminder) })}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => cancel(reminder.id)}
                  title={t("reminders.settings.cancel")}
                  aria-label={t("reminders.settings.cancel")}
                  className="mt-0.5 shrink-0 cursor-pointer rounded-md p-1.5 text-muted transition-colors hover:bg-error/10 hover:text-error focus:outline-none focus-visible:ring-2 focus-visible:ring-error/60"
                >
                  <Trash2 size={15} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </SettingContainer>
    </SettingsGroup>
  );
};
