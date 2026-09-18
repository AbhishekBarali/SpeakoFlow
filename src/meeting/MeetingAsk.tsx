import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowUp, Square } from "lucide-react";
import { useMeetingChat } from "@/components/settings/meetings/useMeetingChat";

interface MeetingAskProps {
  /** Owned by the pill so it can blur the field before collapsing the window —
   *  on Windows a collapsed pill is made unfocusable, and the platform will not
   *  remove focusability from a window that currently holds focus. */
  inputRef: React.RefObject<HTMLInputElement>;
  meetingId: number | null;
}

/**
 * Ask a question about the call that is happening right now.
 *
 * The answer renders above the input, replacing the previous one rather than
 * accumulating a thread — the pill is a few hundred pixels tall and floating over
 * the user's work, so a scrolling chat log inside it would push the live
 * transcript off screen. The full thread is kept in Rust and shown in Settings →
 * Meetings, so nothing is lost by only displaying the latest exchange here.
 */
export const MeetingAsk: React.FC<MeetingAskProps> = ({
  inputRef,
  meetingId,
}) => {
  const { t } = useTranslation();
  const { messages, streaming, busy, error, ask, cancel, dismissError } =
    useMeetingChat(meetingId);
  const [draft, setDraft] = useState("");

  const submit = () => {
    const question = draft.trim();
    if (!question || busy) return;
    ask(question);
    setDraft("");
  };

  // The newest answer, or whatever has streamed of it so far.
  const lastAnswer = [...messages]
    .reverse()
    .find((message) => message.role === "assistant")?.content;
  const shown = streaming || (busy ? "" : lastAnswer);

  const answerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const node = answerRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [shown]);

  return (
    <>
      {error && (
        <p className="pill-notice" role="alert" onClick={dismissError}>
          {error}
        </p>
      )}

      {(shown || busy) && (
        <div className="pill-answer" ref={answerRef}>
          {shown ? (
            <p className="pill-answer-text">{shown}</p>
          ) : (
            <p className="pill-answer-text" data-thinking="true">
              {t("meetings.ask.thinking")}
            </p>
          )}
        </div>
      )}

      <div className="pill-ask">
        <input
          ref={inputRef}
          className="pill-ask-input"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              submit();
            }
            // Escape gives the field up without collapsing the window, so a
            // mistyped question does not cost the transcript view.
            if (event.key === "Escape") {
              event.stopPropagation();
              setDraft("");
              inputRef.current?.blur();
            }
          }}
          placeholder={t("meetings.ask.placeholder")}
          aria-label={t("meetings.ask.placeholder")}
          disabled={meetingId === null}
        />
        {busy ? (
          <button
            type="button"
            className="pill-action"
            onClick={cancel}
            title={t("meetings.ask.stop")}
            aria-label={t("meetings.ask.stop")}
          >
            <Square size={11} />
          </button>
        ) : (
          <button
            type="button"
            className="pill-action"
            onClick={submit}
            disabled={!draft.trim()}
            title={t("meetings.ask.send")}
            aria-label={t("meetings.ask.send")}
          >
            <ArrowUp size={13} />
          </button>
        )}
      </div>
    </>
  );
};
