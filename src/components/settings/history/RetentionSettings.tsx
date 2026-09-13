import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle } from "lucide-react";
import { toast } from "sonner";
import { commands, type RecordingRetentionPeriod } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { Button } from "../../ui/Button";
import { Dropdown } from "../../ui/Dropdown";
import { Input } from "../../ui/Input";
import { SettingContainer } from "../../ui/SettingContainer";

/** Must match `MIN_HISTORY_LIMIT` / `MAX_HISTORY_LIMIT` in `src-tauri/src/settings.rs`.
 *  The minimum is 1, not 0: a limit of 0 deletes every unstarred recording, which
 *  is what an emptied input field would otherwise mean. */
const MIN_LIMIT = 1;
const MAX_LIMIT = 1000;

/** Must match `MIN_RECORDING_RETENTION_DAYS` / `MAX_RECORDING_RETENTION_DAYS`. */
const MIN_DAYS = 1;
const MAX_DAYS = 3650;

/**
 * The literals here are the real wire format — `days3`, not `days_3`.
 *
 * serde (which writes the settings store and parses the command argument) and
 * specta (which generates `bindings.ts`) disagree on where a digit starts a new
 * word, so the generated union used to advertise values the backend never
 * accepted. The Rust enum now pins each string with an explicit `#[serde(rename)]`,
 * which both honor, so `RecordingRetentionPeriod` can finally be used as a real
 * type instead of being cast around.
 */
const PERIOD_OPTIONS: { value: RecordingRetentionPeriod; labelKey: string }[] =
  [
    { value: "never", labelKey: "settings.debug.recordingRetention.never" },
    {
      value: "preserve_limit",
      labelKey: "settings.debug.recordingRetention.preserveLimit",
    },
    { value: "days3", labelKey: "settings.debug.recordingRetention.days3" },
    { value: "weeks2", labelKey: "settings.debug.recordingRetention.weeks2" },
    { value: "months3", labelKey: "settings.debug.recordingRetention.months3" },
    {
      value: "custom_days",
      labelKey: "settings.debug.recordingRetention.customDays",
    },
  ];

/** A change that would delete recordings, held back until the user confirms it. */
interface PendingChange {
  /** How many recordings go away, or `null` when the count couldn't be read. */
  count: number | null;
  apply: () => Promise<void>;
}

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

/**
 * Storage controls for the History feed: how recordings are auto-deleted, and the
 * count or day window the chosen policy needs.
 *
 * The three controls live in one component because they share two things that
 * were previously missing. First, an honest result: `updateSetting` resolves
 * `false` on failure rather than throwing, and the old rows `await`ed it and then
 * toasted success unconditionally — so a rejected value showed "applied" and
 * silently reverted on the next refresh. Second, a confirmation: retention
 * deletes the WAV file along with the row, so tightening a policy is the most
 * destructive action in this panel and it used to happen instantly, with no
 * warning and no count.
 */
