import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { Eye, EyeOff, Pause, Play, Square } from "lucide-react";
import meetingsArt from "@/assets/hero/meetings.webp";
import { SectionTitle } from "@/components/ui/Page";
import {
  MEETING_LEVEL_EVENT,
  type MeetingLevels,
  type MeetingSpeaker,
  type MeetingState,
} from "./api";
import { useMeetingIndicator } from "./CallDetectionToggle";
import { formatClock, type TranscriptItem } from "./speakers";
import { SystemAudioNotice } from "./SystemAudioNotice";
import { TranscriptView } from "./TranscriptView";

interface RecorderCardProps {
  state: MeetingState;
  /** Which long-running call is in flight, so the buttons can say so. Stopping
   *  drains the whole transcription queue. */
  busy: "starting" | "stopping" | null;
  liveItems: readonly TranscriptItem[];
  speakers: readonly MeetingSpeaker[];
  onStop: () => void;
  onTogglePause: () => void;
}

/** Below this a stream counts as silent. Matches the pill. */
const SILENT_LEVEL = 0.02;

/**
 * A meeting that is recording right now, in the main window.
 *
 * The page's banner becomes the recorder: the same dark stage and the same
 * light, now carrying the clock, which side is audible, and the two things you
 * can do (stop, pause). Starting a meeting used to replace the banner with a
 * plain white card, so the moment the page mattered most it looked least like
 * the rest of the app. The live transcript sits underneath as an ordinary
 * section.
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
  onStop,
  onTogglePause,
}) => {
  const { t } = useTranslation();
  const indicator = useMeetingIndicator();
  const paused = state.paused;

  const anchor = useRef({ elapsed: state.elapsed_ms, at: Date.now() });
  const [tick, setTick] = useState(() => Date.now());
  useEffect(() => {
    anchor.current = { elapsed: state.elapsed_ms, at: Date.now() };
    setTick(Date.now());
  }, [state.elapsed_ms, paused]);
  useEffect(() => {
    if (paused) return;
    const id = window.setInterval(() => setTick(Date.now()), 500);
    return () => window.clearInterval(id);
  }, [paused]);

  const elapsedMs = paused
    ? anchor.current.elapsed
    : anchor.current.elapsed + Math.max(0, tick - anchor.current.at);
  const clock = formatClock(elapsedMs);
  const statusLabel = paused
    ? t("meetings.recorder.paused")
    : t("meetings.recorder.recording");

  return (
    <div className="w-full">
      <section
        className="hero-stage rounded-[1.25rem]"
        data-paused={String(paused)}
        aria-label={`${statusLabel} ${clock}`}
      >
        <img
          src={meetingsArt}
          alt=""
          aria-hidden="true"
          draggable={false}
          decoding="async"
          className="hero-art"
        />
        <div className="relative flex min-h-[10.5rem] flex-col justify-center px-7 py-7 @3xl:max-w-[58%] @3xl:px-9 @3xl:py-8">
          <p
            className="flex items-center gap-2.5 text-[0.8125rem] font-medium text-hero-muted"
            role="status"
          >
            <span
              className="rec-dot"
              data-paused={String(paused)}
              aria-hidden="true"
            />
            {statusLabel}
          </p>
          <p
            className="rec-clock mt-3"
            data-paused={String(paused)}
            aria-hidden="true"
          >
            {clock}
          </p>
          <SideLights paused={paused} systemCaptured={state.system_audio} />
          <div className="flex flex-wrap items-center gap-2.5 pt-6">
            <button
              type="button"
              onClick={onStop}
              disabled={busy !== null}
              className="hero-button inline-flex h-11 cursor-pointer items-center justify-center gap-2 rounded-full ps-5 pe-6 text-[0.9375rem] font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-hero-ink focus-visible:ring-offset-2 focus-visible:ring-offset-hero-surface disabled:cursor-wait disabled:opacity-70"
            >
              <Square
                className="h-3.5 w-3.5 fill-[#e5484d] text-[#e5484d]"
                aria-hidden="true"
              />
              {busy === "stopping"
                ? t("meetings.recorder.stopping")
                : t("meetings.pill.stop")}
            </button>
            <button
              type="button"
              onClick={onTogglePause}
              disabled={busy !== null}
              className="hero-glass inline-flex h-11 cursor-pointer items-center justify-center gap-2 rounded-full ps-4 pe-5 text-[0.9375rem] font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-hero-ink/70 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {paused ? (
                <Play className="h-4 w-4" aria-hidden="true" />
              ) : (
                <Pause className="h-4 w-4" aria-hidden="true" />
              )}
              {paused
                ? t("meetings.recorder.resume")
                : t("meetings.recorder.pause")}
            </button>
          </div>
        </div>

        {indicator.enabled !== null && (
          // The floating pill, from the one place that is still on screen when
          // it has been switched off. Where the banner keeps its tips button.
          <button
            type="button"
            onClick={() => indicator.set(!indicator.enabled)}
            disabled={indicator.saving}
            title={
              indicator.enabled
                ? t("meetings.indicator.hide")
                : t("meetings.indicator.show")
            }
            aria-label={
              indicator.enabled
                ? t("meetings.indicator.hide")
                : t("meetings.indicator.show")
            }
            aria-pressed={indicator.enabled}
            className="hero-corner hero-corner-plain absolute end-3.5 top-3.5 grid h-8 w-8 cursor-pointer place-items-center rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-hero-ink/70 disabled:cursor-wait"
          >
            {indicator.enabled ? (
              <Eye className="h-4 w-4" aria-hidden="true" />
            ) : (
              <EyeOff className="h-4 w-4" aria-hidden="true" />
            )}
          </button>
        )}
      </section>

      {busy === "stopping" && (
        <p className="mt-3 px-1 text-sm text-muted" role="status">
          {t("meetings.recorder.stoppingHint")}
        </p>
      )}

      {/* During a recording the session's actual failure is the warning worth
          showing, right under the thing it is about. */}
      {!state.system_audio && (
        <div className="mt-4">
          <SystemAudioNotice live detail={state.system_audio_error} />
        </div>
      )}

      <section className="mt-8">
        <SectionTitle title={t("meetingsPage.transcript")} />
        <div className="overflow-hidden rounded-2xl border border-hairline bg-surface elev-card">
          <TranscriptView
            items={liveItems}
            speakers={speakers}
            stickToBottom
            heightClassName="max-h-[26rem]"
            emptyLabel={t("meetings.recorder.transcriptEmpty")}
            resetKey={state.meeting_id ?? "idle"}
          />
        </div>
      </section>
    </div>
  );
};

