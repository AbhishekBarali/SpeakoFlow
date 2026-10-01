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
import {
  CalendarDays,
  Check,
  ChevronDown,
  Clock,
  Copy,
  FileText,
  Pencil,
  RefreshCw,
  Sparkles,
  Users,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/Button";
import { MenuButton, type MenuItem } from "@/components/ui/Menu";
import { BackLink } from "@/components/ui/Page";
import { Tabs } from "@/components/ui/Tabs";
import {
  DEFAULT_NOTES_TEMPLATE,
  generateMeetingNotes,
  getMeeting,
  getMeetingSegments,
  getMeetingSpeakers,
  isMeetingNotesRunning,
  MEETING_NOTES_PROGRESS_EVENT,
  MEETING_SEGMENT_EVENT,
  NOTES_TEMPLATES,
  renameMeeting,
  renameMeetingSpeaker,
  SEGMENT_PAGE_SIZE,
  setMeetingMyNotes,
  setMeetingNotes,
  type Meeting,
  type MeetingSpeaker,
  type NotesProgress,
  type NotesTemplateId,
  type SegmentEvent,
} from "./api";
import {
  formatDuration,
  itemFromEvent,
  itemFromSegment,
  meetingDurationMs,
  ME_SPEAKER_KEY,
  type TranscriptItem,
} from "./speakers";
import { MeetingAskPanel } from "./MeetingAskPanel";
import {
  DiscussButton,
  EarlierDiscussions,
  useMeetingDiscuss,
} from "./MeetingDiscuss";
import { toggleTaskAt } from "./notesTasks";
import { SpeakerIdentification } from "./SpeakerIdentification";
import { TranscriptView } from "./TranscriptView";

interface MeetingDetailProps {
  meetingId: number;
  /** The meeting recording right now, if any. Opening its detail should keep
   *  filling in as segments land. */
  recordingMeetingId: number | null;
  /** Told when the title changes so the list behind this page agrees with it. */
  onChanged: () => void;
  /** Back to the list. */
  onBack: () => void;
  backLabel: string;
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

/**
 * Markdown for an answer in the Ask tab: the assistant's reply rules at the
 * page's reading size, so an answer reads like the rest of the app rather than
 * like raw syntax.
 */
const chatMarkdown: Components = {
  p: ({ children }) => <p className="mb-3 last:mb-0">{children}</p>,
  ul: ({ children }) => (
    <ul className="mb-3 list-disc space-y-1.5 ps-5 marker:text-muted-soft last:mb-0">
      {children}
    </ul>
  ),
  ol: ({ children }) => (
    <ol className="mb-3 list-decimal space-y-1.5 ps-5 marker:text-muted last:mb-0">
      {children}
    </ol>
  ),
  li: ({ children }) => <li className="ps-1">{children}</li>,
  strong: ({ children }) => (
    <strong className="font-semibold text-ink">{children}</strong>
  ),
  em: ({ children }) => <em className="italic">{children}</em>,
  h1: ({ children }) => (
    <p className="mb-1.5 mt-4 font-semibold text-ink first:mt-0">{children}</p>
  ),
  h2: ({ children }) => (
    <p className="mb-1.5 mt-4 font-semibold text-ink first:mt-0">{children}</p>
  ),
  h3: ({ children }) => (
    <p className="mb-1.5 mt-3 font-semibold text-ink first:mt-0">{children}</p>
  ),
  code: ({ children }) => (
    <code className="rounded bg-mid-gray/15 px-1 py-0.5 font-mono text-[0.85em]">
      {children}
    </code>
  ),
  pre: ({ children }) => (
    <pre className="my-3 overflow-x-auto rounded-lg border border-hairline bg-mid-gray/10 p-3 text-[0.85em] [&_code]:bg-transparent [&_code]:p-0">
      {children}
    </pre>
  ),
  blockquote: ({ children }) => (
    <blockquote className="my-3 border-s-2 border-hairline-strong ps-3 text-muted">
      {children}
    </blockquote>
  ),
};

/** How long the copy button says "Copied". */
const COPIED_MS = 1600;

const isTemplateId = (value: string | null): value is NotesTemplateId =>
  value !== null && NOTES_TEMPLATES.includes(value as NotesTemplateId);

/** The bits of a hast node the task renderer reads. */
type TaskNode = {
  position?: { start: { offset?: number } };
  children?: Array<{
    type: string;
    tagName?: string;
    properties?: { checked?: unknown };
  }>;
};

/**
 * Markdown for the finished notes.
 *
 * Set as a document at reading size: each `##` section is a real heading with
 * a hairline above it, topics are the scannable spine under it, and next steps
 * render as real checkboxes that save, because a task list you can tick off is
 * the part of the notes people actually return to. The headings used to be
 * 11px uppercase labels over 13.5px text, which made the page dense enough to
 * skim past.
 */
const summaryMarkdown = (
  onToggleTask: (offset: number) => void,
  toggleLabel: string,
): Components => ({
  ...chatMarkdown,
  h1: ({ children }) => <SectionHeading>{children}</SectionHeading>,
  h2: ({ children }) => <SectionHeading>{children}</SectionHeading>,
  h3: ({ children }) => (
    <h4 className="mb-1.5 mt-6 text-[0.9375rem] font-semibold text-ink first:mt-0">
      {children}
    </h4>
  ),
  p: ({ children }) => (
    <p className="mb-3.5 text-[0.9375rem] leading-[1.75] text-body last:mb-0">
      {children}
    </p>
  ),
  ul: ({ className, children }) =>
    className?.includes("contains-task-list") ? (
      <ul className="mb-4 space-y-2.5 last:mb-0">{children}</ul>
    ) : (
      <ul className="mb-4 list-disc space-y-2 ps-5 marker:text-muted-soft last:mb-0">
        {children}
      </ul>
    ),
  ol: ({ children }) => (
    <ol className="mb-4 list-decimal space-y-2 ps-5 marker:text-muted last:mb-0">
      {children}
    </ol>
  ),
  li: ({ node, className, children }) => {
    if (!className?.includes("task-list-item"))
      return (
        <li className="ps-1 text-[0.9375rem] leading-[1.7] text-body">
          {children}
        </li>
      );
    const task = node as TaskNode | undefined;
    const box = task?.children?.find(
      (child) => child.type === "element" && child.tagName === "input",
    );
    const checked = Boolean(box?.properties?.checked);
    const offset = task?.position?.start.offset;
    return (
      <li className="flex items-start gap-3 text-[0.9375rem] leading-[1.7] text-body">
        <button
          type="button"
          role="checkbox"
          aria-checked={checked}
          aria-label={toggleLabel}
          title={toggleLabel}
          disabled={offset === undefined}
          onClick={() => offset !== undefined && onToggleTask(offset)}
          className={`mt-[4px] grid h-[18px] w-[18px] shrink-0 cursor-pointer place-items-center rounded-[6px] border transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${
            checked
              ? "border-accent bg-accent text-on-primary"
              : "border-hairline-strong bg-surface hover:border-ink/50"
          }`}
        >
          {checked && <Check className="h-3 w-3" strokeWidth={3} />}
        </button>
        <span
          className={`min-w-0 flex-1 ${checked ? "text-muted line-through decoration-muted-soft" : ""}`}
        >
          {children}
        </span>
      </li>
    );
  },
  // The box is drawn by `li` above; the one GFM emits would be a second,
  // disabled checkbox beside it.
  input: () => null,
});

/** A `##` section of the notes. The first one sits flush with the top of the
 *  document; every later one gets a hairline and room above it. */
const SectionHeading: React.FC<{ children?: React.ReactNode }> = ({
  children,
}) => (
  <h3 className="mb-3 mt-9 border-t border-hairline pt-7 font-display text-[1.0625rem] text-ink first:mt-0 first:border-t-0 first:pt-0">
    {children}
  </h3>
);

/** "Oct 1, 2026, 6:16 PM": the full date, since a detail page has no day
 *  heading above it. */
const formatWhen = (seconds: number, locale: string): string => {
  try {
    return new Intl.DateTimeFormat(locale, {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date(seconds * 1000));
  } catch {
    return new Date(seconds * 1000).toLocaleString();
  }
};

const ICON_ACTION =
  "inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-lg px-2.5 text-[0.8125rem] font-medium text-muted transition-colors hover:bg-ink/6 hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-50 aria-expanded:bg-ink/6 aria-expanded:text-ink";

/**
 * One meeting: what the user thought, what was said, and what it added up to.
 *
 * The transcript is fetched in pages and then windowed, which is two different
 * problems with the same cause — an hour of speech is a thousand segments, and
 * neither one query nor one DOM tree wants all of them at once.
 *
 * The page's title is the meeting's own: it used to sit under a generic
 * "Meeting" heading at 15px, so the one line that says which meeting this is
 * was the smallest text in the header.
 */
export const MeetingDetail: React.FC<MeetingDetailProps> = ({
  meetingId,
  recordingMeetingId,
  onChanged,
  onBack,
  backLabel,
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
    setGenerating(false);
    setNotesError(null);
    setSkippedWindows(0);

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

    // A page opened after the post-call job announced itself would otherwise
    // think nothing is running and offer to start a second, racing job.
    void isMeetingNotesRunning(meetingId)
      .then((running) => {
        if (!cancelled && running) setGenerating(true);
      })
      .catch(() => {});

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
    if (generating) return;
    setGenerating(true);
    setSkippedWindows(0);
    setNotesError(null);
    // Progress and the refreshed title also arrive as events, since the backend
    // announces manual runs the same way as the automatic one. The promise is
    // still handled so this page updates even if the event raced its listener.
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
        void getMeeting(meetingId)
          .then((fresh) => {
            if (fresh) setMeeting(fresh);
          })
          .catch(() => {});
        onChanged();
      })
      .catch((error: unknown) => {
        // Shown in the error box with its own retry, so no toast on top.
        setNotesError(String(error));
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

  /* ── ticking off next steps, and copying ── */

  const notesText = meeting?.notes ?? null;
  const toggleTask = useCallback(
    (offset: number) => {
      if (notesText === null || generating) return;
      const next = toggleTaskAt(notesText, offset);
      if (next === notesText) return;
      // Optimistic: a checkbox that waits for a round trip feels broken.
      setMeeting((current) =>
        current ? { ...current, notes: next } : current,
      );
      void setMeetingNotes(meetingId, next).catch((error: unknown) => {
        setMeeting((current) =>
          current ? { ...current, notes: notesText } : current,
        );
        toast.error(
          t("meetings.errors.notesSaveFailed", { error: String(error) }),
        );
      });
    },
    [notesText, generating, meetingId, t],
  );

  const summaryComponents = useMemo(
    () => summaryMarkdown(toggleTask, t("meetings.summary.toggleTask")),
    [toggleTask, t],
  );

  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), COPIED_MS);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const copyNotes = () => {
    if (!notesText) return;
    void navigator.clipboard
      .writeText(notesText)
      .then(() => setCopied(true))
      .catch((error: unknown) => {
        toast.error(t("meetings.errors.copyFailed", { error: String(error) }));
      });
  };

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

  const discuss = useMeetingDiscuss(
    meetingId,
    isLive || meeting?.status === "recording",
    (meeting?.segment_count ?? 0) > 0 ||
      items.length > 0 ||
      Boolean(meeting?.notes?.trim()),
  );

  const back = <BackLink label={backLabel} onClick={onBack} className="mb-3" />;

  if (!loaded) {
    return (
      <div className="w-full" aria-busy="true">
        {back}
        <div className="h-9 w-2/3 animate-pulse rounded-lg bg-surface-strong/70" />
        <div className="mt-3 h-4 w-1/3 animate-pulse rounded bg-surface-strong/60" />
        <div className="mt-9 h-64 animate-pulse rounded-2xl bg-surface-strong/50" />
        <span className="sr-only">{t("common.loading")}</span>
      </div>
    );
  }

  if (!meeting) {
    return (
      <div className="w-full">
        {back}
        <p className="rounded-2xl border border-dashed border-hairline-strong px-6 py-12 text-center text-sm text-muted">
          {t("meetings.detail.gone")}
        </p>
      </div>
    );
  }

  const durationMs = meetingDurationMs(meeting.started_at, meeting.ended_at);
  const statusPill =
    meeting.status === "interrupted" || meeting.status === "processing"
      ? "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300"
      : meeting.status === "recording"
        ? "border-error/30 bg-error/10 text-error"
        : null;

  const templateItems: MenuItem[] = templateOptions.map((option) => ({
    id: option.value,
    label: option.label,
    hint: t(`meetings.summary.templateHints.${option.value}`),
    checked: option.value === template,
    onSelect: () => setTemplate(option.value),
  }));
  const templateLabel =
    templateOptions.find((option) => option.value === template)?.label ?? "";

  const templatePicker = (
    <MenuButton
      items={templateItems}
      width={300}
      disabled={generating}
      ariaLabel={t("meetings.summary.template")}
      title={t("meetings.summary.template")}
      className={ICON_ACTION}
    >
      <FileText className="h-3.5 w-3.5" aria-hidden="true" />
      {templateLabel}
      <ChevronDown className="h-3.5 w-3.5 opacity-70" aria-hidden="true" />
    </MenuButton>
  );

  return (
    <div className="w-full">
      {back}

      {/* The meeting's own title is the page title, with the facts under it
          and the one action that leaves the page beside it. */}
      <header className="mb-7 flex flex-wrap items-start justify-between gap-x-6 gap-y-4">
        <div className="min-w-0 flex-1">
          {titleDraft === null ? (
            <div className="group flex items-start gap-1.5">
              <h1 className="min-w-0 font-display text-[1.75rem] text-ink [overflow-wrap:anywhere]">
                {meeting.title}
              </h1>
              <button
                type="button"
                onClick={() => setTitleDraft(meeting.title)}
                title={t("meetings.detail.rename")}
                aria-label={t("meetings.detail.rename")}
                className="mt-1.5 grid h-8 w-8 shrink-0 cursor-pointer place-items-center rounded-lg text-muted-soft opacity-0 transition-[opacity,background-color,color] group-hover:opacity-100 hover:bg-ink/6 hover:text-ink focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
              >
                <Pencil className="h-4 w-4" aria-hidden="true" />
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
                className="h-11 min-w-0 flex-1 rounded-xl border border-hairline-strong bg-surface px-3.5 font-display text-[1.25rem] text-ink focus:border-ink focus:outline-none"
              />
              <button
                type="button"
                onClick={commitTitle}
                title={t("common.save")}
                aria-label={t("common.save")}
                className="grid h-10 w-10 cursor-pointer place-items-center rounded-lg text-muted transition-colors hover:bg-ink/6 hover:text-ink"
              >
                <Check className="h-[18px] w-[18px]" />
              </button>
              <button
                type="button"
                onClick={() => setTitleDraft(null)}
                title={t("common.cancel")}
                aria-label={t("common.cancel")}
                className="grid h-10 w-10 cursor-pointer place-items-center rounded-lg text-muted transition-colors hover:bg-ink/6 hover:text-ink"
              >
                <X className="h-[18px] w-[18px]" />
              </button>
            </div>
          )}

          <div className="mt-2.5 flex flex-wrap items-center gap-x-4 gap-y-1.5 text-sm text-muted">
            <span className="inline-flex items-center gap-1.5">
              <CalendarDays
                className="h-4 w-4 text-muted-soft"
                aria-hidden="true"
              />
              {formatWhen(meeting.started_at, i18n.language)}
            </span>
            {durationMs !== null && (
              <span className="inline-flex items-center gap-1.5 tabular-nums">
                <Clock className="h-4 w-4 text-muted-soft" aria-hidden="true" />
                {formatDuration(durationMs, i18n.language)}
              </span>
            )}
            {spokenSpeakers > 0 && (
              <span className="inline-flex items-center gap-1.5">
                <Users className="h-4 w-4 text-muted-soft" aria-hidden="true" />
                {t("meetings.list.speakers", { count: spokenSpeakers })}
              </span>
            )}
            {statusPill && (
              <span
                className={`rounded-full border px-2 py-0.5 text-xs font-medium ${statusPill}`}
              >
                {t(`meetings.status.${meeting.status}`)}
              </span>
            )}
          </div>
        </div>

        <div className="shrink-0">
          <DiscussButton model={discuss} />
        </div>
      </header>

      <Tabs
        label={t("meetings.detail.tabsLabel")}
        value={tab}
        onChange={(value) => {
          // Leaving the notes tab is a good moment to stop trusting a timer.
          if (tab === "notes" && value !== "notes") flushNotes();
          setTab(value);
        }}
        items={(
          [
            ["summary", "meetings.tabs.summary"],
            ["ask", "meetings.tabs.ask"],
            ["transcript", "meetings.tabs.transcript"],
            ["notes", "meetings.tabs.myNotes"],
          ] as const
        ).map(([id, labelKey]) => ({ id, label: t(labelKey) }))}
      />

      <div className="mt-6">
        {tab === "summary" && (
          <div className="space-y-4">
            {skippedWindows > 0 && (
              <div className="rounded-xl border border-amber-500/30 bg-amber-500/[0.07] px-4 py-3 text-sm leading-relaxed text-ink">
                {t("meetings.summary.skipped", { count: skippedWindows })}
              </div>
            )}

            {notesError && !generating && (
              <div
                className="flex flex-wrap items-center gap-3 rounded-xl border border-error/30 bg-error/[0.06] px-4 py-3"
                role="alert"
              >
                <p className="min-w-[12rem] flex-1 text-sm leading-relaxed text-ink">
                  {t("meetings.summary.failed", { error: notesError })}
                </p>
                {items.length > 0 && (
                  <Button variant="secondary" size="sm" onClick={generate}>
                    {t("meetings.summary.tryAgain")}
                  </Button>
                )}
              </div>
            )}

            {/* The notes are the page. They are written automatically when the
                call ends, so the controls that act on them — pick another
                template, rewrite, copy — sit in one quiet row on top of the
                document instead of in a second card below it. */}
            <article className="overflow-hidden rounded-2xl border border-hairline bg-surface elev-card">
              {(meeting.notes || generating) && (
                <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5 border-b border-hairline px-4 py-2 sm:px-5">
                  <span
                    className="min-w-0 flex-1 truncate text-[0.8125rem] text-muted"
                    role={generating ? "status" : undefined}
                  >
                    {generating ? (
                      <span className="inline-flex items-center gap-2 text-body">
                        <Sparkles
                          className="h-3.5 w-3.5 text-accent motion-safe:animate-pulse"
                          aria-hidden="true"
                        />
                        {meeting.notes
                          ? t("meetings.summary.generating")
                          : t("meetings.summary.writing")}
                      </span>
                    ) : (
                      t("meetings.summary.autoCaption")
                    )}
                  </span>
                  <div className="flex shrink-0 items-center gap-0.5">
                    {items.length > 0 && (
                      <>
                        {templatePicker}
                        <button
                          type="button"
                          onClick={generate}
                          disabled={generating}
                          title={t(
                            `meetings.summary.templateHints.${template}`,
                          )}
                          className={ICON_ACTION}
                        >
                          <RefreshCw
                            className={`h-3.5 w-3.5 ${generating ? "animate-spin" : ""}`}
                            aria-hidden="true"
                          />
                          {t("meetings.summary.regenerate")}
                        </button>
                      </>
                    )}
                    {meeting.notes && (
                      <button
                        type="button"
                        onClick={copyNotes}
                        className={ICON_ACTION}
                      >
                        {copied ? (
                          <Check
                            className="h-3.5 w-3.5 text-success"
                            aria-hidden="true"
                          />
                        ) : (
                          <Copy className="h-3.5 w-3.5" aria-hidden="true" />
                        )}
                        {copied
                          ? t("meetings.summary.copied")
                          : t("meetings.summary.copy")}
                      </button>
                    )}
                  </div>
                </div>
              )}

              {meeting.notes ? (
                <div
                  className={`px-6 py-7 transition-opacity select-text sm:px-9 sm:py-8 ${
                    generating ? "opacity-50" : ""
                  }`}
                >
                  <div className="max-w-[70ch]">
                    <ReactMarkdown
                      remarkPlugins={[remarkGfm]}
                      components={summaryComponents}
                    >
                      {meeting.notes}
                    </ReactMarkdown>
                  </div>
                </div>
              ) : generating ? (
                <div className="space-y-3 px-6 py-8 sm:px-9" aria-hidden="true">
                  {/* A skeleton of the document that is coming, so the wait
                      reads as progress rather than as an empty page. */}
                  <div className="h-4 w-28 animate-pulse rounded bg-mid-gray/15" />
                  <div className="h-3.5 w-full animate-pulse rounded bg-mid-gray/10" />
                  <div className="h-3.5 w-11/12 animate-pulse rounded bg-mid-gray/10" />
                  <div className="h-3.5 w-3/4 animate-pulse rounded bg-mid-gray/10" />
                  <div className="!mt-9 h-4 w-36 animate-pulse rounded bg-mid-gray/15" />
                  <div className="h-3.5 w-2/3 animate-pulse rounded bg-mid-gray/10" />
                  <div className="h-3.5 w-4/5 animate-pulse rounded bg-mid-gray/10" />
                </div>
              ) : (
                <div className="flex flex-col items-center px-6 py-12 text-center">
                  <span className="grid h-11 w-11 place-items-center rounded-xl bg-surface-strong text-muted">
                    <FileText className="h-5 w-5" aria-hidden="true" />
                  </span>
                  <p className="mt-4 max-w-sm text-sm leading-relaxed text-muted text-pretty">
                    {items.length === 0
                      ? t("meetings.summary.needsTranscript")
                      : t("meetings.summary.retryCaption")}
                  </p>
                  {items.length > 0 && (
                    <>
                      <div className="mt-5 flex flex-wrap items-center justify-center gap-2">
                        {templatePicker}
                        <Button variant="primary" size="md" onClick={generate}>
                          <Sparkles className="h-4 w-4" aria-hidden="true" />
                          {t("meetings.summary.generate")}
                        </Button>
                      </div>
                      <p className="mt-3 max-w-sm text-xs leading-relaxed text-muted-soft">
                        {t(`meetings.summary.templateHints.${template}`)}
                      </p>
                    </>
                  )}
                </div>
              )}
            </article>
          </div>
        )}

        {tab === "ask" && (
          <>
            <MeetingAskPanel
              meetingId={meetingId}
              hasTranscript={items.length > 0}
              markdown={chatMarkdown}
            />
            <EarlierDiscussions model={discuss} className="mt-8" />
          </>
        )}

        {tab === "transcript" && (
          <div className="space-y-4">
            <SpeakerIdentification
              meetingId={meetingId}
              diarized={meeting.diarized}
              hasSystemAudio={meeting.system_file !== null}
              isLive={isLive}
              onLabelled={() => {
                // Labels changed every system-side row, so both the transcript
                // and the speaker list have to be re-read rather than patched.
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
            <div className="overflow-hidden rounded-2xl border border-hairline bg-surface elev-card">
              <TranscriptView
                items={items}
                speakers={speakers}
                onRenameSpeaker={renameSpeaker}
                stickToBottom={isLive}
                heightClassName="max-h-[min(42rem,calc(100vh-17rem))]"
                emptyLabel={
                  transcriptLoading
                    ? t("common.loading")
                    : t("meetings.transcript.empty")
                }
                resetKey={meetingId}
                footer={
                  transcriptLoading && items.length > 0 ? (
                    <p className="pb-4 text-center text-xs text-muted-soft">
                      {t("meetings.transcript.loadingMore")}
                    </p>
                  ) : null
                }
              />
            </div>
          </div>
        )}

        {tab === "notes" && (
          <div className="overflow-hidden rounded-2xl border border-hairline bg-surface elev-card transition-colors focus-within:border-hairline-strong">
            <div className="flex items-center justify-between gap-3 border-b border-hairline px-5 py-3">
              <p className="min-w-0 text-[0.8125rem] leading-relaxed text-muted">
                {t("meetings.myNotes.description")}
              </p>
              <span
                className="inline-flex shrink-0 items-center gap-1 text-xs text-muted-soft"
                role="status"
              >
                {notesState === "saved" && (
                  <Check
                    className="h-3.5 w-3.5 text-success"
                    aria-hidden="true"
                  />
                )}
                {notesState === "saving"
                  ? t("meetings.myNotes.saving")
                  : notesState === "saved"
                    ? t("meetings.myNotes.saved")
                    : null}
              </span>
            </div>
            <textarea
              value={myNotes}
              onChange={(event) => onMyNotesChange(event.target.value)}
              onBlur={flushNotes}
              placeholder={t("meetings.myNotes.placeholder")}
              aria-label={t("meetings.tabs.myNotes")}
              className="block min-h-[22rem] w-full resize-y bg-transparent px-6 py-5 text-[0.9375rem] leading-[1.75] text-ink placeholder:text-muted-soft select-text focus:outline-none sm:px-7"
            />
          </div>
        )}
      </div>
    </div>
  );
};
