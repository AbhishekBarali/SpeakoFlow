import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AudioLines,
  ChevronRight,
  MoreHorizontal,
  Trash2,
  Users,
} from "lucide-react";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { MenuButton, type MenuItem } from "@/components/ui/Menu";
import { formatTimeOfDay, groupByDay } from "@/utils/dayGroups";
import type { Meeting, MeetingStatus } from "./api";
import { formatDuration, meetingDurationMs } from "./speakers";

interface MeetingsListProps {
  meetings: readonly Meeting[];
  /** Speakers known per meeting, filled in a second pass so the list can render
   *  immediately rather than waiting on it. */
  speakerCounts: ReadonlyMap<number, number>;
  loading: boolean;
  hasMore: boolean;
  onLoadMore: () => void;
  onOpen: (meetingId: number) => void;
  onDelete: (meetingId: number) => void;
  /** The meeting recording right now, which cannot be deleted — capture threads
   *  would keep writing into a row and files that no longer exist. */
  recordingMeetingId: number | null;
}

/** Only states worth a badge get one; "complete" is the normal case and a pill
 *  on every row is noise. */
const STATUS_PILL: Partial<Record<MeetingStatus, string>> = {
  recording: "border-error/30 bg-error/10 text-error",
  processing:
    "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  interrupted:
    "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300",
};

const ICON_BUTTON =
  "grid h-8 w-8 cursor-pointer place-items-center rounded-lg text-muted transition-colors hover:bg-ink/6 hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 aria-expanded:bg-ink/6 aria-expanded:text-ink";

/**
 * Recorded meetings, newest first, grouped by day exactly like History: a
 * day heading, then one card of rows for that day. A row is the title and one
 * line of facts (time, length, who spoke); opening it is
 * the whole row, and the rare actions sit behind a ⋯ that shows on hover.
 * There is no "Notes" marker: a meeting with notes already shows their summary
 * as its title, so the marker only repeated what the row said. */
