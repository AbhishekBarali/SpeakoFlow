import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { readFile } from "@tauri-apps/plugin-fs";
import {
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  FolderOpen,
  Camera,
  MessageCircle,
  RotateCcw,
  Star,
  Trash2,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import ReactMarkdown, { type Components } from "react-markdown";
import { toast } from "sonner";
import {
  commands,
  events,
  type AssistantHistoryEntry,
  type HistoryEntry,
  type HistoryUpdatePayload,
} from "@/bindings";
import { useOsType } from "@/hooks/useOsType";
import { formatDateTime } from "@/utils/dateFormat";
import { AudioPlayer } from "../../ui/AudioPlayer";
import { Button } from "../../ui/Button";

/** Must match SCREENSHOT_MARKER in src-tauri/src/assistant.rs */
const SCREENSHOT_MARKER = "[screenshot attached]";

/** Strip the screenshot marker the backend appends to stored user messages. */
const cleanMessageContent = (
  raw: string,
): { text: string; screenshot: boolean } => {
  if (raw.endsWith(SCREENSHOT_MARKER)) {
    return {
      text: raw.slice(0, -SCREENSHOT_MARKER.length).trimEnd(),
      screenshot: true,
    };
  }
  return { text: raw, screenshot: false };
};

/**
 * Markdown styling for assistant replies in the expanded conversation —
 * mirrors the assistant panel so bold, lists, code, etc. render properly
 * instead of leaking raw markdown syntax.
 */
const assistantMarkdown: Components = {
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
    <p className="mb-1 mt-2 font-semibold first:mt-0">{children}</p>
  ),
  h2: ({ children }) => (
    <p className="mb-1 mt-2 font-semibold first:mt-0">{children}</p>
  ),
  h3: ({ children }) => (
    <p className="mb-1 mt-2 font-semibold first:mt-0">{children}</p>
  ),
  a: ({ href, children }) => (
    <a
      href={href}
      target="_blank"
      rel="noreferrer noopener"
      className="underline decoration-hairline-strong underline-offset-2 hover:text-ink"
    >
      {children}
    </a>
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

const IconButton: React.FC<{
  onClick: () => void;
  title: string;
  disabled?: boolean;
  active?: boolean;
  children: React.ReactNode;
}> = ({ onClick, title, disabled, active, children }) => (
  <button
    onClick={onClick}
    disabled={disabled}
    className={`p-1.5 rounded-lg flex items-center justify-center transition-colors cursor-pointer disabled:cursor-not-allowed disabled:text-muted-soft/50 ${
      active ? "text-ink hover:text-ink/80" : "text-muted hover:text-ink"
    }`}
    title={title}
  >
    {children}
  </button>
);

const PAGE_SIZE = 30;

interface OpenRecordingsButtonProps {
  onClick: () => void;
  label: string;
}

const OpenRecordingsButton: React.FC<OpenRecordingsButtonProps> = ({
  onClick,
  label,
}) => (
  <Button
    onClick={onClick}
    variant="secondary"
    size="sm"
    className="flex items-center gap-2"
    title={label}
  >
    <FolderOpen className="w-4 h-4" />
    <span>{label}</span>
  </Button>
);

/**
 * A single item in the unified history feed. Transcriptions and assistant
 * conversations are interleaved by time; `sortTime` is the seconds-epoch used
 * for ordering (last activity for conversations, recording time otherwise).
 */
type FeedItem =
  | { kind: "transcription"; sortTime: number; entry: HistoryEntry }
  | { kind: "assistant"; sortTime: number; session: AssistantHistoryEntry };

export const HistorySettings: React.FC = () => {
  const { t } = useTranslation();
  const osType = useOsType();
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [hasMore, setHasMore] = useState(true);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const entriesRef = useRef<HistoryEntry[]>([]);
  const loadingRef = useRef(false);

  // Assistant conversations are stored separately from transcriptions, so we
  // load them as their own list and merge for display. They are few and
  // capped on the backend, so a single fetch (no pagination) is enough.
  const [assistantSessions, setAssistantSessions] = useState<
    AssistantHistoryEntry[]
  >([]);
  const [assistantLoaded, setAssistantLoaded] = useState(false);
  const [expandedAssistant, setExpandedAssistant] = useState<Set<number>>(
    new Set(),
  );

  // Keep ref in sync for use in IntersectionObserver callback
  useEffect(() => {
    entriesRef.current = entries;
  }, [entries]);

  const loadPage = useCallback(async (cursor?: number) => {
    const isFirstPage = cursor === undefined;
    if (!isFirstPage && loadingRef.current) return;
    loadingRef.current = true;

    if (isFirstPage) setLoading(true);

    try {
      const result = await commands.getHistoryEntries(
        cursor ?? null,
        PAGE_SIZE,
      );
      if (result.status === "ok") {
        const { entries: newEntries, has_more } = result.data;
        setEntries((prev) =>
          isFirstPage ? newEntries : [...prev, ...newEntries],
        );
        setHasMore(has_more);
      }
    } catch (error) {
      console.error("Failed to load history entries:", error);
    } finally {
      setLoading(false);
      loadingRef.current = false;
    }
  }, []);

  const loadAssistantSessions = useCallback(async () => {
    try {
      const result = await commands.getAssistantHistoryEntries(null, null);
      if (result.status === "ok") {
        setAssistantSessions(result.data.entries);
      }
    } catch (error) {
      console.error("Failed to load assistant history:", error);
    } finally {
      setAssistantLoaded(true);
    }
  }, []);

  // Initial load
  useEffect(() => {
    loadPage();
    loadAssistantSessions();
  }, [loadPage, loadAssistantSessions]);

  // Infinite scroll via IntersectionObserver. Pagination tracks only
  // transcriptions (cursor = last transcription id); assistant sessions are
  // already fully loaded, so they just interleave into the sorted feed.
  useEffect(() => {
    if (loading) return;

    const sentinel = sentinelRef.current;
    if (!sentinel || !hasMore) return;

    const observer = new IntersectionObserver(
      (observerEntries) => {
        const first = observerEntries[0];
        if (first.isIntersecting) {
          const lastEntry = entriesRef.current[entriesRef.current.length - 1];
          if (lastEntry) {
            loadPage(lastEntry.id);
          }
        }
      },
      { threshold: 0 },
    );

    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [loading, hasMore, loadPage]);

  // Listen for new entries added from the transcription pipeline
  useEffect(() => {
    const unlisten = events.historyUpdatePayload.listen((event) => {
      const payload: HistoryUpdatePayload = event.payload;
      if (payload.action === "added") {
        setEntries((prev) => [payload.entry, ...prev]);
      } else if (payload.action === "updated") {
        setEntries((prev) =>
          prev.map((e) => (e.id === payload.entry.id ? payload.entry : e)),
        );
      }
      // "deleted" and "toggled" are handled by optimistic updates only,
      // so we intentionally ignore them here to avoid double-mutation.
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Listen for assistant conversation changes (the panel is a separate window,
  // so a turn there can't update this list directly). Refetch on each signal —
  // expansion state is keyed by id, so it survives the reload.
  useEffect(() => {
    const unlisten = listen("assistant-history-updated", () => {
      loadAssistantSessions();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [loadAssistantSessions]);

  const toggleSaved = async (id: number) => {
    // Optimistic update
    setEntries((prev) =>
      prev.map((e) => (e.id === id ? { ...e, saved: !e.saved } : e)),
    );
    try {
      const result = await commands.toggleHistoryEntrySaved(id);
      if (result.status !== "ok") {
        // Revert on failure
        setEntries((prev) =>
          prev.map((e) => (e.id === id ? { ...e, saved: !e.saved } : e)),
        );
      }
    } catch (error) {
      console.error("Failed to toggle saved status:", error);
      // Revert on failure
      setEntries((prev) =>
        prev.map((e) => (e.id === id ? { ...e, saved: !e.saved } : e)),
      );
    }
  };

  const copyToClipboard = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch (error) {
      console.error("Failed to copy to clipboard:", error);
    }
  };

  const getAudioUrl = useCallback(
    async (fileName: string) => {
      try {
        const result = await commands.getAudioFilePath(fileName);
        if (result.status === "ok") {
          if (osType === "linux") {
            const fileData = await readFile(result.data);
            const blob = new Blob([fileData], { type: "audio/wav" });
            return URL.createObjectURL(blob);
          }
          return convertFileSrc(result.data, "asset");
        }
        return null;
      } catch (error) {
        console.error("Failed to get audio file path:", error);
        return null;
      }
    },
    [osType],
  );

  const deleteAudioEntry = async (id: number) => {
    // Optimistically remove
    setEntries((prev) => prev.filter((e) => e.id !== id));
    try {
      const result = await commands.deleteHistoryEntry(id);
      if (result.status !== "ok") {
        // Reload on failure
        loadPage();
      }
    } catch (error) {
      console.error("Failed to delete entry:", error);
      loadPage();
    }
  };

  const retryHistoryEntry = async (id: number) => {
    const result = await commands.retryHistoryEntryTranscription(id);
    if (result.status !== "ok") {
      throw new Error(String(result.error));
    }
  };

  const toggleExpandAssistant = useCallback((id: number) => {
    setExpandedAssistant((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }, []);

  const copyConversation = useCallback(
    (session: AssistantHistoryEntry) => {
      const text = session.messages
        .map((message) => {
          const { text: body } = cleanMessageContent(message.content);
          const label =
            message.role === "user"
              ? t("settings.history.roleUser")
              : t("settings.history.roleAssistant");
          return `${label}: ${body}`;
        })
        .join("\n\n");
      void copyToClipboard(text);
    },
    [t],
  );

  const deleteAssistantSession = useCallback(
    async (id: number) => {
      // Optimistically remove
      setAssistantSessions((prev) => prev.filter((s) => s.id !== id));
      setExpandedAssistant((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
      const result = await commands.deleteAssistantHistoryEntry(id);
      if (result.status !== "ok") {
        // Reload on failure to restore the optimistic removal
        loadAssistantSessions();
        throw new Error(String(result.error));
      }
    },
    [loadAssistantSessions],
  );

  const openRecordingsFolder = async () => {
    try {
      const result = await commands.openRecordingsFolder();
      if (result.status !== "ok") {
        throw new Error(String(result.error));
      }
    } catch (error) {
      console.error("Failed to open recordings folder:", error);
    }
  };

  // Merge transcriptions and assistant conversations into a single feed,
  // newest activity first.
  const feed = useMemo<FeedItem[]>(() => {
    const items: FeedItem[] = [];
    for (const entry of entries) {
      items.push({ kind: "transcription", sortTime: entry.timestamp, entry });
    }
    for (const session of assistantSessions) {
      items.push({
        kind: "assistant",
        sortTime: session.updated_at,
        session,
      });
    }
    items.sort((a, b) => b.sortTime - a.sortTime);
    return items;
  }, [entries, assistantSessions]);

  let content: React.ReactNode;

  if (loading || !assistantLoaded) {
    content = (
      <div className="px-4 py-3 text-center text-text/60">
        {t("settings.history.loading")}
      </div>
    );
  } else if (feed.length === 0) {
    content = (
      <div className="px-4 py-3 text-center text-text/60">
        {t("settings.history.empty")}
      </div>
    );
  } else {
    content = (
      <>
        <div className="divide-y divide-mid-gray/20">
          {feed.map((item) =>
            item.kind === "transcription" ? (
              <HistoryEntryComponent
                key={`t-${item.entry.id}`}
                entry={item.entry}
                onToggleSaved={() => toggleSaved(item.entry.id)}
                onCopyText={() =>
                  copyToClipboard(item.entry.transcription_text)
                }
                getAudioUrl={getAudioUrl}
                deleteAudio={deleteAudioEntry}
                retryTranscription={retryHistoryEntry}
              />
            ) : (
              <AssistantHistoryEntryComponent
                key={`a-${item.session.id}`}
                session={item.session}
                expanded={expandedAssistant.has(item.session.id)}
                onToggleExpand={() => toggleExpandAssistant(item.session.id)}
                onCopyConversation={() => copyConversation(item.session)}
                onDelete={() => deleteAssistantSession(item.session.id)}
              />
            ),
          )}
        </div>
        {/* Sentinel for infinite scroll */}
        <div ref={sentinelRef} className="h-1" />
      </>
    );
  }

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <div className="space-y-2">
        <div className="px-1 flex items-center justify-between">
          <div>
            <h2 className="text-[11px] font-semibold text-muted uppercase tracking-[0.14em]">
              {t("settings.history.title")}
            </h2>
          </div>
          <OpenRecordingsButton
            onClick={openRecordingsFolder}
            label={t("settings.history.openFolder")}
          />
        </div>
        <div className="bg-surface border border-hairline rounded-2xl overflow-visible shadow-[0_1px_2px_rgba(12,10,9,0.04)]">
          {content}
        </div>
      </div>
    </div>
  );
};

interface HistoryEntryProps {
  entry: HistoryEntry;
  onToggleSaved: () => void;
  onCopyText: () => void;
  getAudioUrl: (fileName: string) => Promise<string | null>;
  deleteAudio: (id: number) => Promise<void>;
  retryTranscription: (id: number) => Promise<void>;
}

const HistoryEntryComponent: React.FC<HistoryEntryProps> = ({
  entry,
  onToggleSaved,
  onCopyText,
  getAudioUrl,
  deleteAudio,
  retryTranscription,
}) => {
  const { t, i18n } = useTranslation();
  const [showCopied, setShowCopied] = useState(false);
  const [retrying, setRetrying] = useState(false);

  const hasTranscription = entry.transcription_text.trim().length > 0;

  const handleLoadAudio = useCallback(
    () => getAudioUrl(entry.file_name),
    [getAudioUrl, entry.file_name],
  );

  const handleCopyText = () => {
    if (!hasTranscription) {
      return;
    }

    onCopyText();
    setShowCopied(true);
    setTimeout(() => setShowCopied(false), 2000);
  };

  const handleDeleteEntry = async () => {
    try {
      await deleteAudio(entry.id);
    } catch (error) {
      console.error("Failed to delete entry:", error);
      toast.error(t("settings.history.deleteError"));
    }
  };

  const handleRetranscribe = async () => {
    try {
      setRetrying(true);
      await retryTranscription(entry.id);
    } catch (error) {
      console.error("Failed to re-transcribe:", error);
      toast.error(t("settings.history.retranscribeError"));
    } finally {
      setRetrying(false);
    }
  };

  const formattedDate = formatDateTime(String(entry.timestamp), i18n.language);

  return (
    <div className="px-4 py-2 pb-5 flex flex-col gap-3">
      <div className="flex justify-between items-center">
        <p className="text-sm font-medium">{formattedDate}</p>
        <div className="flex items-center">
          <IconButton
            onClick={handleCopyText}
            disabled={!hasTranscription || retrying}
            title={t("settings.history.copyToClipboard")}
          >
            {showCopied ? (
              <Check width={16} height={16} />
            ) : (
              <Copy width={16} height={16} />
            )}
          </IconButton>
          <IconButton
            onClick={onToggleSaved}
            disabled={retrying}
            active={entry.saved}
            title={
              entry.saved
                ? t("settings.history.unsave")
                : t("settings.history.save")
            }
          >
            <Star
              width={16}
              height={16}
              fill={entry.saved ? "currentColor" : "none"}
            />
          </IconButton>
          <IconButton
            onClick={handleRetranscribe}
            disabled={retrying}
            title={t("settings.history.retranscribe")}
          >
            <RotateCcw
              width={16}
              height={16}
              style={
                retrying
                  ? { animation: "spin 1s linear infinite reverse" }
                  : undefined
              }
            />
          </IconButton>
          <IconButton
            onClick={handleDeleteEntry}
            disabled={retrying}
            title={t("settings.history.delete")}
          >
            <Trash2 width={16} height={16} />
          </IconButton>
        </div>
      </div>

      <p
        className={`italic text-sm pb-2 ${
          retrying
            ? ""
            : hasTranscription
              ? "text-text/90 select-text cursor-text whitespace-pre-wrap break-words"
              : "text-text/40"
        }`}
        style={
          retrying
            ? { animation: "transcribe-pulse 3s ease-in-out infinite" }
            : undefined
        }
      >
        {retrying && (
          <style>{`
            @keyframes transcribe-pulse {
              0%, 100% { color: color-mix(in srgb, var(--color-text) 40%, transparent); }
              50% { color: color-mix(in srgb, var(--color-text) 90%, transparent); }
            }
          `}</style>
        )}
        {retrying
          ? t("settings.history.transcribing")
          : hasTranscription
            ? entry.transcription_text
            : t("settings.history.transcriptionFailed")}
      </p>

      <AudioPlayer onLoadRequest={handleLoadAudio} className="w-full" />
    </div>
  );
};

interface AssistantHistoryEntryProps {
  session: AssistantHistoryEntry;
  expanded: boolean;
  onToggleExpand: () => void;
  onCopyConversation: () => void;
  onDelete: () => Promise<void>;
}

/**
 * Assistant conversations render as collapsible entries: a header with the
 * date and an "Assistant" badge, a one-line preview when collapsed, and the
 * full turn-by-turn transcript when expanded. No audio or re-transcribe
 * controls — these are chats, not recordings.
 */
const AssistantHistoryEntryComponent: React.FC<AssistantHistoryEntryProps> = ({
  session,
  expanded,
  onToggleExpand,
  onCopyConversation,
  onDelete,
}) => {
  const { t, i18n } = useTranslation();
  const [showCopied, setShowCopied] = useState(false);

  const formattedDate = formatDateTime(
    String(session.updated_at),
    i18n.language,
  );

  const handleCopy = () => {
    onCopyConversation();
    setShowCopied(true);
    setTimeout(() => setShowCopied(false), 2000);
  };

  const handleDelete = async () => {
    try {
      await onDelete();
    } catch (error) {
      console.error("Failed to delete assistant conversation:", error);
      toast.error(t("settings.history.deleteAssistantError"));
    }
  };

  return (
    <div className="px-4 py-2 pb-5 flex flex-col gap-3">
      <div className="flex justify-between items-center gap-2">
        <button
          onClick={onToggleExpand}
          className="flex items-center gap-2 min-w-0 text-left cursor-pointer text-muted hover:text-ink transition-colors"
          title={
            expanded
              ? t("settings.history.hideConversation")
              : t("settings.history.showConversation")
          }
        >
          {expanded ? (
            <ChevronDown width={15} height={15} className="shrink-0" />
          ) : (
            <ChevronRight width={15} height={15} className="shrink-0" />
          )}
          <span className="text-sm font-medium text-ink">{formattedDate}</span>
          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-full text-[10px] font-medium uppercase tracking-wide bg-mid-gray/15 text-muted shrink-0">
            <MessageCircle width={10} height={10} />
            {t("settings.history.assistantLabel")}
          </span>
        </button>
        <div className="flex items-center shrink-0">
          <IconButton
            onClick={handleCopy}
            title={t("settings.history.copyConversation")}
          >
            {showCopied ? (
              <Check width={16} height={16} />
            ) : (
              <Copy width={16} height={16} />
            )}
          </IconButton>
          <IconButton
            onClick={handleDelete}
            title={t("settings.history.delete")}
          >
            <Trash2 width={16} height={16} />
          </IconButton>
        </div>
      </div>

      {!expanded && (
        <button
          onClick={onToggleExpand}
          className="text-left cursor-pointer flex flex-col gap-1"
        >
          <p className="text-sm text-text/90 line-clamp-2 break-words">
            {session.title}
          </p>
          <span className="text-xs text-muted">
            {t("settings.history.messageCount", {
              count: session.messages.length,
            })}
          </span>
        </button>
      )}

      {expanded && (
        <div className="flex flex-col gap-2 pt-0.5">
          {session.messages.map((message, index) => {
            const { text, screenshot } = cleanMessageContent(message.content);
            const isUser = message.role === "user";
            return (
              <div
                key={index}
                className={`flex ${isUser ? "justify-end" : "justify-start"}`}
              >
                <div
                  className={
                    isUser
                      ? "max-w-[85%] rounded-2xl rounded-br-md bg-ink-soft px-3 py-2 text-sm leading-relaxed text-on-primary select-text whitespace-pre-wrap break-words"
                      : "max-w-[85%] rounded-2xl rounded-bl-md border border-hairline bg-surface-strong px-3 py-2 text-sm leading-relaxed text-ink select-text break-words"
                  }
                >
                  {isUser ? (
                    text
                  ) : (
                    <ReactMarkdown components={assistantMarkdown}>
                      {text}
                    </ReactMarkdown>
                  )}
                  {screenshot && (
                    <span
                      className={`mt-1.5 inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium ${
                        isUser
                          ? "bg-on-primary/15 text-on-primary/80"
                          : "bg-mid-gray/15 text-muted"
                      }`}
                    >
                      <Camera width={10} height={10} />
                      {t("settings.history.screenshotAttached")}
                    </span>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
