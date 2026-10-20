import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands, events, type Insights } from "@/bindings";
import { useNavigation } from "@/components/shell/navigation";

/** Coalesces a burst of events (a call's turns, a meeting's state changes)
 *  into one read. */
const REFRESH_DELAY_MS = 400;

/**
 * Dictation, assistant and meeting usage, kept current.
 *
 * Re-read when a dictation lands, when a conversation is saved, when a meeting
 * changes state, when a usage counter is written (a call hung up, a question
 * counted), and whenever the user comes back to Home or Insights: pages stay
 * mounted while hidden, so without that last one a meeting finished elsewhere
 * would not show until the next dictation.
 *
 * `insights` is `null` until the first read lands; `failed` says the last
 * read failed, so a caller still waiting for figures can stop. `reads` counts
 * finished reads, so a caller with its own fallback knows when to refresh it.
 */
export const useInsights = (): {
  insights: Insights | null;
  failed: boolean;
  reads: number;
} => {
  const [insights, setInsights] = useState<Insights | null>(null);
  const [failed, setFailed] = useState(false);
  const [reads, setReads] = useState(0);
  const { page } = useNavigation();
  const timer = useRef<number | null>(null);
  /** Bumped per read, so a slow answer never replaces a newer one. */
  const request = useRef(0);

  const load = useCallback(() => {
    const id = ++request.current;
    const fail = () => {
      if (id !== request.current) return;
      setFailed(true);
      setReads((n) => n + 1);
    };
    void commands
      .getInsights()
      .then((result) => {
        if (id !== request.current) return;
        if (result.status === "ok") {
          setReads((n) => n + 1);
          setInsights(result.data);
          setFailed(false);
        } else {
          fail();
        }
      })
      .catch(fail);
  }, []);

  const loadSoon = useCallback(() => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      load();
    }, REFRESH_DELAY_MS);
  }, [load]);

  useEffect(() => {
    if (page === "home" || page === "insights") load();
  }, [page, load]);

  useEffect(() => {
    const offHistory = events.historyUpdatePayload.listen((event) => {
      if (
        event.payload.action === "added" ||
        event.payload.action === "updated"
      ) {
        loadSoon();
      }
    });
    const offAssistant = listen("assistant-history-updated", loadSoon);
    const offMeeting = listen("meeting-state", loadSoon);
    const offCounters = listen("insights-updated", loadSoon);
    return () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
      for (const off of [offHistory, offAssistant, offMeeting, offCounters]) {
        void off.then((unlisten) => unlisten());
      }
    };
  }, [loadSoon]);

  return { insights, failed, reads };
};
