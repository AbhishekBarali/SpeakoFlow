import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { ArrowUp, Eraser, MessageSquareText, Square, X } from "lucide-react";
import { useMeetingChat } from "./useMeetingChat";

interface MeetingAskPanelProps {
  meetingId: number;
  /** Suppresses the input when there is nothing to ask about — a model handed an
   *  empty transcript invents a meeting rather than admitting it has none. */
  hasTranscript: boolean;
  /** Markdown rules, shared with the Summary tab so an answer reads like the rest
   *  of the app rather than like raw syntax. */
  markdown: Components;
}

/** Ways in on an empty thread. The same four questions an empty call about a
 *  meeting offers, so the two surfaces suggest the same things. */
const PROMPTS = ["recap", "next", "followUp", "missed"] as const;

/**
 * Ask questions about a recorded meeting.
 *
 * The same thread the floating pill uses, so a question asked mid-call and its
 * follow-up asked afterwards are one conversation. Unlike the pill — which shows
 * only the newest exchange because it is a few hundred pixels tall — this shows
 * the whole thread, since there is room for it.
 *
 * Laid out like a conversation rather than a log: your question is a bubble on
 * the trailing side, the answer is ordinary text at reading width. It used to
 * put an uppercase "YOU ASKED" / "ASK ABOUT THIS MEETING" label over every
 * message, which made a four-message thread read as eight.
 */
