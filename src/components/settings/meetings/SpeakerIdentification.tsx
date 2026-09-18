import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { Download, Users } from "lucide-react";
import { Button } from "@/components/ui/Button";
import {
  diarizeMeeting,
  downloadDiarizationModel,
  getDiarizationStatus,
  MEETING_DIARIZATION_PROGRESS_EVENT,
  type DiarizationProgress,
  type DiarizationSkipReason,
} from "./api";

interface SpeakerIdentificationProps {
  meetingId: number;
  /** Already labelled, so there is nothing to offer. */
  diarized: boolean;
  /** No far-side audio was captured, so there is nobody to tell apart. */
  hasSystemAudio: boolean;
  /** Still recording: the WAV has no finalised header and cannot be decoded. */
  isLive: boolean;
  /** Refetch the transcript once labels land. */
  onLabelled: () => void;
}

/**
 * Offer to identify who said what on the far side of a meeting.
 *
 * Normally this never appears: diarization runs automatically when a call ends.
 * It shows up in exactly two situations, and both are worth an explicit
 * affordance rather than silence:
 *
 * - the 27 MB speaker model is not installed yet, so the automatic pass skipped;
 * - it ran and something went wrong, so a retry is the only way forward.
 *
 * The download is a separate step from recording on purpose. A first meeting must
 * not stall behind a model nobody asked for, and someone who only ever talks to
 * one person on a call never needs it.
 */
export const SpeakerIdentification: React.FC<SpeakerIdentificationProps> = ({
  meetingId,
  diarized,
  hasSystemAudio,
  isLive,
  onLabelled,
}) => {
  const { t } = useTranslation();
  const [installed, setInstalled] = useState<boolean | null>(null);
  const [downloadMb, setDownloadMb] = useState(27);
  const [busy, setBusy] = useState<"downloading" | "running" | null>(null);
  const [progress, setProgress] = useState<DiarizationProgress | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const refresh = useCallback(() => {
    void getDiarizationStatus()
      .then((status) => {
        setInstalled(status.installed);
        setDownloadMb(status.download_mb);
      })
      .catch(() => setInstalled(null));
  }, []);

  useEffect(refresh, [refresh]);

  useEffect(() => {
    const unlisten = listen<DiarizationProgress>(
      MEETING_DIARIZATION_PROGRESS_EVENT,
      (event) => {
        if (event.payload.meeting_id !== meetingId) return;
        setProgress(event.payload);
      },
    );
    return () => {
      void unlisten.then((off) => off());
    };
  }, [meetingId]);

  const run = () => {
    setBusy("running");
    setMessage(null);
    setProgress(null);
    void diarizeMeeting(meetingId)
      .then((outcome) => {
        switch (outcome.outcome) {
          case "labelled":
            onLabelled();
            break;
          case "one_speaker":
            setMessage(t("meetings.diarize.oneSpeaker"));
            onLabelled();
            break;
          case "skipped":
            setMessage(skipMessage(outcome.reason, t));
            refresh();
            break;
        }
      })
      .catch((error: unknown) => setMessage(String(error)))
      .finally(() => {
        setBusy(null);
        setProgress(null);
      });
  };

  const install = () => {
    setBusy("downloading");
    setMessage(null);
    void downloadDiarizationModel()
      .then(() => {
        setInstalled(true);
        // Straight into the pass: the user asked for speaker labels, not for a
        // download, so making them press a second button would be the app
        // reporting its own implementation steps.
        run();
      })
      .catch((error: unknown) => {
        setMessage(String(error));
        setBusy(null);
      });
  };

  // Nothing to offer: already labelled, still recording, or there was never a far
  // side to tell apart.
  if (diarized || isLive || !hasSystemAudio || installed === null) return null;

  return (
    <div className="rounded-xl border border-hairline bg-surface-strong/60 px-3.5 py-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-1.5 text-[12.5px] font-medium text-ink">
            <Users size={13} />
            {t("meetings.diarize.title")}
          </p>
          <p className="mt-0.5 text-[11.5px] text-muted">
            {busy === "running" && progress && progress.total > 0
              ? t("meetings.diarize.progress", {
                  done: progress.done,
                  total: progress.total,
                })
              : installed
                ? t("meetings.diarize.ready")
                : t("meetings.diarize.needsModel", { size: downloadMb })}
          </p>
        </div>
        <Button
          variant="secondary"
          size="sm"
          onClick={installed ? run : install}
          disabled={busy !== null}
          className="gap-1.5"
        >
          {installed ? <Users size={13} /> : <Download size={13} />}
          {busy === "downloading"
            ? t("meetings.diarize.downloading")
            : busy === "running"
              ? t("meetings.diarize.running")
              : installed
                ? t("meetings.diarize.identify")
                : t("meetings.diarize.install")}
        </Button>
      </div>
      {message && (
        <p className="mt-2 text-[11.5px] text-muted-soft">{message}</p>
      )}
    </div>
  );
};

/** A sentence per skip reason, because a generic failure gives no next step. */
const skipMessage = (
  reason: DiarizationSkipReason,
  t: (key: string) => string,
): string => {
  switch (reason) {
    case "model_not_installed":
      return t("meetings.diarize.skipped.modelNotInstalled");
    case "no_system_audio":
      return t("meetings.diarize.skipped.noSystemAudio");
    case "audio_unreadable":
      return t("meetings.diarize.skipped.audioUnreadable");
    case "too_few_segments":
      return t("meetings.diarize.skipped.tooFewSegments");
    case "already_diarized":
      return t("meetings.diarize.skipped.alreadyDone");
    case "model_unusable":
      return t("meetings.diarize.skipped.modelUnusable");
  }
};
