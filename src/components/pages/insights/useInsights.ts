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
 * changes state, and whenever the user comes back to Home or Insights: pages
 * stay mounted while hidden, so without that last one a meeting finished
 * elsewhere would not show until the next dictation.
 */
export const useInsights = (): Insights | null => {
  const [insights, setInsights] = useState<Insights | null>(null);
  const { page } = useNavigation();
  const timer = useRef<number | null>(null);

  const load = useCallback(() => {
    void commands
      .getInsights()
      .then((result) => {
        if (result.status === "ok") setInsights(result.data);
      })
      .catch(() => {});
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
    return () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
      for (const off of [offHistory, offAssistant, offMeeting]) {
        void off.then((unlisten) => unlisten());
      }
    };
  }, [loadSoon]);

  return insights;
};