export const MeetingsList: React.FC<MeetingsListProps> = ({
  meetings,
  speakerCounts,
  loading,
  hasMore,
  onLoadMore,
  onOpen,
  onDelete,
  recordingMeetingId,
}) => {
  const { t, i18n } = useTranslation();
  const [confirming, setConfirming] = useState<Meeting | null>(null);

  const groups = useMemo(
    () =>
      groupByDay(meetings, (meeting) => meeting.started_at, i18n.language, {
        today: t("historyPage.today"),
        yesterday: t("historyPage.yesterday"),
      }),
    [meetings, i18n.language, t],
  );

  if (loading && meetings.length === 0) {
    return (
      <div className="space-y-3" aria-busy="true">
        {[0, 1].map((index) => (
          <div
            key={index}
            className="h-[4.25rem] animate-pulse rounded-xl bg-surface-strong/60"
          />
        ))}
        <span className="sr-only">{t("common.loading")}</span>
      </div>
    );
  }

  if (meetings.length === 0) {
    return (
      <div className="flex flex-col items-center rounded-2xl border border-dashed border-hairline-strong px-6 py-12 text-center">
        <span className="grid h-11 w-11 place-items-center rounded-xl bg-surface-strong text-muted">
          <AudioLines className="h-5 w-5" aria-hidden="true" />
        </span>
        <p className="mt-4 text-[0.9375rem] font-medium text-ink">
          {t("meetings.list.emptyTitle")}
        </p>
        <p className="mt-1.5 max-w-sm text-sm leading-relaxed text-muted text-pretty">
          {t("meetings.list.emptyHint")}
        </p>
      </div>
    );
  }

  return (
    <>
      <div className="space-y-6">
        {groups.map((group) => (
          <section key={group.key} aria-label={group.label}>
            <h3 className="sticky top-0 z-10 -mx-1 mb-2 bg-canvas/95 px-1 py-1.5 text-sm font-semibold text-muted backdrop-blur-[2px]">
              {group.label}
            </h3>
            <ul className="divide-y divide-hairline overflow-hidden rounded-xl border border-hairline bg-surface elev-card">
              {group.items.map((meeting) => (
                <MeetingRow
                  key={meeting.id}
                  meeting={meeting}
                  speakerCount={speakerCounts.get(meeting.id)}
                  isRecording={recordingMeetingId === meeting.id}
                  onOpen={() => onOpen(meeting.id)}
                  onDelete={() => setConfirming(meeting)}
                />
              ))}
            </ul>
          </section>
        ))}
      </div>

      {hasMore && (
        <div className="mt-5 flex justify-center">
          <Button
            variant="secondary"
            size="sm"
            onClick={onLoadMore}
            disabled={loading}
          >
            {loading ? t("common.loading") : t("meetings.list.loadMore")}
          </Button>
        </div>
      )}

      <Dialog
        open={confirming !== null}
        onClose={() => setConfirming(null)}
        size="sm"
        title={t("meetings.delete.confirmTitle")}
        description={confirming?.title}
        footer={
          <>
            <Button
              variant="ghost"
              size="sm"
              className="ms-auto"
              onClick={() => setConfirming(null)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              size="sm"
              onClick={() => {
                if (confirming) onDelete(confirming.id);
                setConfirming(null);
              }}
            >
              {t("common.delete")}
            </Button>
          </>
        }
      >
        <p className="text-sm leading-relaxed text-body">
          {t("meetings.delete.confirmBody")}
        </p>
      </Dialog>
    </>
  );
};

interface MeetingRowProps {
  meeting: Meeting;
  speakerCount: number | undefined;
  isRecording: boolean;
  onOpen: () => void;
  onDelete: () => void;
}

const MeetingRow: React.FC<MeetingRowProps> = ({
  meeting,
  speakerCount,
  isRecording,
  onOpen,
  onDelete,
}) => {
  const { t, i18n } = useTranslation();
  const durationMs = meetingDurationMs(meeting.started_at, meeting.ended_at);
  const pill = STATUS_PILL[meeting.status];

  const menuItems: MenuItem[] = [
    {
      id: "open",
      label: t("common.open"),
      icon: ChevronRight,
      onSelect: onOpen,
    },
    {
      id: "delete",
      label: t("meetings.delete.action"),
      hint: isRecording ? t("meetings.delete.whileRecording") : undefined,
      icon: Trash2,
      tone: "danger",
      separated: true,
      disabled: isRecording,
      onSelect: onDelete,
    },
  ];

  return (
    <li className="group relative flex items-center gap-2 pe-2 transition-colors hover:bg-surface-muted/70">
      <button
        type="button"
        onClick={onOpen}
        className="min-w-0 flex-1 cursor-pointer py-3.5 ps-4 text-start focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent/40"
      >
        <span className="flex min-w-0 items-center gap-2">
          {isRecording && (
            <span
              className="h-2 w-2 shrink-0 rounded-full bg-error motion-safe:animate-pulse"
              aria-hidden="true"
            />
          )}
          <span className="truncate text-sm font-medium text-ink">
            {meeting.title}
          </span>
        </span>
        <span className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted">
          <span className="tabular-nums">
            {formatTimeOfDay(meeting.started_at, i18n.language)}
          </span>
          {durationMs !== null && (
            <>
              <Dot />
              <span className="tabular-nums">
                {formatDuration(durationMs, i18n.language)}
              </span>
            </>
          )}
          {speakerCount !== undefined && speakerCount > 0 && (
            <>
              <Dot />
              <span className="inline-flex items-center gap-1">
                <Users width={11} height={11} aria-hidden="true" />
                {t("meetings.list.speakers", { count: speakerCount })}
              </span>
            </>
          )}
        </span>
      </button>

      {pill && (
        <span
          className={`shrink-0 rounded-full border px-2 py-0.5 text-[0.6875rem] font-medium ${pill}`}
        >
          {t(`meetings.status.${meeting.status}`)}
        </span>
      )}

      <div className="shrink-0 opacity-0 transition-opacity duration-150 group-hover:opacity-100 focus-within:opacity-100 has-[[aria-expanded=true]]:opacity-100">
        <MenuButton
          items={menuItems}
          width={240}
          ariaLabel={t("common.more")}
          title={t("common.more")}
          className={ICON_BUTTON}
        >
          <MoreHorizontal width={16} height={16} />
        </MenuButton>
      </div>
    </li>
  );
};

const Dot: React.FC = () => (
  <span aria-hidden="true" className="text-muted-soft">
    ·
  </span>
);
