import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight, Sparkles, Trash2, Users } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { TONE_PILL } from "@/components/ui/tones";
import { formatDateTime } from "@/utils/dateFormat";
import type { Meeting, MeetingStatus } from "./api";
import { formatClock, meetingDurationMs } from "./speakers";

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
const STATUS_TONE: Partial<Record<MeetingStatus, keyof typeof TONE_PILL>> = {
  recording: "rose",
  processing: "amber",
  interrupted: "amber",
};

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
  const [confirmingId, setConfirmingId] = useState<number | null>(null);

  if (loading && meetings.length === 0) {
    return (
      <div className="px-4 py-10 text-center text-[13px] text-muted">
        {t("common.loading")}
      </div>
    );
  }

  if (meetings.length === 0) {
    return (
      <div className="flex flex-col items-center gap-2 px-6 py-12 text-center">
        <span className="flex h-10 w-10 items-center justify-center rounded-xl bg-surface-strong text-muted-soft">
          <Users size={18} />
        </span>
        <p className="text-[13.5px] font-medium text-ink">
          {t("meetings.list.emptyTitle")}
        </p>
        <p className="max-w-sm text-[12.5px] leading-relaxed text-muted">
          {t("meetings.list.emptyHint")}
        </p>
      </div>
    );
  }

  return (
    <>
      <ul className="divide-y divide-hairline">
        {meetings.map((meeting) => {
          const durationMs = meetingDurationMs(
            meeting.started_at,
            meeting.ended_at,
          );
          const speakerCount = speakerCounts.get(meeting.id);
          // Assembled in JS rather than as JSX text so the separator is not a
          // literal string in markup, and so an unknown value drops out of the
          // line instead of rendering as an em dash nobody can interpret.
          const meta = [
            formatDateTime(String(meeting.started_at), i18n.language),
            durationMs === null ? null : formatClock(durationMs),
            speakerCount === undefined
              ? null
              : t("meetings.list.speakers", { count: speakerCount }),
          ]
            .filter((part): part is string => Boolean(part))
            .join(" · ");
          const tone = STATUS_TONE[meeting.status];
          const confirming = confirmingId === meeting.id;
          const isRecording = recordingMeetingId === meeting.id;

          return (
            <li
              key={meeting.id}
              className="flex items-center gap-2 px-3 py-2.5"
            >
              <button
                type="button"
                onClick={() => onOpen(meeting.id)}
                className="flex min-w-0 flex-1 items-center gap-2 rounded-lg px-1 py-1 text-start transition-colors hover:bg-ink/4 cursor-pointer"
              >
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13px] text-ink">
                    {meeting.title}
                  </span>
                  <span className="mt-0.5 block truncate text-[11.5px] text-muted">
                    {meta}
                  </span>
                </span>
                {meeting.notes && (
                  <span
                    className="shrink-0 text-accent"
                    title={t("meetings.list.hasNotes")}
                  >
                    <Sparkles size={13} />
                  </span>
                )}
                {tone && (
                  <span
                    className={`shrink-0 rounded-md border px-2 py-0.5 text-[10.5px] font-semibold uppercase tracking-wide ${TONE_PILL[tone]}`}
                  >
                    {t(`meetings.status.${meeting.status}`)}
                  </span>
                )}
                <ChevronRight
                  size={15}
                  className="shrink-0 text-muted-soft"
                  aria-hidden="true"
                />
              </button>

              {confirming ? (
                <span className="flex shrink-0 items-center gap-1.5">
                  <Button
                    variant="danger"
                    size="sm"
                    onClick={() => {
                      setConfirmingId(null);
                      onDelete(meeting.id);
                    }}
                  >
                    {t("common.delete")}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setConfirmingId(null)}
                  >
                    {t("common.cancel")}
                  </Button>
                </span>
              ) : (
                <button
                  type="button"
                  onClick={() => setConfirmingId(meeting.id)}
                  disabled={isRecording}
                  title={
                    isRecording
                      ? t("meetings.delete.whileRecording")
                      : t("meetings.delete.action")
                  }
                  aria-label={t("meetings.delete.action")}
                  className="shrink-0 cursor-pointer rounded-md p-1.5 text-muted transition-colors hover:bg-error/10 hover:text-error disabled:cursor-not-allowed disabled:text-muted-soft/50 disabled:hover:bg-transparent"
                >
                  <Trash2 size={15} />
                </button>
              )}
            </li>
          );
        })}
      </ul>

      {hasMore && (
        <div className="flex justify-center border-t border-hairline px-4 py-3">
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
    </>
  );
};
