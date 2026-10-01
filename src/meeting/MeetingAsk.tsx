import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { AlertCircle, ArrowUp, Square } from "lucide-react";
import { useMeetingChat } from "@/components/settings/meetings/useMeetingChat";

interface MeetingAskProps {
  /** Owned by the pill so it can blur the field before collapsing the window —
   *  on Windows a collapsed pill is made unfocusable, and the platform will not
   *  remove focusability from a window that currently holds focus. */
  inputRef: React.RefObject<HTMLInputElement>;
  meetingId: number | null;
}

/** Answers are markdown. Without this the card showed the model's `**` and
 *  `- ` literally; the styling itself lives on `.pill-answer-text`. */
const ANSWER_MARKDOWN: Components = {
  a: ({ children }) => <span>{children}</span>,
  h1: ({ children }) => (
    <p>
      <strong>{children}</strong>
    </p>
  ),
  h2: ({ children }) => (
    <p>
      <strong>{children}</strong>
    </p>
  ),
  h3: ({ children }) => (
    <p>
      <strong>{children}</strong>
    </p>
  ),
};

/**
 * Ask a question about the call that is happening right now.
 *
 * The answer renders above the input, replacing the previous one rather than
 * accumulating a thread — the pill is a few hundred pixels tall and floating over
 * the user's work, so a scrolling chat log inside it would push the live
 * transcript off screen. The full thread is kept in Rust and shown in Settings →
 * Meetings, so nothing is lost by only displaying the latest exchange here. The
 * question it answers sits on one quiet line above it, so an answer read a
 * minute later still says what it was about.
 */
export const MeetingAsk: React.FC<MeetingAskProps> = ({
  inputRef,
  meetingId,
}) => {
  const { t } = useTranslation();
  const { messages, streaming, busy, error, ask, cancel, dismissError } =
    useMeetingChat(meetingId);
  const [draft, setDraft] = useState("");
  // The question just sent, until the thread snapshot that contains it lands.
  const [asked, setAsked] = useState<string | null>(null);

  useEffect(() => {
    setAsked(null);
  }, [messages]);

  const submit = () => {
    const question = draft.trim();
    if (!question || busy) return;
    setAsked(question);
    ask(question);
    setDraft("");
  };

  // The newest exchange, or whatever has streamed of its answer so far.
  const reversed = [...messages].reverse();
  const lastAnswer = reversed.find(
    (message) => message.role === "assistant",
  )?.content;
  const lastQuestion =
    asked ?? reversed.find((message) => message.role === "user")?.content;
  const shown = streaming || (busy ? "" : lastAnswer);

  const answerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const node = answerRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [shown]);

  return (
    <>
      {error && (
        <p
          className="pill-notice"
          data-tone="error"
          role="alert"
          onClick={dismissError}
          title={t("common.close")}
        >
          <AlertCircle size={13} aria-hidden="true" />
          <span className="pill-notice-text">{error}</span>
        </p>
      )}

      {(shown || busy) && (
        <div className="pill-answer" ref={answerRef}>
          {lastQuestion && (
            <p className="pill-answer-question" title={lastQuestion}>
              {lastQuestion}
            </p>
          )}
          {shown ? (
            <div className="pill-answer-text">
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                components={ANSWER_MARKDOWN}
              >
                {shown}
              </ReactMarkdown>
            </div>
          ) : (
            <p className="pill-answer-thinking" role="status">
              <span className="pill-dots" aria-hidden="true">
                <span />
                <span />
                <span />
              </span>
              {t("meetings.ask.thinking")}
            </p>
          )}
        </div>
      )}

      <div className="pill-ask">
        <div className="pill-ask-field">
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
              className="pill-send"
              data-variant="stop"
              onClick={cancel}
              title={t("meetings.ask.stop")}
              aria-label={t("meetings.ask.stop")}
            >
              <Square size={11} fill="currentColor" />
            </button>
          ) : (
            <button
              type="button"
              className="pill-send"
              onClick={submit}
              disabled={!draft.trim()}
              title={t("meetings.ask.send")}
              aria-label={t("meetings.ask.send")}
            >
              <ArrowUp size={15} />
            </button>
          )}
        </div>
      </div>
    </>
  );
};
