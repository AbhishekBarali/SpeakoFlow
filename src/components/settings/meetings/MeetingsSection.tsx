import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { SubPage } from "@/components/ui/SubPage";
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
} from "./api";
import { itemFromEvent, type TranscriptItem } from "./speakers";
import { CallDetectionToggle } from "./CallDetectionToggle";
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
export const MeetingsSection: React.FC = () => {
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
      if (event.payload.meeting_id === null) setLiveItems([]);
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
      <SubPage
        title={t("meetings.detail.title")}
        onBack={() => setOpenId(null)}
      >
        <MeetingDetail
          meetingId={openId}
          recordingMeetingId={state.meeting_id}
          onChanged={refresh}
        />
      </SubPage>
    );
  }

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SectionHeader
        title={t("sidebar.meetings")}
        description={t("sectionSubtitles.meetings")}
      />

      <RecorderCard
        state={state}
        busy={busy}
        liveItems={liveItems}
        // No speaker names live: the only keys that exist during a recording are
        // the two channel-derived ones, whose stored names are the English seeds,
        // and the transcript view translates those itself. Diarization — the pass
        // that produces real speakers to name — runs after the meeting ends.
        speakers={[]}
        systemAudioHelp={systemAudioHelp}
        systemAudioSupported={systemAudioSupported}
        onStart={start}
        onStop={stop}
        onTogglePause={togglePause}
      />

      <CallDetectionToggle />

      <div className="space-y-2">
        <h2 className="px-1 text-[13.5px] font-semibold tracking-tight text-ink">
          {t("meetings.list.title")}
        </h2>
        <div className="rounded-2xl border border-hairline bg-surface elev-card overflow-hidden">
          <MeetingsList
            meetings={meetings}
            speakerCounts={speakerCounts}
            loading={loading}
            hasMore={hasMore}
            onLoadMore={() => setPages((value) => value + 1)}
            onOpen={setOpenId}
            onDelete={remove}
            recordingMeetingId={state.meeting_id}
          />
        </div>
      </div>
    </div>
  );
};
