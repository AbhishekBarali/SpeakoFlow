import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { BellRing, Check, Clock, X } from "lucide-react";
import { FONT_SIZES } from "@/assistant/appearance";
// The panel's own stylesheet, so this window is literally the assistant's card
// rather than something that resembles it. Same import the settings preview
// uses, and for the same reason: two copies of a visual language drift.
import "@/assistant/AssistantPanel.css";
import "./ReminderPopup.css";

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

const SNOOZE_CHOICES = [5, 15, 60] as const;

/*
 * Commands are called through `invoke` rather than the generated `commands.*`
 * wrappers on purpose: `src/bindings.ts` is only regenerated while the app runs
 * in dev (see the `#[cfg(debug_assertions)]` export in `lib.rs`), so a fresh
 * checkout would fail to type-check a window that depended on it. The same
 * reason `useVoiceConversation` invokes its ticket commands directly.
 */

/**
 * How long ago a reminder came due, in the coarsest unit that is still true.
 *
 * Exported for its test: "5 min ago" on something 30 seconds overdue is the kind
 * of small lie that makes a user distrust the whole feature, and rounding is
 * exactly where that creeps in.
 */
export function overdueBucket(
  dueAtMs: number,
  nowMs: number,
): { unit: "now" | "minutes" | "hours"; value: number } {
  // `Date.parse` answers NaN for a malformed timestamp, and NaN survives every
  // arithmetic step below — including `Math.max(0, NaN)`, which is NaN, not 0.
  // Without this the popup renders "NaN h ago" beside a perfectly good reminder.
  const elapsed = nowMs - dueAtMs;
  if (!Number.isFinite(elapsed)) return { unit: "now", value: 0 };
  const seconds = Math.max(0, Math.floor(elapsed / 1000));
  if (seconds < 60) return { unit: "now", value: 0 };
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return { unit: "minutes", value: minutes };
  return { unit: "hours", value: Math.floor(minutes / 60) };
}

