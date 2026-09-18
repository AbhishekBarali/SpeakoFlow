import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Mic, Pause, Play, Square } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { TONE_TILE_VIVID } from "@/components/ui/tones";
import type { MeetingSpeaker, MeetingState } from "./api";
import { formatClock, type TranscriptItem } from "./speakers";
import { SystemAudioNotice } from "./SystemAudioNotice";
import { TranscriptView } from "./TranscriptView";

interface RecorderCardProps {
  state: MeetingState;
  /** Which long-running call is in flight, so the buttons can say so. Starting
   *  opens two audio devices; stopping drains the whole Whisper queue. */
  busy: "starting" | "stopping" | null;
  liveItems: readonly TranscriptItem[];
  speakers: readonly MeetingSpeaker[];
  /** Platform help for a machine with no loopback source at all. */
  systemAudioHelp: string | null;
  systemAudioSupported: boolean;
  onStart: () => void;
  onStop: () => void;
  onTogglePause: () => void;
}

/**
 * The record button, and everything a recording needs on screen while it runs.
 *
 * The elapsed time ticks locally rather than waiting for the backend. State
 * events only arrive when something changes — a segment lands, the user pauses —
 * so a clock driven by them alone would sit still through a silence and look
 * frozen. `elapsed_ms` from the last event is the anchor; wall time since then is
 * added on top, and paused time is excluded because the backend's counter is the
 * microphone accumulator, which drops paused frames rather than buffering them.
 */
export const RecorderCard: React.FC<RecorderCardProps> = ({
  state,
  busy,
  liveItems,
  speakers,
  systemAudioHelp,
  systemAudioSupported,
  onStart,
  onStop,
  onTogglePause,
}) => {
  const { t } = useTranslation();
  const recording = state.meeting_id !== null;
  const paused = state.paused;

  const anchor = useRef({ elapsed: state.elapsed_ms, at: Date.now() });
  const [tick, setTick] = useState(() => Date.now());
  useEffect(() => {
    anchor.current = { elapsed: state.elapsed_ms, at: Date.now() };
    setTick(Date.now());
  }, [state.elapsed_ms, paused]);
  useEffect(() => {
    if (!recording || paused) return;
    const id = window.setInterval(() => setTick(Date.now()), 500);
    return () => window.clearInterval(id);
  }, [recording, paused]);

  const elapsedMs = paused
    ? anchor.current.elapsed
    : anchor.current.elapsed + Math.max(0, tick - anchor.current.at);

  // Before anything is recorded, the machine's own capability is the warning
  // worth showing; during a recording, the session's actual failure is.
  const showNotice = recording ? !state.system_audio : !systemAudioSupported;
  const noticeDetail = recording ? state.system_audio_error : systemAudioHelp;

  return (
    <div className="w-full space-y-3">
      {showNotice && (
        <SystemAudioNotice live={recording} detail={noticeDetail} />
      )}

      <div className="rounded-2xl border border-hairline bg-surface elev-card">
        <div className="flex items-center gap-3 px-4 py-3.5">
          <span
            className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-xl ${TONE_TILE_VIVID.rose}`}
          >
            <Mic size={18} />
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="text-[13.5px] font-medium text-ink">
                {recording
                  ? paused
                    ? t("meetings.recorder.paused")
                    : t("meetings.recorder.recording")
                  : t("meetings.recorder.idleTitle")}
              </span>
              {recording && !paused && (
                <span
                  className="h-1.5 w-1.5 rounded-full bg-error motion-safe:animate-pulse"
                  aria-hidden="true"
                />
              )}
            </div>
            <p className="mt-0.5 text-xs text-muted">
              {recording
                ? t("meetings.recorder.liveCaption")
                : t("meetings.recorder.idleCaption")}
            </p>
          </div>

          {recording && (
            <span className="shrink-0 rounded-lg bg-surface-strong px-2.5 py-1 font-mono text-[13px] tabular-nums text-ink">
              {formatClock(elapsedMs)}
            </span>
          )}

          <div className="flex shrink-0 items-center gap-2">
            {recording ? (
              <>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={onTogglePause}
                  disabled={busy !== null}
                  className="gap-1.5"
                >
                  {paused ? <Play size={13} /> : <Pause size={13} />}
                  {paused
                    ? t("meetings.recorder.resume")
                    : t("meetings.recorder.pause")}
                </Button>
                <Button
                  variant="danger"
                  size="sm"
                  onClick={onStop}
                  disabled={busy !== null}
                  className="gap-1.5"
                >
                  <Square size={12} />
                  {busy === "stopping"
                    ? t("meetings.recorder.stopping")
                    : t("meetings.recorder.stop")}
                </Button>
              </>
            ) : (
              <Button
                variant="primary"
                size="sm"
                onClick={onStart}
                disabled={busy !== null}
                className="gap-1.5"
              >
                <Mic size={13} />
                {busy === "starting"
                  ? t("meetings.recorder.starting")
                  : t("meetings.recorder.start")}
              </Button>
            )}
          </div>
        </div>

        {recording && (
          <div className="border-t border-hairline">
            <TranscriptView
              items={liveItems}
              speakers={speakers}
              stickToBottom
              heightClassName="max-h-[280px]"
              emptyLabel={t("meetings.recorder.transcriptEmpty")}
              resetKey={state.meeting_id ?? "idle"}
            />
          </div>
        )}
      </div>

      {busy === "stopping" && (
        <p className="px-1 text-xs text-muted">
          {t("meetings.recorder.stoppingHint")}
        </p>
      )}
    </div>
  );
};