/**
 * "You" and "Others", each lit while that side is audible.
 *
 * Its own component so the ~15 level events a second re-render two chips
 * rather than the whole recorder and the transcript under it. Levels stop
 * arriving the instant capture stops, which would leave a light frozen on, so
 * they decay to zero when nothing has arrived for a moment.
 */
const SideLights: React.FC<{ paused: boolean; systemCaptured: boolean }> = ({
  paused,
  systemCaptured,
}) => {
  const { t } = useTranslation();
  const [levels, setLevels] = useState<MeetingLevels>({ mic: 0, system: 0 });

  useEffect(() => {
    const unlisten = listen<MeetingLevels>(MEETING_LEVEL_EVENT, (event) => {
      setLevels(event.payload);
    });
    const decay = window.setInterval(() => {
      setLevels((current) =>
        current.mic === 0 && current.system === 0
          ? current
          : { mic: current.mic * 0.6, system: current.system * 0.6 },
      );
    }, 400);
    return () => {
      window.clearInterval(decay);
      void unlisten.then((off) => off());
    };
  }, []);

  const side = (label: string, level: number, captured: boolean) => (
    <span
      className="rec-side"
      data-captured={String(captured)}
      data-active={String(!paused && captured && level >= SILENT_LEVEL)}
    >
      <span className="rec-side-light" aria-hidden="true" />
      {label}
    </span>
  );

  return (
    <div className="mt-5 flex flex-wrap items-center gap-2" aria-hidden="true">
      {side(t("meetings.speakers.me"), levels.mic, true)}
      {side(t("meetings.speakers.others"), levels.system, systemCaptured)}
    </div>
  );
};
