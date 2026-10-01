import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { Mic } from "lucide-react";
import { PageHeader, SectionTitle } from "@/components/ui/Page";
import Badge from "@/components/ui/Badge";
import { Hero, HeroTitle } from "@/components/ui/Hero";
import { usePageReset } from "@/components/shell/navigation";
import {
  deleteMeeting,
  getMeetingSpeakers,
  getMeetingState,
  getSystemAudioStatus,
  listMeetings,
  MEETING_PAGE_SIZE,
  MEETING_SEGMENT_EVENT,
  MEETING_STATE_EVENT,
  MEETINGS_UPDATED_EVENT,
  setMeetingPaused,
  startMeeting,
  stopMeeting,
  type Meeting,
  type MeetingSpeaker,
  type MeetingState,
  type SegmentEvent,
  liveMeetingId,
} from "./api";
import { itemFromEvent, type TranscriptItem } from "./speakers";
import { SystemAudioNotice } from "./SystemAudioNotice";
import { MeetingDetail } from "./MeetingDetail";
import { MeetingsList } from "./MeetingsList";
import { RecorderCard } from "./RecorderCard";

const IDLE_STATE: MeetingState = {
  meeting_id: null,
  status: "complete",
  paused: false,
  system_audio: false,
  system_audio_error: null,
  elapsed_ms: 0,
  dropped_chunks: 0,
};

/**
 * The Meetings section: record a call, read it back split by speaker, turn it
 * into notes.
 *
 * This component owns the two live subscriptions for the whole section, because
 * both the recorder card and an open detail page need them and duplicating the
 * listeners would mean two components disagreeing about whether a meeting is
 * running.
 */
