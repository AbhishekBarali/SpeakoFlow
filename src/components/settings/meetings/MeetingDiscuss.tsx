import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { AudioLines, ChevronRight, MessageCircle } from "lucide-react";
import { commands, type AssistantHistorySummary } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "@/components/ui/Button";
import { formatRelativeTime } from "@/utils/dateFormat";

/** Earlier discussions shown on the page; the call's history has the rest. */
const DISCUSSIONS_SHOWN = 3;

export interface MeetingDiscussModel {
  /** False when the assistant is switched off: nothing here is offered. */
  available: boolean;
  starting: boolean;
  /** Why the button is disabled, said in words; null when it is not. */
  disabledReason: string | null;
  discussions: AssistantHistorySummary[];
  hasMore: boolean;
  discuss: () => void;
  resume: (conversationId: number) => void;
}

/**
 * Take a meeting into an assistant call.
 *
 * The Ask tab answers *from* the transcript and nothing else. A call is for
 * talking it over: what to do next, a draft of the follow-up, whether an
 * estimate was reasonable — the assistant's own judgement with the meeting as
 * context. Short meetings ride along whole; long ones are read on demand (see
 * `meetings::discuss`), so a two-hour call costs no more per turn than a short
 * one.
 *
 * Conversations started here remember their meeting, so the ones about this
 * meeting can be listed and picked up again. One hook feeds two places — the
 * button in the page header and the list of earlier discussions — so they
 * cannot disagree.
 */
export const useMeetingDiscuss = (
  meetingId: number,
  /** Recording right now: the call's own voice would land in the transcript. */
  isLive: boolean,
  /** There is a transcript or notes to talk about. */
  hasMaterial: boolean,
): MeetingDiscussModel => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const available = getSetting("assistant_enabled") ?? true;
  const [starting, setStarting] = useState(false);
  const [discussions, setDiscussions] = useState<AssistantHistorySummary[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const generationRef = useRef(0);

  const loadDiscussions = useCallback(async () => {
    generationRef.current += 1;
    const generation = generationRef.current;
    try {
      const result = await commands.listAssistantConversations(
        { query: null, meetings_only: false, meeting_id: meetingId },
        0,
        DISCUSSIONS_SHOWN,
      );
      if (generation !== generationRef.current || result.status !== "ok")
        return;
      setDiscussions(result.data.entries);
      setHasMore(result.data.has_more);
    } catch {
      // The list is a convenience; the button works without it.
    }
  }, [meetingId]);

  useEffect(() => {
    setDiscussions([]);
    setHasMore(false);
    void loadDiscussions();
  }, [loadDiscussions]);

  // A call about this meeting saves after every turn.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const unlisten = listen("assistant-history-updated", () => {
      if (timer !== null) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        void loadDiscussions();
      }, 400);
    });
    return () => {
      if (timer !== null) clearTimeout(timer);
      void unlisten.then((off) => off());
    };
  }, [loadDiscussions]);

  const discuss = () => {
    if (starting) return;
    setStarting(true);
    void commands
      .assistantDiscussMeeting(meetingId)
      .then((result) => {
        if (result.status !== "ok")
          toast.error(t("meetings.discuss.failed", { error: result.error }));
      })
      .catch((error: unknown) =>
        toast.error(t("meetings.discuss.failed", { error: String(error) })),
      )
      .finally(() => setStarting(false));
  };

  const resume = (id: number) => {
    void commands
      .assistantResumeSession(id)
      .then((result) => {
        if (result.status !== "ok")
          toast.error(t("meetings.discuss.failed", { error: result.error }));
      })
      .catch((error: unknown) =>
        toast.error(t("meetings.discuss.failed", { error: String(error) })),
      );
  };

  const disabledReason = isLive
    ? t("meetings.discuss.whileRecording")
    : !hasMaterial
      ? t("meetings.discuss.nothingYet")
      : null;

  return {
    available,
    starting,
    disabledReason,
    discussions,
    hasMore,
    discuss,
    resume,
  };
};

/** "Discuss in a call", for the page header. The reason it is unavailable is
 *  its tooltip, so the header stays one row. */
export const DiscussButton: React.FC<{ model: MeetingDiscussModel }> = ({
  model,
}) => {
  const { t } = useTranslation();
  if (!model.available) return null;
  return (
    <Button
      variant="secondary"
      size="md"
      onClick={model.discuss}
      disabled={model.starting || model.disabledReason !== null}
      title={model.disabledReason ?? t("meetings.discuss.hint")}
    >
      <AudioLines className="h-4 w-4" aria-hidden="true" />
      {t("meetings.discuss.button")}
    </Button>
  );
};

/** Calls already had about this meeting, each one click from picking it up
 *  again. Renders nothing until there is at least one. */
export const EarlierDiscussions: React.FC<{
  model: MeetingDiscussModel;
  className?: string;
}> = ({ model, className = "" }) => {
  const { t, i18n } = useTranslation();
  if (!model.available || model.discussions.length === 0) return null;

  return (
    <section className={`space-y-2.5 ${className}`}>
      <h3 className="px-1 text-[0.8125rem] font-semibold text-muted">
        {t("meetings.discuss.earlier")}
      </h3>
      <ul className="divide-y divide-hairline overflow-hidden rounded-2xl border border-hairline bg-surface elev-card">
        {model.discussions.map((entry) => (
          <li key={entry.id}>
            <button
              type="button"
              onClick={() => model.resume(entry.id)}
              title={t("meetings.discuss.continue")}
              className="group flex w-full cursor-pointer items-center gap-3 px-4 py-3 text-start transition-colors hover:bg-surface-muted/70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-accent/40"
            >
              <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-surface-strong text-muted">
                <MessageCircle className="h-4 w-4" aria-hidden="true" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium text-ink">
                  {entry.title.trim() || t("meetings.discuss.untitled")}
                </span>
                <span className="mt-0.5 block text-xs text-muted">
                  {formatRelativeTime(
                    String(entry.updated_at || entry.timestamp),
                    i18n.language,
                  )}
                  {" · "}
                  {t("meetings.discuss.messages", {
                    count: entry.message_count,
                  })}
                </span>
              </span>
              <ChevronRight
                className="h-4 w-4 shrink-0 text-muted-soft transition-transform group-hover:translate-x-0.5 rtl:rotate-180"
                aria-hidden="true"
              />
            </button>
          </li>
        ))}
      </ul>
      {model.hasMore && (
        <p className="px-1 text-xs text-muted">{t("meetings.discuss.more")}</p>
      )}
    </section>
  );
};
