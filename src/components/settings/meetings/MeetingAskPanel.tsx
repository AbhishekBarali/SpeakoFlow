import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { ArrowUp, Eraser, Square } from "lucide-react";
import { Button } from "@/components/ui/Button";
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

/**
 * Ask questions about a recorded meeting.
 *
 * The same thread the floating pill uses, so a question asked mid-call and its
 * follow-up asked afterwards are one conversation. Unlike the pill — which shows
 * only the newest exchange because it is a few hundred pixels tall — this shows
 * the whole thread, since there is room for it.
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
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [messages.length, streaming]);

  const submit = () => {
    const question = draft.trim();
    if (!question || busy) return;
    ask(question);
    setDraft("");
  };

  if (!hasTranscript) {
    return (
      <div className="rounded-2xl border border-hairline bg-surface elev-card p-4">
        <p className="py-6 text-center text-[13px] text-muted">
          {t("meetings.summary.needsTranscript")}
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {error && (
        <div className="flex items-start justify-between gap-3 rounded-xl border border-error/40 bg-error/10 px-3.5 py-2.5">
          <p className="text-[12.5px] text-error">{error}</p>
          <button
            type="button"
            onClick={dismissError}
            className="shrink-0 cursor-pointer text-[11.5px] text-muted underline hover:text-ink"
          >
            {t("common.close")}
          </button>
        </div>
      )}

      <div className="rounded-2xl border border-hairline bg-surface elev-card">
        <div className="max-h-[420px] overflow-y-auto px-4 py-3">
          {messages.length === 0 && !streaming ? (
            <p className="py-8 text-center text-[13px] text-muted">
              {t("meetings.ask.empty")}
            </p>
          ) : (
            <div className="space-y-4">
              {messages.map((message, index) => (
                <div
                  // Index is safe here: the thread is append-only and always
                  // re-rendered from a full snapshot, so an index never points at
                  // a different message than it did before.
                  key={`${message.role}-${index}`}
                  className="space-y-1"
                >
                  <p className="text-[10.5px] font-semibold uppercase tracking-wide text-muted-soft">
                    {message.role === "user"
                      ? t("meetings.ask.youAsked")
                      : t("meetings.ask.title")}
                  </p>
                  {message.role === "user" ? (
                    <p className="text-[13px] leading-relaxed text-ink">
                      {message.content}
                    </p>
                  ) : (
                    <div className="text-[13px] leading-relaxed text-body">
                      <ReactMarkdown
                        remarkPlugins={[remarkGfm]}
                        components={markdown}
                      >
                        {message.content}
                      </ReactMarkdown>
                    </div>
                  )}
                </div>
              ))}

              {/* The reply in flight. Rendered from the transient buffer, and
                  replaced by the authoritative snapshot when the turn ends. */}
              {streaming && (
                <div className="space-y-1">
                  <p className="text-[10.5px] font-semibold uppercase tracking-wide text-muted-soft">
                    {t("meetings.ask.title")}
                  </p>
                  <p className="whitespace-pre-wrap text-[13px] leading-relaxed text-body">
                    {streaming}
                  </p>
                </div>
              )}

              {busy && !streaming && (
                <p className="text-[12.5px] italic text-muted-soft">
                  {t("meetings.ask.thinking")}
                </p>
              )}
              <div ref={endRef} />
            </div>
          )}
        </div>

        <div className="flex items-center gap-2 border-t border-hairline px-3 py-2.5">
          <input
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
            className="min-w-0 flex-1 rounded-lg border border-hairline-strong bg-surface px-3 py-2 text-[13px] text-ink focus:border-ink focus:outline-none"
          />
          {busy ? (
            <Button
              variant="secondary"
              size="sm"
              onClick={cancel}
              className="gap-1.5"
            >
              <Square size={11} />
              {t("meetings.ask.stop")}
            </Button>
          ) : (
            <Button
              variant="primary"
              size="sm"
              onClick={submit}
              disabled={!draft.trim()}
              className="gap-1.5"
            >
              <ArrowUp size={13} />
              {t("meetings.ask.send")}
            </Button>
          )}
          {messages.length > 0 && !busy && (
            <button
              type="button"
              onClick={clear}
              title={t("meetings.ask.clear")}
              aria-label={t("meetings.ask.clear")}
              className="shrink-0 cursor-pointer rounded-md p-1.5 text-muted-soft transition-colors hover:bg-ink/6 hover:text-ink"
            >
              <Eraser size={13} />
            </button>
          )}
        </div>
      </div>
    </div>
  );
};