export const RetentionSettings: React.FC<{ grouped?: boolean }> = ({
  grouped = true,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();

  // No cast needed: the generated type now lists the literals the backend really
  // accepts, which is the fix this whole component depends on.
  const period = getSetting("recording_retention_period") ?? "preserve_limit";
  const storedLimit = getSetting("history_limit") ?? 20;
  const storedDays = getSetting("recording_retention_days") ?? 30;

  const [pending, setPending] = useState<PendingChange | null>(null);
  const [limitInput, setLimitInput] = useState(String(storedLimit));
  const [daysInput, setDaysInput] = useState(String(storedDays));

  useEffect(() => setLimitInput(String(storedLimit)), [storedLimit]);
  useEffect(() => setDaysInput(String(storedDays)), [storedDays]);

  // A dropdown selection or a blurred input can resolve after the section is
  // gone (the History page unmounts on section change), so don't touch state
  // afterwards.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const busy =
    isUpdating("recording_retention_period") ||
    isUpdating("history_limit") ||
    isUpdating("recording_retention_days");

  /**
   * Ask the backend how many recordings a prospective policy would delete.
   *
   * Returns `null` when the count could not be determined. That is deliberately
   * distinct from `0`: an unknown count must still raise the confirmation, because
   * the alternative is deleting an unknown number of recordings unannounced.
   */
  const previewCount = useCallback(
    async (
      nextPeriod: RecordingRetentionPeriod,
      nextLimit: number,
      nextDays: number,
    ): Promise<number | null> => {
      try {
        const result = await commands.previewRecordingRetention(
          nextPeriod,
          nextLimit,
          nextDays,
        );
        return result.status === "ok" ? result.data : null;
      } catch (error) {
        // The generated wrapper rethrows real `Error`s, so an IPC failure would
        // otherwise escape the (void-typed) dropdown/blur handler unhandled and
        // the change would vanish with no toast and no console trace.
        console.error("Failed to preview recording retention:", error);
        return null;
      }
    },
    [],
  );

  /**
   * Run a settings write and report what actually happened. `updateSetting`
   * returns `false` instead of throwing, which is the failure mode the old rows
   * reported as success.
   */
  const commit = useCallback(
    async (
      write: () => Promise<boolean>,
      onFailure?: () => void,
    ): Promise<void> => {
      const ok = await write();
      if (!mounted.current) return;
      if (ok) {
        toast.success(t("settings.debug.recordingRetention.appliedToast"));
      } else {
        onFailure?.();
        toast.error(t("settings.debug.recordingRetention.saveFailed"));
      }
    },
    [t],
  );

  /** A pending change is superseded as soon as the user makes another one. */
  const generation = useRef(0);

  /**
   * Apply immediately only when the backend confirmed nothing is destroyed.
   * A count of 0 applies; `null` (unknown) and anything positive ask first.
   */
  const guard = useCallback(
    async (token: number, count: number | null, apply: () => Promise<void>) => {
      // A newer selection or edit started while this preview was in flight.
      // Applying now would write the value the user has already moved on from.
      if (token !== generation.current) return;
      if (count === 0) {
        await apply();
        return;
      }
      if (mounted.current) setPending({ count, apply });
    },
    [],
  );

  /** Start a change: invalidate anything in flight and clear a stale prompt. */
  const beginChange = useCallback(() => {
    generation.current += 1;
    setPending(null);
    return generation.current;
  }, []);

  const handlePeriodSelect = async (value: string) => {
    const nextPeriod = value as RecordingRetentionPeriod;
    if (nextPeriod === period) return;
    const token = beginChange();

    const count = await previewCount(nextPeriod, storedLimit, storedDays);
    await guard(token, count, () =>
      commit(() => updateSetting("recording_retention_period", nextPeriod)),
    );
  };

  const handleLimitBlur = async () => {
    const parsed = Number.parseInt(limitInput, 10);
    if (!Number.isFinite(parsed)) {
      setLimitInput(String(storedLimit));
      return;
    }

    const next = clamp(parsed, MIN_LIMIT, MAX_LIMIT);
    setLimitInput(String(next));
    if (next === storedLimit) return;
    const token = beginChange();

    const count = await previewCount(period, next, storedDays);
    await guard(token, count, () =>
      commit(
        () => updateSetting("history_limit", next),
        // Put the input back to what is actually stored, so a rejected value
        // can't sit on screen looking saved.
        () => setLimitInput(String(storedLimit)),
      ),
    );
  };

  const handleDaysBlur = async () => {
    const parsed = Number.parseInt(daysInput, 10);
    if (!Number.isFinite(parsed)) {
      setDaysInput(String(storedDays));
      return;
    }

    const next = clamp(parsed, MIN_DAYS, MAX_DAYS);
    setDaysInput(String(next));
    if (next === storedDays) return;
    const token = beginChange();

    const count = await previewCount(period, storedLimit, next);
    await guard(token, count, () =>
      commit(
        () => updateSetting("recording_retention_days", next),
        () => setDaysInput(String(storedDays)),
      ),
    );
  };

  /** Enter commits, so a typed number doesn't need a click elsewhere to save. */
  const commitOnEnter = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      event.currentTarget.blur();
    }
  };

  const confirmPending = async () => {
    if (!pending) return;
    const { apply } = pending;
    setPending(null);
    await apply();
  };

  const cancelPending = () => {
    setPending(null);
    // Both inputs mirror the stored value again; the dropdown already reads from
    // the store, so nothing else needs resetting.
    setLimitInput(String(storedLimit));
    setDaysInput(String(storedDays));
  };

  const confirmHeading =
    pending === null
      ? ""
      : pending.count === null
        ? t("settings.debug.recordingRetention.confirmTitleUnknown")
        : t("settings.debug.recordingRetention.confirmTitle", {
            count: pending.count,
          });

  return (
    <>
      <SettingContainer
        title={t("settings.debug.recordingRetention.title")}
        description={t("settings.debug.recordingRetention.description")}
        descriptionMode="tooltip"
        grouped={grouped}
      >
        <Dropdown
          options={PERIOD_OPTIONS.map((option) => ({
            value: option.value,
            label: t(option.labelKey),
          }))}
          selectedValue={period}
          onSelect={handlePeriodSelect}
          placeholder={t("settings.debug.recordingRetention.placeholder")}
          disabled={busy}
        />
      </SettingContainer>

      {period === "preserve_limit" && (
        <SettingContainer
          title={t("settings.debug.historyLimit.title")}
          description={t("settings.debug.historyLimit.description")}
          descriptionMode="tooltip"
          grouped={grouped}
          layout="horizontal"
        >
          <div className="flex items-center space-x-2">
            <Input
              type="number"
              min={String(MIN_LIMIT)}
              max={String(MAX_LIMIT)}
              value={limitInput}
              onChange={(event) => setLimitInput(event.target.value)}
              onBlur={handleLimitBlur}
              onKeyDown={commitOnEnter}
              disabled={busy}
              className="w-20"
              aria-label={t("settings.debug.historyLimit.title")}
            />
            <span className="text-sm text-text">
              {t("settings.debug.historyLimit.entries")}
            </span>
          </div>
        </SettingContainer>
      )}

      {period === "custom_days" && (
        <SettingContainer
          title={t("settings.debug.recordingRetention.customDaysTitle")}
          description={t(
            "settings.debug.recordingRetention.customDaysDescription",
          )}
          descriptionMode="tooltip"
          grouped={grouped}
          layout="horizontal"
        >
          <div className="flex items-center space-x-2">
            <Input
              type="number"
              min={String(MIN_DAYS)}
              max={String(MAX_DAYS)}
              value={daysInput}
              onChange={(event) => setDaysInput(event.target.value)}
              onBlur={handleDaysBlur}
              onKeyDown={commitOnEnter}
              disabled={busy}
              className="w-20"
              aria-label={t(
                "settings.debug.recordingRetention.customDaysTitle",
              )}
            />
            <span className="text-sm text-text">
              {t("settings.debug.recordingRetention.days")}
            </span>
          </div>
        </SettingContainer>
      )}

      {pending && (
        <div
          className={grouped ? "px-4 py-3" : "px-4 py-3 rounded-xl"}
          role="alertdialog"
          aria-label={confirmHeading}
        >
          <div className="flex items-start gap-2.5 rounded-xl border border-error/30 bg-error/8 px-3 py-2.5">
            <AlertTriangle
              width={15}
              height={15}
              className="mt-0.5 shrink-0 text-error"
              aria-hidden="true"
            />
            <div className="min-w-0 flex-1">
              <p className="text-[13px] font-medium text-ink">
                {confirmHeading}
              </p>
              <p className="mt-0.5 text-xs leading-snug text-muted">
                {t("settings.debug.recordingRetention.confirmBody")}
              </p>
              <div className="mt-2.5 flex items-center gap-2">
                <Button size="sm" variant="danger" onClick={confirmPending}>
                  {pending.count === null
                    ? t(
                        "settings.debug.recordingRetention.confirmActionUnknown",
                      )
                    : t("settings.debug.recordingRetention.confirmAction", {
                        count: pending.count,
                      })}
                </Button>
                <Button size="sm" variant="secondary" onClick={cancelPending}>
                  {t("common.cancel")}
                </Button>
              </div>
            </div>
          </div>
        </div>
      )}
    </>
  );
};
