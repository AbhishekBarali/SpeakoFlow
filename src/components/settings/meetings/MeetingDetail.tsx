import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { toast } from "sonner";
import { Check, Pencil, Sparkles, Users, X } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { Dropdown } from "@/components/ui/Dropdown";
import { Textarea } from "@/components/ui/Textarea";
import { formatDateTime } from "@/utils/dateFormat";
import {
  DEFAULT_NOTES_TEMPLATE,
  generateMeetingNotes,
  getMeeting,
  getMeetingSegments,
  getMeetingSpeakers,
  MEETING_NOTES_PROGRESS_EVENT,
  MEETING_SEGMENT_EVENT,
  NOTES_TEMPLATES,
  renameMeeting,
  renameMeetingSpeaker,
  SEGMENT_PAGE_SIZE,
  setMeetingMyNotes,
  type Meeting,
  type MeetingSpeaker,
  type NotesProgress,
  type NotesTemplateId,
  type SegmentEvent,
} from "./api";
import {
  formatClock,
  itemFromEvent,
  itemFromSegment,
  meetingDurationMs,
  ME_SPEAKER_KEY,
  type TranscriptItem,
} from "./speakers";
import { MeetingAskPanel } from "./MeetingAskPanel";
import { SpeakerIdentification } from "./SpeakerIdentification";
import { TranscriptView } from "./TranscriptView";

interface MeetingDetailProps {
  meetingId: number;
  /** The meeting recording right now, if any. Opening its detail should keep
   *  filling in as segments land. */
  recordingMeetingId: number | null;
  /** Told when the title changes so the list behind this page agrees with it. */
  onChanged: () => void;
}

type DetailTab = "notes" | "transcript" | "summary" | "ask";

/** How long typing pauses before "My thoughts" is written. Long enough not to
 *  write on every keystroke, short enough that switching tabs or closing the
 *  page cannot plausibly beat it — and both of those flush explicitly anyway. */
const NOTES_DEBOUNCE_MS = 700;

/** Ceiling on the background transcript fetch: 100 pages of 200 is 20,000
 *  segments, far past any real meeting, and it means a bug in `has_more` cannot
 *  turn into an endless request loop. */
const MAX_TRANSCRIPT_PAGES = 100;

/** Notes are markdown. Same rules as the assistant's replies so a summary reads
 *  like the rest of the app rather than like raw syntax. */
const notesMarkdown: Components = {
  p: ({ children }) => <p className="mb-2 last:mb-0">{children}</p>,
  ul: ({ children }) => (
    <ul className="mb-2 list-disc space-y-1 ps-5 last:mb-0">{children}</ul>
  ),
  ol: ({ children }) => (
    <ol className="mb-2 list-decimal space-y-1 ps-5 last:mb-0">{children}</ol>
  ),
  li: ({ children }) => <li className="leading-relaxed">{children}</li>,
  strong: ({ children }) => (
    <strong className="font-semibold text-ink">{children}</strong>
  ),
  em: ({ children }) => <em className="italic">{children}</em>,
  h1: ({ children }) => (
    <p className="mb-1 mt-3 text-[13.5px] font-semibold text-ink first:mt-0">
      {children}
    </p>
  ),
  h2: ({ children }) => (
    <p className="mb-1 mt-3 text-[13.5px] font-semibold text-ink first:mt-0">
      {children}
    </p>
  ),
  h3: ({ children }) => (
    <p className="mb-1 mt-2 font-semibold text-ink first:mt-0">{children}</p>
  ),
  code: ({ children }) => (
    <code className="rounded bg-mid-gray/15 px-1 py-0.5 font-mono text-[0.85em]">
      {children}
    </code>
  ),
  pre: ({ children }) => (
    <pre className="my-2 overflow-x-auto rounded-lg border border-hairline bg-mid-gray/10 p-3 text-[0.85em] [&_code]:bg-transparent [&_code]:p-0">
      {children}
    </pre>
  ),
  blockquote: ({ children }) => (
    <blockquote className="my-2 border-s-2 border-hairline-strong ps-3 text-muted">
      {children}
    </blockquote>
  ),
};