export const MeetingAskPanel: React.FC<MeetingAskPanelProps> = ({
  meetingId,
  hasTranscript,
  markdown,
}) => {
  const { t } = useTranslation();
  const { messages, streaming, busy, error, ask, cancel, clear, dismissError } =
    useMeetingChat(meetingId);
  const [draft, setDraft] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Scroll the thread, never the page: `scrollIntoView` on the last message
  // moved the whole window to it.
  useEffect(() => {
    const node = scrollRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [messages.length, streaming, busy]);

  const submit = (text: string = draft) => {
    const question = text.trim();
    if (!question || busy) return;
    ask(question);
    setDraft("");
  };

  if (!hasTranscript) {
    return (
      <div className="rounded-2xl border border-dashed border-hairline-strong px-6 py-12 text-center text-sm text-muted">
        {t("meetings.summary.needsTranscript")}
      </div>
    );
  }

  const empty = messages.length === 0 && !streaming && !busy;

  return (
    <div className="space-y-3">
      {error && (
        <div
          className="flex items-start gap-3 rounded-xl border border-error/30 bg-error/[0.06] px-4 py-3"
          role="alert"
        >
          <p className="min-w-0 flex-1 text-sm leading-relaxed text-ink">
            {error}
          </p>
          <button
            type="button"
            onClick={dismissError}
            title={t("common.close")}
            aria-label={t("common.close")}
            className="-me-1 grid h-7 w-7 shrink-0 cursor-pointer place-items-center rounded-md text-muted transition-colors hover:bg-ink/6 hover:text-ink"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </div>
      )}

      <div className="overflow-hidden rounded-2xl border border-hairline bg-surface elev-card">
        <div
          ref={scrollRef}
          className="max-h-[32rem] min-h-[16rem] overflow-y-auto px-5 py-6 sm:px-7"
        >
          {empty ? (
            <div className="flex flex-col items-center py-6 text-center">
              <span className="grid h-11 w-11 place-items-center rounded-xl bg-surface-strong text-muted">
                <MessageSquareText className="h-5 w-5" aria-hidden="true" />
              </span>
              <p className="mt-4 max-w-sm text-sm leading-relaxed text-muted text-pretty">
                {t("meetings.ask.empty")}
              </p>
              <div className="mt-5 flex max-w-lg flex-wrap justify-center gap-2">
                {PROMPTS.map((key) => {
                  const text = t(
                    `assistant.conversation.meeting.prompts.${key}`,
                  );
                  return (
                    <button
                      key={key}
                      type="button"
                      onClick={() => submit(text)}
                      className="cursor-pointer rounded-full border border-hairline-strong bg-surface px-3.5 py-1.5 text-[0.8125rem] text-body transition-colors hover:border-ink/30 hover:bg-surface-strong hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
                    >
                      {text}
                    </button>
                  );
                })}
              </div>
            </div>
          ) : (
            <div className="space-y-6">
              {messages.map((message, index) =>
                // Index is safe here: the thread is append-only and always
                // re-rendered from a full snapshot, so an index never points at
                // a different message than it did before.
                message.role === "user" ? (
                  <Question key={`user-${index}`} text={message.content} />
                ) : (
                  <Answer
                    key={`assistant-${index}`}
                    label={t("meetings.ask.title")}
                  >
                    <ReactMarkdown
                      remarkPlugins={[remarkGfm]}
                      components={markdown}
                    >
                      {message.content}
                    </ReactMarkdown>
                  </Answer>
                ),
              )}

              {/* The reply in flight. Rendered from the transient buffer, and
                  replaced by the authoritative snapshot when the turn ends. */}
              {streaming && (
                <Answer label={t("meetings.ask.title")}>
                  <p className="whitespace-pre-wrap">{streaming}</p>
                </Answer>
              )}

              {busy && !streaming && (
                <p
                  className="flex items-center gap-2.5 text-sm text-muted"
                  role="status"
                >
                  <span className="flex gap-1" aria-hidden="true">
                    {[0, 1, 2].map((dot) => (
                      <span
                        key={dot}
                        className="h-1.5 w-1.5 rounded-full bg-muted-soft motion-safe:animate-pulse"
                        style={{ animationDelay: `${dot * 160}ms` }}
                      />
                    ))}
                  </span>
                  {t("meetings.ask.thinking")}
                </p>
              )}
            </div>
          )}
        </div>

        <div className="border-t border-hairline p-3">
          <div className="flex items-center gap-1.5 rounded-xl border border-hairline-strong bg-surface ps-4 pe-1.5 transition-colors focus-within:border-ink/60">
            <input
              ref={inputRef}
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault();
                  submit();
                }
              }}
              placeholder={t("meetings.ask.placeholder")}
              aria-label={t("meetings.ask.placeholder")}
              className="h-11 min-w-0 flex-1 bg-transparent text-[0.9375rem] text-ink placeholder:text-muted-soft focus:outline-none"
            />
            {messages.length > 0 && !busy && (
              <button
                type="button"
                onClick={() => {
                  clear();
                  inputRef.current?.focus();
                }}
                title={t("meetings.ask.clear")}
                aria-label={t("meetings.ask.clear")}
                className="grid h-8 w-8 shrink-0 cursor-pointer place-items-center rounded-lg text-muted transition-colors hover:bg-ink/6 hover:text-ink"
              >
                <Eraser className="h-4 w-4" aria-hidden="true" />
              </button>
            )}
            {busy ? (
              <button
                type="button"
                onClick={cancel}
                title={t("meetings.ask.stop")}
                aria-label={t("meetings.ask.stop")}
                className="grid h-8 w-8 shrink-0 cursor-pointer place-items-center rounded-lg bg-surface-strong text-ink transition-colors hover:bg-hairline-strong"
              >
                <Square className="h-3 w-3 fill-current" aria-hidden="true" />
              </button>
            ) : (
              <button
                type="button"
                onClick={() => submit()}
                disabled={!draft.trim()}
                title={t("meetings.ask.send")}
                aria-label={t("meetings.ask.send")}
                className="grid h-8 w-8 shrink-0 cursor-pointer place-items-center rounded-lg bg-ink text-on-ink transition-[background-color,opacity] hover:bg-ink-soft disabled:cursor-not-allowed disabled:bg-surface-strong disabled:text-muted-soft"
              >
                <ArrowUp className="h-4 w-4" aria-hidden="true" />
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

const Question: React.FC<{ text: string }> = ({ text }) => (
  <div className="flex justify-end">
    <p className="max-w-[80%] rounded-2xl rounded-ee-md bg-surface-strong px-4 py-2.5 text-[0.9375rem] leading-relaxed whitespace-pre-wrap break-words text-ink select-text">
      {text}
    </p>
  </div>
);

/** An answer: plain text at reading width. The label is for screen readers;
 *  on screen, the bubble opposite already says who asked. */
const Answer: React.FC<{ label: string; children: React.ReactNode }> = ({
  label,
  children,
}) => (
  <div className="max-w-[72ch]">
    <p className="sr-only">{label}</p>
    <div className="text-[0.9375rem] leading-[1.7] break-words text-body select-text">
      {children}
    </div>
  </div>
);