const ReminderPopup: React.FC = () => {
  const { t } = useTranslation();
  const [reminders, setReminders] = useState<Reminder[]>([]);
  const [snoozeOpenFor, setSnoozeOpenFor] = useState<string | null>(null);
  const [fontSize, setFontSize] = useState(FONT_SIZES.medium);
  // Re-render on a timer so "just now" becomes "3 min ago" while the popup sits
  // there. A reminder often waits a long time, and a stale relative time is
  // worse than none.
  const [, setTick] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);

  const refresh = useCallback(() => {
    void invoke<Reminder[]>("list_waiting_reminders")
      .then(setReminders)
      .catch(() => setReminders([]));
  }, []);

  // Mount: ask for the list rather than waiting for an event. The window is
  // created and shown in the same breath as the first `reminder-due` emit, so a
  // webview still booting would otherwise miss it and render an empty card.
  useEffect(() => {
    refresh();
    const unlisten = listen<Reminder[]>("reminder-due", (event) => {
      setReminders(event.payload ?? []);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refresh]);

  // Follow the panel's text-size setting. Someone who chose Extra large chose it
  // because they cannot comfortably read the default, and a reminder is the one
  // thing in the app they most need to read.
  useEffect(() => {
    void invoke<{ assistant_font_size?: string }>("get_app_settings")
      .then((settings) =>
        setFontSize(
          FONT_SIZES[settings?.assistant_font_size ?? "medium"] ??
            FONT_SIZES.medium,
        ),
      )
      .catch(() => {
        /* Keep the default; a notification must never fail to appear. */
      });
  }, []);

  useEffect(() => {
    const id = window.setInterval(() => setTick((n) => n + 1), 30_000);
    return () => window.clearInterval(id);
  }, []);

  // Report the height the cards actually need, so the window fits them instead
  // of clipping one. A ResizeObserver rather than a one-shot measure, because
  // the content grows in three ways: a second reminder comes due, a long text
  // wraps, or the snooze choices open. That last one is why they are laid out in
  // normal flow instead of as a popover — an absolutely-positioned menu is
  // invisible to this measurement, so it opened straight through the bottom edge
  // of the window and was clipped to a sliver.
  useEffect(() => {
    const node = rootRef.current;
    if (!node) return;
    const report = () => {
      const height = Math.ceil(node.getBoundingClientRect().height);
      if (height > 0) void invoke("fit_reminder_popup", { height });
    };
    report();
    const observer = new ResizeObserver(report);
    observer.observe(node);
    return () => observer.disconnect();
  }, [reminders.length, snoozeOpenFor, fontSize]);

  const done = (id: string) => {
    setSnoozeOpenFor(null);
    void invoke("complete_reminder", { id }).catch(() => refresh());
  };

  const snooze = (id: string, minutes: number) => {
    setSnoozeOpenFor(null);
    void invoke("snooze_reminder", { id, minutes }).catch(() => refresh());
  };

  const relative = (reminder: Reminder) => {
    const bucket = overdueBucket(Date.parse(reminder.due_at), Date.now());
    if (bucket.unit === "now") return t("reminders.due.now");
    if (bucket.unit === "minutes")
      return t("reminders.due.minutesAgo", { count: bucket.value });
    return t("reminders.due.hoursAgo", { count: bucket.value });
  };

  /** "Reminder · just now · Snoozed 2×" — assembled here so no separator
   *  character ends up as a bare string inside JSX. */
  const heading = (reminder: Reminder) =>
    [
      t("reminders.title"),
      relative(reminder),
      reminder.snoozes > 0
        ? t("reminders.snoozedTimes", { count: reminder.snoozes })
        : null,
    ]
      .filter(Boolean)
      .join(" · ");

  if (reminders.length === 0) return null;

  return (
    <div
      className="assistant-scope reminder-scope"
      ref={rootRef}
      style={{ "--as-msg-font": fontSize } as React.CSSProperties}
    >
      {reminders.map((reminder) => (
        <section className="ask-card" key={reminder.id}>
          {/* The card's own header, exactly as the ask card uses it: a quiet
              label on the left, the actions on the right, and the whole strip
              draggable — a reminder can land on top of the thing it is about. */}
          <header className="ask-head" data-tauri-drag-region>
            <BellRing className="ask-head-icon" size={12} />
            <p className="ask-question-text">{heading(reminder)}</p>
            <div className="ask-head-actions">
              <button
                type="button"
                className="ask-action labelled"
                onClick={() => done(reminder.id)}
              >
                <Check size={13} />
                <span>{t("reminders.done")}</span>
              </button>
              <button
                type="button"
                className="ask-action"
                title={t("reminders.snooze")}
                aria-label={t("reminders.snooze")}
                aria-expanded={snoozeOpenFor === reminder.id}
                onClick={() =>
                  setSnoozeOpenFor(
                    snoozeOpenFor === reminder.id ? null : reminder.id,
                  )
                }
              >
                <Clock size={13} />
              </button>
              <button
                type="button"
                className="ask-action close"
                title={t("reminders.later")}
                aria-label={t("reminders.later")}
                onClick={() => void invoke("dismiss_reminder_popup")}
              >
                <X size={13} />
              </button>
            </div>
          </header>

          <div className="ask-answer-body">
            <p>{reminder.text}</p>
            {reminder.note && <p className="reminder-note">{reminder.note}</p>}
          </div>

          {snoozeOpenFor === reminder.id && (
            <div className="reminder-snooze-row">
              {SNOOZE_CHOICES.map((minutes) => (
                <button
                  type="button"
                  className="ask-action labelled"
                  key={minutes}
                  onClick={() => snooze(reminder.id, minutes)}
                >
                  {t("reminders.snoozeFor", { count: minutes })}
                </button>
              ))}
            </div>
          )}
        </section>
      ))}
    </div>
  );
};

export default ReminderPopup;