export const MeetingsSection: React.FC<{
  /** Rows under the banner (models, switches), shown while nothing records. */
  settings?: React.ReactNode;
}> = ({ settings }) => {
  const { t, i18n } = useTranslation();

  const [state, setState] = useState<MeetingState>(IDLE_STATE);
  const [busy, setBusy] = useState<"starting" | "stopping" | null>(null);
  const [liveItems, setLiveItems] = useState<TranscriptItem[]>([]);

  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(true);
  const [pages, setPages] = useState(1);
  const [speakerCounts, setSpeakerCounts] = useState<Map<number, number>>(
    new Map(),
  );

  const [systemAudioSupported, setSystemAudioSupported] = useState(true);
  const [systemAudioHelp, setSystemAudioHelp] = useState<string | null>(null);

  const [openId, setOpenId] = useState<number | null>(null);
  // Clicking Meetings in the sidebar again goes back to the list.
  usePageReset("meetings", () => setOpenId(null));

  /* ── the list ── */

  const loadList = useCallback(
    async (pageCount: number) => {
      setLoading(true);
      try {
        // One request per page rather than one growing request. The backend
        // clamps any limit to 500, so asking for "everything loaded so far" in a
        // single call silently stops growing past page 17 and leaves a Load more
        // button that does nothing.
        const collected: Meeting[] = [];
        let more = false;
        for (let page = 0; page < pageCount; page += 1) {
          const result = await listMeetings(
            MEETING_PAGE_SIZE,
            page * MEETING_PAGE_SIZE,
          );
          collected.push(...result.meetings);
          more = result.has_more;
          if (!more) break;
        }
        setMeetings(collected);
        setHasMore(more);
      } catch (error) {
        toast.error(t("meetings.errors.loadFailed", { error: String(error) }));
      } finally {
        setLoading(false);
      }
    },
    [t],
  );

  useEffect(() => {
    void loadList(pages);
  }, [loadList, pages]);

  // Speaker counts are a second pass rather than part of the list query: the
  // list renders immediately, and a page's worth of one-row lookups is cheaper
  // than making every meeting wait for a join it mostly does not need.
  const requested = useRef<Set<number>>(new Set());
  useEffect(() => {
    const missing = meetings
      .map((meeting) => meeting.id)
      .filter((id) => !requested.current.has(id));
    if (missing.length === 0) return;
    for (const id of missing) requested.current.add(id);

    let cancelled = false;
    void Promise.all(
      missing.map((id) =>
        getMeetingSpeakers(id)
          .then((speakers: MeetingSpeaker[]) => [id, speakers.length] as const)
          .catch(() => null),
      ),
    ).then((results) => {
      if (cancelled) return;
      setSpeakerCounts((current) => {
        const next = new Map(current);
        for (const entry of results) {
          if (entry) next.set(entry[0], entry[1]);
        }
        return next;
      });
    });
    return () => {
      cancelled = true;
    };
  }, [meetings]);

  const refresh = useCallback(() => {
    void loadList(pages);
  }, [loadList, pages]);

  /* ── live state ── */

  useEffect(() => {
    // Ask rather than wait: the section can be opened mid-meeting, and the next
    // state event might be minutes away.
    void getMeetingState()
      .then(setState)
      .catch(() => {});
    void getSystemAudioStatus()
      .then((status) => {
        setSystemAudioSupported(status.supported);
        setSystemAudioHelp(status.help);
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    const unlisten = listen<MeetingState>(MEETING_STATE_EVENT, (event) => {
      setState(event.payload);
      // A recording that just ended has nothing live left to show, and its
      // transcript is now readable from the database.
      if (liveMeetingId(event.payload) === null) setLiveItems([]);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  useEffect(() => {
    const unlisten = listen<SegmentEvent>(MEETING_SEGMENT_EVENT, (event) => {
      setLiveItems((current) => [...current, itemFromEvent(event.payload)]);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  useEffect(() => {
    const unlisten = listen(MEETINGS_UPDATED_EVENT, () => {
      refresh();
      // A row that changed may have gained speakers (a diarization pass), so the
      // cached counts for it are no longer trustworthy.
      requested.current.clear();
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refresh]);

  /* ── actions ── */

  const start = () => {
    setBusy("starting");
    // The title is composed here so the default is localised — Rust falls back
    // to a bare timestamp precisely because it must not invent English.
    const when = new Intl.DateTimeFormat(i18n.language, {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date());
    void startMeeting(t("meetings.newTitle", { when }))
      .then(() => {
        setLiveItems([]);
        return getMeetingState().then(setState);
      })
      .then(refresh)
      .catch((error: unknown) => {
        toast.error(t("meetings.errors.startFailed", { error: String(error) }));
      })
      .finally(() => setBusy(null));
  };

  const stop = () => {
    setBusy("stopping");
    void stopMeeting()
      .then((meetingId) => {
        setLiveItems([]);
        setState(IDLE_STATE);
        // Open what was just recorded: the transcript is complete by the time
        // `stop_meeting` answers, so there is something to read immediately.
        setOpenId(meetingId);
      })
      .catch((error: unknown) => {
        toast.error(t("meetings.errors.stopFailed", { error: String(error) }));
      })
      .finally(() => {
        setBusy(null);
        void getMeetingState()
          .then(setState)
          .catch(() => {});
      });
  };

  const togglePause = () => {
    const next = !state.paused;
    void setMeetingPaused(next)
      .then(() => setState((current) => ({ ...current, paused: next })))
      .catch((error: unknown) => {
        toast.error(t("meetings.errors.pauseFailed", { error: String(error) }));
      });
  };

  const remove = (meetingId: number) => {
    void deleteMeeting(meetingId)
      .then(() => {
        if (openId === meetingId) setOpenId(null);
        setMeetings((current) => current.filter((m) => m.id !== meetingId));
      })
      .catch((error: unknown) => {
        toast.error(
          t("meetings.errors.deleteFailed", { error: String(error) }),
        );
      });
  };

  if (openId !== null) {
    return (
      <MeetingDetail
        meetingId={openId}
        recordingMeetingId={liveMeetingId(state)}
        onChanged={refresh}
        onBack={() => setOpenId(null)}
        backLabel={t("nav.meetings")}
      />
    );
  }

  const recording = liveMeetingId(state) !== null;

  return (
    <div className="w-full">
      <PageHeader
        title={t("sidebar.meetings")}
        badge={<Badge variant="outline">{t("common.beta")}</Badge>}
        description={t("sectionSubtitles.meetings")}
      />

      {recording ? (
        <RecorderCard
          state={state}
          busy={busy}
          liveItems={liveItems}
          // No speaker names live: the only keys that exist during a recording
          // are the two channel-derived ones, whose stored names are the
          // English seeds, and the transcript view translates those itself.
          speakers={[]}
          onStop={stop}
          onTogglePause={togglePause}
        />
      ) : (
        <>
          {/* Before anything is recorded, the machine's own capability is the
              warning worth showing. */}
          {!systemAudioSupported && (
            <div className="mb-5">
              <SystemAudioNotice live={false} detail={systemAudioHelp} />
            </div>
          )}
          <Hero
            art="meetings"
            title={<HeroTitle i18nKey="meetingsPage.hero.title" />}
            subtitle={t("meetingsPage.hero.subtitle")}
            actions={
              <button
                type="button"
                onClick={start}
                disabled={busy !== null}
                className="hero-button inline-flex h-11 cursor-pointer items-center justify-center gap-2 rounded-full ps-5 pe-6 text-[0.9375rem] font-semibold focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-hero-ink focus-visible:ring-offset-2 focus-visible:ring-offset-hero-surface disabled:cursor-wait disabled:opacity-70"
              >
                <Mic className="h-4 w-4" aria-hidden="true" />
                {busy === "starting"
                  ? t("meetings.recorder.starting")
                  : t("meetings.recorder.start")}
              </button>
            }
          />
          {settings && <div className="mt-6">{settings}</div>}
        </>
      )}

      <section className="mt-10">
        <SectionTitle title={t("meetings.list.title")} />
        <MeetingsList
          meetings={meetings}
          speakerCounts={speakerCounts}
          loading={loading}
          hasMore={hasMore}
          onLoadMore={() => setPages((value) => value + 1)}
          onOpen={setOpenId}
          onDelete={remove}
          recordingMeetingId={liveMeetingId(state)}
        />
      </section>
    </div>
  );
};
