import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  getMeetingState,
  MEETING_CALL_OFFER_EVENT,
  MEETING_LEVEL_EVENT,
  MEETING_PILL_MODE_EVENT,
  MEETING_SEGMENT_EVENT,
  MEETING_STATE_EVENT,
  setMeetingPillExpanded,
  type CallOffer,
  type MeetingLevels,
  type MeetingState,
  type SegmentEvent,
} from "@/components/settings/meetings/api";
import {
  itemFromEvent,
  type TranscriptItem,
} from "@/components/settings/meetings/speakers";

const IDLE_STATE: MeetingState = {
  meeting_id: null,
  status: "complete",
  paused: false,
  system_audio: false,
  system_audio_error: null,
  elapsed_ms: 0,
};

const NO_LEVELS: MeetingLevels = { mic: 0, system: 0 };

/**
 * How many live utterances the pill keeps.
 *
 * The pill is on screen for the length of a call, which can be hours, and every
 * segment it holds is a React element and a string that never gets released. The
 * full transcript is in SQLite and readable in Settings; what the pill is for is
 * the last minute of conversation, so old items are dropped from the front rather
 * than accumulated. Without this, an all-day recording ends with a webview
 * holding thousands of DOM nodes.
 */
const MAX_LIVE_ITEMS = 240;

/** How often the local clock re-renders while recording. */
const CLOCK_TICK_MS = 500;

export interface MeetingPillModel {
  state: MeetingState;
  recording: boolean;
  paused: boolean;
  /** Locally interpolated, so the clock does not freeze between state events. */
  elapsedMs: number;
  levels: MeetingLevels;
  items: TranscriptItem[];
  expanded: boolean;
  setExpanded: (expanded: boolean) => void;
  /** A call was detected and nothing is recording yet.
   *
   *  `{ app: null }` is a real offer, not the absence of one: reading the process
   *  name is allowed to fail and the detection does not depend on it, so the card
   *  must be phraseable without a name. Absence of an offer is `null`. */
  offer: { app: string | null } | null;
  clearOffer: () => void;
}

/**
 * Everything the pill renders from.
 *
 * All four subscriptions live here rather than in the component so the component
 * is a pure function of this model — which is what makes the pill's two modes two
 * renders of one state rather than two components that can disagree about whether
 * a meeting is running.
 */
export const useMeetingPill = (): MeetingPillModel => {
  const [state, setState] = useState<MeetingState>(IDLE_STATE);
  const [levels, setLevels] = useState<MeetingLevels>(NO_LEVELS);
  const [items, setItems] = useState<TranscriptItem[]>([]);
  const [expanded, setExpandedState] = useState(false);
  const [offer, setOffer] = useState<{ app: string | null } | null>(null);

  const recording = state.meeting_id !== null;
  const paused = state.paused;

  /* ── state ── */

  useEffect(() => {
    // Ask rather than wait. The window is shown at the same moment the recorder
    // starts, and the next state event could be a pause minutes away — so
    // without this the pill would render "not recording" over a live call.
    void getMeetingState()
      .then(setState)
      .catch(() => {});
  }, []);

  useEffect(() => {
    const unlisten = listen<MeetingState>(MEETING_STATE_EVENT, (event) => {
      setState(event.payload);
      // A recording that ended has nothing live left to show.
      if (event.payload.meeting_id === null) {
        setItems([]);
        setLevels(NO_LEVELS);
      }
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  /* ── transcript ── */

  useEffect(() => {
    const unlisten = listen<SegmentEvent>(MEETING_SEGMENT_EVENT, (event) => {
      setItems((current) => {
        const next = [...current, itemFromEvent(event.payload)];
        return next.length > MAX_LIVE_ITEMS
          ? next.slice(next.length - MAX_LIVE_ITEMS)
          : next;
      });
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  /* ── levels ── */

  useEffect(() => {
    const unlisten = listen<MeetingLevels>(MEETING_LEVEL_EVENT, (event) => {
      setLevels(event.payload);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  // Levels stop arriving the instant capture stops, which would leave the meter
  // frozen at its last reading — indistinguishable from a stalled capture. Decay
  // to zero when nothing has arrived for a while.
  useEffect(() => {
    if (!recording) return;
    const id = window.setInterval(() => {
      setLevels((current) =>
        current.mic === 0 && current.system === 0
          ? current
          : { mic: current.mic * 0.6, system: current.system * 0.6 },
      );
    }, 400);
    return () => window.clearInterval(id);
  }, [recording]);

  /* ── mode ── */

  useEffect(() => {
    const unlisten = listen<boolean>(MEETING_PILL_MODE_EVENT, (event) => {
      setExpandedState(event.payload);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  const setExpanded = useCallback((next: boolean) => {
    // Optimistic, so the card opens on the click rather than on the round trip.
    // Rust echoes `meeting-pill-mode` back, which is a no-op when they agree.
    setExpandedState(next);
    void setMeetingPillExpanded(next).catch(() => {
      // The window refused to change mode, so the UI must not claim it did —
      // an expanded card in a collapsed-size window is clipped to a sliver.
      setExpandedState((current) => (current === next ? !next : current));
    });
  }, []);

  /* ── the call offer ── */

  useEffect(() => {
    const unlisten = listen<CallOffer>(MEETING_CALL_OFFER_EVENT, (event) => {
      setOffer(
        event.payload.active ? { app: event.payload.app ?? null } : null,
      );
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  // A recording that has started answers the offer, so the card must go — whether
  // it was this offer that started it or the user pressing Start in Settings.
  useEffect(() => {
    if (recording) setOffer(null);
  }, [recording]);

  const clearOffer = useCallback(() => setOffer(null), []);

  /* ── clock ── */

  // `elapsed_ms` only arrives when something changes, so a clock driven by it
  // alone sits still through a silence and looks frozen. The last event is the
  // anchor and wall time since then is added on top; paused time is excluded
  // because the backend's counter is the microphone accumulator, which drops
  // paused frames rather than buffering them.
  const anchor = useRef({ elapsed: state.elapsed_ms, at: Date.now() });
  const [tick, setTick] = useState(() => Date.now());
  useEffect(() => {
    anchor.current = { elapsed: state.elapsed_ms, at: Date.now() };
    setTick(Date.now());
  }, [state.elapsed_ms, paused]);
  useEffect(() => {
    if (!recording || paused) return;
    const id = window.setInterval(() => setTick(Date.now()), CLOCK_TICK_MS);
    return () => window.clearInterval(id);
  }, [recording, paused]);

  const elapsedMs = paused
    ? anchor.current.elapsed
    : anchor.current.elapsed + Math.max(0, tick - anchor.current.at);

  return {
    state,
    recording,
    paused,
    elapsedMs,
    levels,
    items,
    expanded,
    setExpanded,
    offer,
    clearOffer,
  };
};