const isTemplateId = (value: string | null): value is NotesTemplateId =>
  value !== null && NOTES_TEMPLATES.includes(value as NotesTemplateId);

/**
 * One meeting: what the user thought, what was said, and what it added up to.
 *
 * The transcript is fetched in pages and then windowed, which is two different
 * problems with the same cause — an hour of speech is a thousand segments, and
 * neither one query nor one DOM tree wants all of them at once.
 */
export const MeetingDetail: React.FC<MeetingDetailProps> = ({
  meetingId,
  recordingMeetingId,
  onChanged,
}) => {
  const { t, i18n } = useTranslation();
  // Summary first. It used to be "My thoughts", which made sense when notes only
  // existed if you asked for them; now they are written automatically the moment
  // the call ends, so the summary is what someone opening a finished meeting came
  // to read.
  const [tab, setTab] = useState<DetailTab>("summary");
  const [meeting, setMeeting] = useState<Meeting | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [speakers, setSpeakers] = useState<MeetingSpeaker[]>([]);
  const [items, setItems] = useState<TranscriptItem[]>([]);
  const [transcriptLoading, setTranscriptLoading] = useState(true);

  const [titleDraft, setTitleDraft] = useState<string | null>(null);
  const [myNotes, setMyNotes] = useState("");
  const [notesState, setNotesState] = useState<"idle" | "saving" | "saved">(
    "idle",
  );
  const [template, setTemplate] = useState<NotesTemplateId>(
    DEFAULT_NOTES_TEMPLATE,
  );
  const [generating, setGenerating] = useState(false);
  const [skippedWindows, setSkippedWindows] = useState(0);
  /** Why the automatic notes job failed, so the retry says what to expect. */
  const [notesError, setNotesError] = useState<string | null>(null);
  /** Whether the template picker is showing. Hidden by default: the notes are
   *  already written by the time anyone reads this page, so a template is an
   *  override for the minority case rather than a step on the way in. */
  const [showTemplates, setShowTemplates] = useState(false);

  const isLive = recordingMeetingId === meetingId;

  const refreshSpeakers = useCallback(() => {
    void getMeetingSpeakers(meetingId)
      .then(setSpeakers)
      .catch(() => {});
  }, [meetingId]);

  useEffect(() => {
    let cancelled = false;
    setLoaded(false);
    setItems([]);
    setTranscriptLoading(true);

    void getMeeting(meetingId)
      .then((result) => {
        if (cancelled) return;
        setMeeting(result);
        setLoaded(true);
        if (!result) return;
        setMyNotes(result.my_notes);
        if (isTemplateId(result.notes_template))
          setTemplate(result.notes_template);
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setLoaded(true);
        toast.error(t("meetings.errors.loadFailed", { error: String(error) }));
      });

    refreshSpeakers();

    // Pages are fetched back to back rather than on scroll: the transcript is
    // append-only and finished by the time it is read, so offsets are stable and
    // the whole thing is a handful of cheap queries. Windowing is what keeps the
    // DOM small; this only keeps the queries small.
    void (async () => {
      let offset = 0;
      for (let page = 0; page < MAX_TRANSCRIPT_PAGES; page += 1) {
        try {
          const result = await getMeetingSegments(
            meetingId,
            SEGMENT_PAGE_SIZE,
            offset,
          );
          if (cancelled) return;
          const fetched = result.segments.map(itemFromSegment);
          setItems((current) => [...current, ...fetched]);
          offset += result.segments.length;
          if (!result.has_more || result.segments.length === 0) break;
        } catch (error) {
          if (!cancelled) {
            toast.error(
              t("meetings.errors.transcriptFailed", { error: String(error) }),
            );
          }
          break;
        }
      }
      if (!cancelled) setTranscriptLoading(false);
    })();

    return () => {
      cancelled = true;
    };
  }, [meetingId, refreshSpeakers, t]);

  // A meeting opened while it is still recording keeps filling in.
  useEffect(() => {
    if (!isLive) return;
    const unlisten = listen<SegmentEvent>(MEETING_SEGMENT_EVENT, (event) => {
      if (event.payload.meeting_id !== meetingId) return;
      setItems((current) => [...current, itemFromEvent(event.payload)]);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [isLive, meetingId]);

  // Notes are generated automatically when the call ends, so this page has to be
  // able to learn about a job it did not start — the user very often opens the
  // meeting while the summary is still being written.
  useEffect(() => {
    const unlisten = listen<NotesProgress>(
      MEETING_NOTES_PROGRESS_EVENT,
      (event) => {
        const progress = event.payload;
        if (progress.meeting_id !== meetingId) return;
        switch (progress.stage) {
          case "started":
            setGenerating(true);
            setNotesError(null);
            setSkippedWindows(0);
            break;
          case "finished":
            setGenerating(false);
            setSkippedWindows(progress.skipped_windows);
            setMeeting((current) =>
              current ? { ...current, notes: progress.notes } : current,
            );
            // The backend also derives a title from the notes, so re-read the row
            // rather than guessing what it chose.
            void getMeeting(meetingId)
              .then((fresh) => {
                if (fresh) setMeeting(fresh);
              })
              .catch(() => {});
            onChanged();
            break;
          case "failed":
            setGenerating(false);
            setNotesError(progress.error);
            break;
        }
      },
    );
    return () => {
      void unlisten.then((off) => off());
    };
  }, [meetingId, onChanged]);

  /* ── My thoughts: debounced, and flushed rather than dropped ── */

  const pendingNotes = useRef<string | null>(null);
  const notesTimer = useRef<number | null>(null);

  const flushNotes = useCallback(() => {
    if (notesTimer.current !== null) {
      window.clearTimeout(notesTimer.current);
      notesTimer.current = null;
    }
    const value = pendingNotes.current;
    if (value === null) return;
    pendingNotes.current = null;
    setNotesState("saving");
    void setMeetingMyNotes(meetingId, value)
      .then(() => setNotesState("saved"))
      .catch((error: unknown) => {
        setNotesState("idle");
        toast.error(
          t("meetings.errors.notesSaveFailed", { error: String(error) }),
        );
      });
  }, [meetingId, t]);

  const onMyNotesChange = (value: string) => {
    setMyNotes(value);
    setNotesState("saving");
    pendingNotes.current = value;
    if (notesTimer.current !== null) window.clearTimeout(notesTimer.current);
    notesTimer.current = window.setTimeout(flushNotes, NOTES_DEBOUNCE_MS);
  };

  // Unmount is the one moment the debounce would silently lose the last
  // sentence someone typed, so it writes instead of waiting.
  useEffect(
    () => () => {
      if (notesTimer.current !== null) {
        window.clearTimeout(notesTimer.current);
        notesTimer.current = null;
      }
      const value = pendingNotes.current;
      pendingNotes.current = null;
      if (value !== null)
        void setMeetingMyNotes(meetingId, value).catch(() => {});
    },
    [meetingId],
  );

  /* ── title ── */

  const commitTitle = () => {
    const next = titleDraft?.trim() ?? "";
    setTitleDraft(null);
    if (!meeting || !next || next === meeting.title) return;
    void renameMeeting(meetingId, next)
      .then(() => {
        setMeeting((current) =>
          current ? { ...current, title: next } : current,
        );
        onChanged();
      })
      .catch((error: unknown) => {
        toast.error(
          t("meetings.errors.renameFailed", { error: String(error) }),
        );
      });
  };

  /* ── speakers ── */

  const renameSpeaker = (speakerKey: string, displayName: string) => {
    void renameMeetingSpeaker(meetingId, speakerKey, displayName)
      .then(() => {
        setSpeakers((current) =>
          current.some((s) => s.speaker_key === speakerKey)
            ? current.map((s) =>
                s.speaker_key === speakerKey
                  ? { ...s, display_name: displayName }
                  : s,
              )
            : [
                ...current,
                {
                  speaker_key: speakerKey,
                  display_name: displayName,
                  is_me: speakerKey === ME_SPEAKER_KEY,
                },
              ],
        );
      })
      .catch((error: unknown) => {
        toast.error(
          t("meetings.errors.renameSpeakerFailed", { error: String(error) }),
        );
      });
  };

  /* ── summary ── */

  const generate = () => {
    setGenerating(true);
    setSkippedWindows(0);
    setNotesError(null);
    void generateMeetingNotes(meetingId, template)
      .then((result) => {
        setMeeting((current) =>
          current
            ? {
                ...current,
                notes: result.notes,
                notes_template: result.template_id,
              }
            : current,
        );
        setSkippedWindows(result.skipped_windows);
        setShowTemplates(false);
        onChanged();
      })
      .catch((error: unknown) => {
        setNotesError(String(error));
        toast.error(
          t("meetings.errors.generateFailed", { error: String(error) }),
        );
      })
      .finally(() => setGenerating(false));
  };

  const templateOptions = useMemo(
    () =>
      NOTES_TEMPLATES.map((id) => ({
        value: id,
        label: t(`meetings.summary.templates.${id}`),
      })),
    [t],
  );

  /* ── header numbers ── */

  // Speakers who actually said something. The seeded pair exists for every
  // meeting, so counting the table would claim two voices on a recording where
  // system audio failed and only the user was ever captured.
  const spokenSpeakers = useMemo(() => {
    const keys = new Set<string>();
    for (const item of items) {
      if (item.text.trim()) keys.add(item.speakerKey);
    }
    return keys.size;
  }, [items]);

  if (!loaded) {
    return (
      <p className="px-1 py-8 text-center text-[13px] text-muted">
        {t("common.loading")}
      </p>
    );
  }

  if (!meeting) {
    return (
      <p className="px-1 py-8 text-center text-[13px] text-muted">
        {t("meetings.detail.gone")}
      </p>
    );
  }

  const durationMs = meetingDurationMs(meeting.started_at, meeting.ended_at);
  const pill = [
    spokenSpeakers > 0
      ? t("meetings.list.speakers", { count: spokenSpeakers })
      : null,
    durationMs === null ? null : formatClock(durationMs),
  ]
    .filter((part): part is string => Boolean(part))
    .join(" · ");

  return (
    <div className="space-y-4">
      {/* Title + the "2 speakers · 38:59" summary line. */}
      <div className="space-y-2">
        {titleDraft === null ? (
          <div className="flex items-center gap-2">
            <h3 className="min-w-0 truncate text-[15px] font-medium text-ink">
              {meeting.title}
            </h3>
            <button
              type="button"
              onClick={() => setTitleDraft(meeting.title)}
              title={t("meetings.detail.rename")}
              aria-label={t("meetings.detail.rename")}
              className="shrink-0 cursor-pointer rounded-md p-1 text-muted-soft transition-colors hover:bg-ink/6 hover:text-ink"
            >
              <Pencil size={13} />
            </button>
          </div>
        ) : (
          <div className="flex items-center gap-1.5">
            <input
              value={titleDraft}
              autoFocus
              onChange={(event) => setTitleDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") commitTitle();
                if (event.key === "Escape") setTitleDraft(null);
              }}
              aria-label={t("meetings.detail.rename")}
              className="min-w-0 flex-1 rounded-lg border border-hairline-strong bg-surface px-2.5 py-1.5 text-[14px] text-ink focus:border-ink focus:outline-none"
            />
            <button
              type="button"
              onClick={commitTitle}
              title={t("common.save")}
              aria-label={t("common.save")}
              className="cursor-pointer rounded-md p-1.5 text-muted transition-colors hover:bg-ink/6 hover:text-ink"
            >
              <Check size={14} />
            </button>
            <button
              type="button"
              onClick={() => setTitleDraft(null)}
              title={t("common.cancel")}
              aria-label={t("common.cancel")}
              className="cursor-pointer rounded-md p-1.5 text-muted transition-colors hover:bg-ink/6 hover:text-ink"
            >
              <X size={14} />
            </button>
          </div>
        )}

        <div className="flex flex-wrap items-center gap-2">
          {pill && (
            <span className="inline-flex items-center gap-1.5 rounded-md bg-surface-strong px-2 py-0.5 text-[10.5px] font-semibold uppercase tracking-wide text-muted">
              <Users size={11} />
              {pill}
            </span>
          )}
          <span className="text-[11.5px] text-muted-soft">
            {formatDateTime(String(meeting.started_at), i18n.language)}
          </span>
        </div>
      </div>

      {/* Tabs across the top, the same segmented control the History filters use. */}
      <div
        className="inline-flex items-center rounded-lg bg-surface-strong p-0.5"
        role="group"
        aria-label={t("meetings.detail.tabsLabel")}
      >
        {(
          [
            ["summary", "meetings.tabs.summary"],
            ["ask", "meetings.tabs.ask"],
            ["transcript", "meetings.tabs.transcript"],
            ["notes", "meetings.tabs.myNotes"],
          ] as const
        ).map(([value, labelKey]) => (
          <button
            key={value}
            type="button"
            aria-pressed={tab === value}
            onClick={() => {
              // Leaving the notes tab is a good moment to stop trusting a timer.
              if (tab === "notes" && value !== "notes") flushNotes();
              setTab(value);
            }}
            className={`cursor-pointer rounded-[7px] px-3 py-1.5 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${
              tab === value
                ? "bg-surface text-ink shadow-sm"
                : "text-muted hover:text-ink"
            }`}
          >
            {t(labelKey)}
          </button>
        ))}
      </div>

      {tab === "notes" && (
        <div className="rounded-2xl border border-hairline bg-surface elev-card p-4">
          <div className="mb-2 flex items-center justify-between gap-2">
            <p className="text-xs text-muted">
              {t("meetings.myNotes.description")}
            </p>
            <span className="shrink-0 text-[11px] text-muted-soft">
              {notesState === "saving"
                ? t("meetings.myNotes.saving")
                : notesState === "saved"
                  ? t("meetings.myNotes.saved")
                  : null}
            </span>
          </div>
          <Textarea
            value={myNotes}
            onChange={(event) => onMyNotesChange(event.target.value)}
            onBlur={flushNotes}
            placeholder={t("meetings.myNotes.placeholder")}
            className="min-h-[220px] w-full"
          />
        </div>
      )}

      {tab === "transcript" && (
        <div className="space-y-3">
          <SpeakerIdentification
            meetingId={meetingId}
            diarized={meeting.diarized}
            hasSystemAudio={meeting.system_file !== null}
            isLive={isLive}
            onLabelled={() => {
              // Labels changed every system-side row, so both the transcript and
              // the speaker list have to be re-read rather than patched.
              void getMeeting(meetingId)
                .then((fresh) => {
                  if (fresh) setMeeting(fresh);
                })
                .catch(() => {});
              refreshSpeakers();
              void getMeetingSegments(meetingId, SEGMENT_PAGE_SIZE, 0)
                .then((result) =>
                  setItems(result.segments.map(itemFromSegment)),
                )
                .catch(() => {});
              onChanged();
            }}
          />
          <div className="rounded-2xl border border-hairline bg-surface elev-card">
            <TranscriptView
              items={items}
              speakers={speakers}
              onRenameSpeaker={renameSpeaker}
              heightClassName="max-h-[520px]"
              emptyLabel={
                transcriptLoading
                  ? t("common.loading")
                  : t("meetings.transcript.empty")
              }
              resetKey={meetingId}
              footer={
                transcriptLoading && items.length > 0 ? (
                  <p className="py-2 text-center text-[11.5px] text-muted-soft">
                    {t("meetings.transcript.loadingMore")}
                  </p>
                ) : null
              }
            />
          </div>
        </div>
      )}

      {tab === "ask" && (
        <MeetingAskPanel
          meetingId={meetingId}
          hasTranscript={items.length > 0}
          markdown={notesMarkdown}
        />
      )}

      {tab === "summary" && (
        <div className="space-y-3">
          {/* The notes themselves come first. They are written automatically when
              the call ends, so on this page they are the content rather than
              something the user has to go and produce. */}
          <div className="rounded-2xl border border-hairline bg-surface elev-card p-4">
            {generating && !meeting.notes ? (
              <p className="py-6 text-center text-[13px] text-muted">
                {t("meetings.summary.writing")}
              </p>
            ) : meeting.notes ? (
              <div className="text-[13px] leading-relaxed text-body">
                <ReactMarkdown
                  remarkPlugins={[remarkGfm]}
                  components={notesMarkdown}
                >
                  {meeting.notes}
                </ReactMarkdown>
              </div>
            ) : (
              <p className="py-6 text-center text-[13px] text-muted">
                {items.length === 0
                  ? t("meetings.summary.needsTranscript")
                  : t("meetings.summary.empty")}
              </p>
            )}
          </div>

          {skippedWindows > 0 && (
            <div className="rounded-xl border border-amber-500/40 bg-amber-500/10 px-3.5 py-2.5">
              <p className="text-[12.5px] text-amber-700 dark:text-amber-300">
                {t("meetings.summary.skipped", { count: skippedWindows })}
              </p>
            </div>
          )}

          {notesError && (
            <div className="rounded-xl border border-error/40 bg-error/10 px-3.5 py-2.5">
              <p className="text-[12.5px] text-error">
                {t("meetings.summary.failed", { error: notesError })}
              </p>
            </div>
          )}

          {/* Regenerating is the override, so it sits below the notes and its
              template picker stays folded away until asked for. Offering the
              choice up front meant deciding how to shape a summary of a transcript
              nobody had read yet. */}
          {items.length > 0 && (
            <div className="rounded-2xl border border-hairline bg-surface elev-card p-4">
              {showTemplates ? (
                <div className="space-y-3">
                  <div>
                    <p className="mb-1.5 text-[12.5px] font-medium text-ink">
                      {t("meetings.summary.template")}
                    </p>
                    <Dropdown
                      options={templateOptions}
                      selectedValue={template}
                      onSelect={(value) =>
                        setTemplate(value as NotesTemplateId)
                      }
                      disabled={generating}
                    />
                    <p className="mt-1.5 text-[11.5px] text-muted-soft">
                      {t(`meetings.summary.templateHints.${template}`)}
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <Button
                      variant="primary"
                      size="sm"
                      onClick={generate}
                      disabled={generating}
                      className="gap-1.5"
                    >
                      <Sparkles size={13} />
                      {generating
                        ? t("meetings.summary.generating")
                        : t("meetings.summary.regenerate")}
                    </Button>
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => setShowTemplates(false)}
                      disabled={generating}
                    >
                      {t("common.cancel")}
                    </Button>
                  </div>
                </div>
              ) : (
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <p className="text-xs text-muted">
                    {meeting.notes
                      ? t("meetings.summary.autoCaption")
                      : t("meetings.summary.retryCaption")}
                  </p>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() =>
                      meeting.notes ? setShowTemplates(true) : generate()
                    }
                    disabled={generating}
                    className="gap-1.5"
                  >
                    <Sparkles size={13} />
                    {generating
                      ? t("meetings.summary.generating")
                      : meeting.notes
                        ? t("meetings.summary.rewrite")
                        : t("meetings.summary.generate")}
                  </Button>
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
};
