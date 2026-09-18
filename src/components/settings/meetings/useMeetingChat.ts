import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  askAboutMeeting,
  cancelMeetingChat,
  clearMeetingChat,
  getMeetingChat,
  MEETING_CHAT_ERROR_EVENT,
  MEETING_CHAT_MESSAGES_EVENT,
  MEETING_CHAT_STATE_EVENT,
  MEETING_CHAT_TOKEN_EVENT,
  type MeetingChatMessage,
} from "./api";

export interface MeetingChatModel {
  messages: MeetingChatMessage[];
  /** Text streamed so far for the reply in flight. Empty when idle. */
  streaming: string;
  busy: boolean;
  error: string | null;
  ask: (question: string) => void;
  cancel: () => void;
  clear: () => void;
  dismissError: () => void;
}

/**
 * The question-and-answer thread for one meeting.
 *
 * Shared by the floating pill and the meeting detail view, so a question asked
 * mid-call and its follow-up asked afterwards are the same conversation.
 *
 * **Renders from snapshots, not from deltas.** `messages` is only ever replaced
 * wholesale by what Rust sends, and `streaming` is a transient buffer cleared on
 * every snapshot. That makes the UI idempotent: with both surfaces mounted, or a
 * listener registered twice by a StrictMode double-effect, a message still cannot
 * appear twice. Appending deltas into `messages` instead is how that breaks.
 */
export const useMeetingChat = (meetingId: number | null): MeetingChatModel => {
  const [messages, setMessages] = useState<MeetingChatMessage[]>([]);
  const [streaming, setStreaming] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Read in the token listener, which must not re-subscribe whenever the open
  // meeting changes — a listener torn down mid-stream loses the rest of the reply.
  const active = useRef<number | null>(meetingId);
  active.current = meetingId;

  /* ── the thread ── */

  useEffect(() => {
    if (meetingId === null) {
      setMessages([]);
      setStreaming("");
      return;
    }
    let cancelled = false;
    // Also what tells Rust which meeting is open, so it can clear a thread that
    // belonged to a different one.
    void getMeetingChat(meetingId)
      .then((thread) => {
        if (!cancelled) setMessages(thread);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [meetingId]);

  useEffect(() => {
    const unlisten = listen<MeetingChatMessage[]>(
      MEETING_CHAT_MESSAGES_EVENT,
      (event) => {
        setMessages(event.payload);
        // The snapshot is authoritative and already contains the finished reply,
        // so the buffer must go or the last answer renders twice.
        setStreaming("");
      },
    );
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  useEffect(() => {
    const unlisten = listen<string>(MEETING_CHAT_TOKEN_EVENT, (event) => {
      setStreaming((current) => current + event.payload);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  useEffect(() => {
    const unlisten = listen<boolean>(MEETING_CHAT_STATE_EVENT, (event) => {
      setBusy(event.payload);
      if (!event.payload) setStreaming("");
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  useEffect(() => {
    const unlisten = listen<string>(MEETING_CHAT_ERROR_EVENT, (event) => {
      setError(event.payload);
      setBusy(false);
      setStreaming("");
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  /* ── actions ── */

  const ask = useCallback(
    (question: string) => {
      const text = question.trim();
      if (!text || meetingId === null) return;
      setError(null);
      // Optimistic, so the input clears and the spinner appears on the keystroke
      // rather than after the round trip. Rust echoes the real state back.
      setBusy(true);
      setStreaming("");
      void askAboutMeeting(meetingId, text)
        .catch((reason: unknown) => {
          setError(String(reason));
        })
        .finally(() => setBusy(false));
    },
    [meetingId],
  );

  const cancel = useCallback(() => {
    void cancelMeetingChat().catch(() => {});
  }, []);

  const clear = useCallback(() => {
    setMessages([]);
    setStreaming("");
    setError(null);
    void clearMeetingChat().catch(() => {});
  }, []);

  const dismissError = useCallback(() => setError(null), []);

  return {
    messages,
    streaming,
    busy,
    error,
    ask,
    cancel,
    clear,
    dismissError,
  };
};
