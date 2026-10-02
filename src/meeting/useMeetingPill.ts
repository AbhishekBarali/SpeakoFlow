import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
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
  liveMeetingId,
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
  dropped_chunks: 0,
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

/**
 * The latest levels, held outside React state.
 *
 * `meeting-level` arrives ~15x a second for the whole call. Kept in the hook's
 * state, every event re-rendered the entire pill, the expanded card's
 * transcript list included. The pill's root never draws a level itself, so the
 * levels live here and only the leaves that draw them (the waveform, the two
 * side lights) subscribe, through `useMeetingLevel`.
 */
export interface LevelStore {
  get: () => MeetingLevels;
  set: (next: MeetingLevels) => void;
  update: (next: (current: MeetingLevels) => MeetingLevels) => void;
  subscribe: (onChange: () => void) => () => void;
}

function createLevelStore(): LevelStore {
  let value = NO_LEVELS;
  const listeners = new Set<() => void>();
  const set = (next: MeetingLevels) => {
    if (next === value) return;
    value = next;
    for (const listener of listeners) listener();
  };
  return {
    get: () => value,
    set,
    update: (next) => set(next(value)),
    subscribe: (onChange) => {
      listeners.add(onChange);
      return () => {
        listeners.delete(onChange);
      };
    },
  };
}

/** One number derived from the live levels; re-renders only when it changes. */
export const useMeetingLevel = (
  store: LevelStore,
  pick: (levels: MeetingLevels) => number,
): number => useSyncExternalStore(store.subscribe, () => pick(store.get()));

/** Where the locally interpolated clock counts from: the last `elapsed_ms` the
 *  backend reported and the wall time it arrived. */
export interface ClockAnchor {
  elapsed: number;
  at: number;
}

/**
 * The pill's clock, ticking on its own.
 *
 * `elapsed_ms` only arrives when something changes, so a clock driven by it
 * alone sits still through a silence and looks frozen. The anchor is the last
 * event and wall time since then is added on top; paused time is excluded
 * because the backend's counter is the microphone accumulator, which drops
 * paused frames rather than buffering them. The 500 ms tick lives in whichever
 * leaf calls this, so it re-renders that leaf and not the whole pill.
 */
export const useMeetingClock = (
  anchor: ClockAnchor,
  paused: boolean,
): number => {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    setNow(Date.now());
  }, [anchor]);
  useEffect(() => {
    if (paused) return;
    const id = window.setInterval(() => setNow(Date.now()), CLOCK_TICK_MS);
    return () => window.clearInterval(id);
  }, [paused]);
  return paused
    ? anchor.elapsed
    : anchor.elapsed + Math.max(0, now - anchor.at);
};

export interface MeetingPillModel {
  state: MeetingState;
  recording: boolean;
  paused: boolean;
  /** What the clock interpolates from; read it with `useMeetingClock`, which
   *  keeps the clock from freezing between state events. */
  clockAnchor: ClockAnchor;
  /** The live levels; read them with `useMeetingLevel`. */
  levels: LevelStore;
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
  const levelStore = useRef<LevelStore | null>(null);
  levelStore.current ??= createLevelStore();
  const levels = levelStore.current;
  const [items, setItems] = useState<TranscriptItem[]>([]);
  const [expanded, setExpandedState] = useState(false);
  const [offer, setOffer] = useState<{ app: string | null } | null>(null);

  const recording = liveMeetingId(state) !== null;
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
      if (liveMeetingId(event.payload) === null) {
        setItems([]);
        levels.set(NO_LEVELS);
      }
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [levels]);

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
      levels.set(event.payload);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [levels]);

  // Levels stop arriving the instant capture stops, which would leave the meter
  // frozen at its last reading — indistinguishable from a stalled capture. Decay
  // to zero when nothing has arrived for a while.
  useEffect(() => {
    if (!recording) return;
    const id = window.setInterval(() => {
      levels.update((current) =>
        current.mic === 0 && current.system === 0
          ? current
          : { mic: current.mic * 0.6, system: current.system * 0.6 },
      );
    }, 400);
    return () => window.clearInterval(id);
  }, [recording, levels]);

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

  // Re-anchored whenever the backend reports a new count or the pause state
  // flips. The ticking is `useMeetingClock`'s, inside the leaf that draws the
  // clock, so it re-renders that leaf rather than the whole pill.
  const [clockAnchor, setClockAnchor] = useState<ClockAnchor>(() => ({
    elapsed: state.elapsed_ms,
    at: Date.now(),
  }));
  useEffect(() => {
    setClockAnchor({ elapsed: state.elapsed_ms, at: Date.now() });
  }, [state.elapsed_ms, paused]);

  return {
    state,
    recording,
    paused,
    clockAnchor,
    levels,
    items,
    expanded,
    setExpanded,
    offer,
    clearOffer,
  };
};
