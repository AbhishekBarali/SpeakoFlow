import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { usePortalTarget } from "@/components/ui/portal";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { readFile } from "@tauri-apps/plugin-fs";
import {
  ArrowUpRight,
  Check,
  ChevronDown,
  Copy,
  FolderOpen,
  Camera,
  FileText,
  GitBranch,
  HardDrive,
  MessageCircle,
  Mic,
  Pause,
  Play,
  RotateCcw,
  Sparkles,
  Star,
  Trash2,
  Wand2,
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
import { useSettings } from "@/hooks/useSettings";
import { AudioPlayer } from "../../ui/AudioPlayer";
import { Button } from "../../ui/Button";
import { PageHeader } from "../../ui/Page";
import { Tabs } from "../../ui/Tabs";
import { useNavigation } from "../../shell/navigation";
import { VOICE_INTERRUPTED_MARKER } from "@/assistant/conversationPolicy";

/** Must match the marker constants in src-tauri/src/assistant.rs */
const SCREENSHOT_MARKER = "[screenshot attached]";
const IMAGE_MARKER = "[image attached]";
const FILE_MARKER_PREFIX = "[file attached:";

/** Stable marker written by src-tauri/src/flow.rs. Existing successful Flow
 *  rows already carry this value, so they appear in the new filter too. */
const FLOW_HISTORY_MARKER = "Generate with Flow";

const isFlowHistoryEntry = (entry: HistoryEntry): boolean =>
  entry.post_process_prompt === FLOW_HISTORY_MARKER;

/** Strip the attachment markers the backend appends to stored user messages,
 *  returning the clean text plus what rode along (screen capture / files). */
const cleanMessageContent = (
  raw: string,
): { text: string; screenshot: boolean; files: string[] } => {
  let screenshot = false;
  const files: string[] = [];
  const kept: string[] = [];
  for (const line of raw.split("\n")) {
    const trimmed = line.trim();
    if (trimmed === VOICE_INTERRUPTED_MARKER) continue;
    if (trimmed === SCREENSHOT_MARKER) {
      screenshot = true;
      continue;
    }
    if (trimmed === IMAGE_MARKER) {
      continue;
    }
    if (trimmed.startsWith(FILE_MARKER_PREFIX) && trimmed.endsWith("]")) {
      files.push(trimmed.slice(FILE_MARKER_PREFIX.length, -1).trim());
      continue;
    }
    kept.push(line);
  }
  return { text: kept.join("\n").trim(), screenshot, files };
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
    className={`p-1.5 rounded-md flex items-center justify-center transition-colors cursor-pointer hover:bg-ink/6 disabled:cursor-not-allowed disabled:text-muted-soft/50 disabled:hover:bg-transparent ${
      active ? "text-ink" : "text-muted hover:text-ink"
    }`}
    title={title}
  >
    {children}
  </button>
);

/** Thumbnails of the image(s) sent with a stored message — the screen capture
 *  (badged) and/or attached pictures. Click one to pop a full-size lightbox
 *  (click anywhere, or Esc, to close). The compact thumbnails are what the app
 *  persists in history; the full-resolution frame only ever went to the model. */
const HistoryThumbnails: React.FC<{
  urls: string[];
  hasScreen?: boolean;
  isUser: boolean;
  screenLabel: string;
}> = ({ urls, hasScreen, isUser, screenLabel }) => {
  const [open, setOpen] = useState<string | null>(null);
  const [shown, setShown] = useState(false);
  const portalTarget = usePortalTarget();

  useEffect(() => {
    if (!open) return;
    setShown(false);
    const raf = requestAnimationFrame(() => setShown(true));
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(null);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <>
      <div className="mt-2 flex flex-wrap gap-1.5">
        {urls.map((url, i) => (
          <button
            key={i}
            type="button"
            onClick={() => setOpen(url)}
            aria-label={hasScreen && i === 0 ? screenLabel : undefined}
            className={`relative h-12 w-16 cursor-zoom-in overflow-hidden rounded-lg border transition-transform hover:-translate-y-0.5 active:scale-95 ${
              isUser ? "border-on-primary/25" : "border-hairline"
            }`}
          >
            <img
              src={url}
              alt=""
              draggable={false}
              className="h-full w-full object-cover"
            />
            {hasScreen && i === 0 && (
              <span className="absolute bottom-0.5 end-0.5 flex h-3.5 w-3.5 items-center justify-center rounded bg-black/60 text-white">
                <Camera width={9} height={9} />
              </span>
            )}
          </button>
        ))}
      </div>
      {open &&
        portalTarget &&
        createPortal(
          <div
            className="fixed inset-0 z-[100] flex cursor-zoom-out items-center justify-center bg-black/70 p-10"
            onClick={() => setOpen(null)}
            role="button"
            tabIndex={-1}
          >
            <img
              src={open}
              alt=""
              draggable={false}
              className={`max-h-full max-w-full rounded-xl shadow-2xl transition-all duration-150 ${
                shown ? "scale-100 opacity-100" : "scale-90 opacity-0"
              }`}
            />
          </div>,
          portalTarget,
        )}
    </>
  );
};

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

type HistoryFilter = "all" | "recordings" | "flow" | "assistant";

interface DayGroup {
  key: string;
  label: string;
  items: FeedItem[];
}

const dayKey = (seconds: number): string => {
  const date = new Date(seconds * 1000);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
};

/** "Today", "Yesterday", "Monday", or a date — newest group first. */
const groupFeedByDay = (
  items: FeedItem[],
  locale: string,
  labels: { today: string; yesterday: string },
): DayGroup[] => {
  const now = new Date();
  const today = dayKey(now.getTime() / 1000);
  const yesterday = dayKey(now.getTime() / 1000 - 86_400);
  const weekAgo = now.getTime() - 6 * 86_400_000;
  const groups: DayGroup[] = [];
  for (const item of items) {
    const key = dayKey(item.sortTime);
    let group = groups[groups.length - 1];
    if (!group || group.key !== key) {
      const date = new Date(item.sortTime * 1000);
      let label: string;
      if (key === today) label = labels.today;
      else if (key === yesterday) label = labels.yesterday;
      else {
        try {
          label = new Intl.DateTimeFormat(
            locale,
            date.getTime() >= weekAgo
              ? { weekday: "long" }
              : date.getFullYear() === now.getFullYear()
                ? { weekday: "short", month: "long", day: "numeric" }
                : { year: "numeric", month: "long", day: "numeric" },
          ).format(date);
        } catch {
          label = date.toDateString();
        }
      }
      group = { key, label, items: [] };
      groups.push(group);
    }
    group.items.push(item);
  }
  return groups;
};

export const HistorySettings: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { openSettings, navigate } = useNavigation();
  const { getSetting } = useSettings();
  const assistantEnabled = getSetting("assistant_enabled") ?? true;
  const osType = useOsType();
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<HistoryFilter>("all");
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

  // Re-apply the stored retention policy whenever this panel opens.
  //
  // A time-based policy ("after 3 days", "keep for N days") is otherwise only
  // enforced at launch and right after a new recording is saved, so on a machine
  // where the app stays open, entries that crossed the cutoff were still listed —
  // the retention setting looked like it did nothing. Anything it removes arrives
  // through the `history-retention-applied` listener below, so no extra refetch
  // is needed here.
  useEffect(() => {
    void commands.enforceRecordingRetention();
  }, []);

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

  // Retention commands now clean recordings immediately. Refetch the first
  // page after cleanup so deleted rows disappear without an app restart.
  useEffect(() => {
    const unlisten = listen("history-retention-applied", () => {
      void loadPage();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [loadPage]);

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

  /** Load a past conversation back into the assistant panel and open it. */
  const resumeAssistantSession = useCallback(
    async (id: number) => {
      // With the assistant off the panel cannot open, and the click would
      // otherwise do nothing at all.
      if (!assistantEnabled) {
        toast(t("historyPage.chat.assistantOff"), {
          action: {
            label: t("nav.assistant"),
            onClick: () => navigate("assistant"),
          },
        });
        return;
      }
      const result = await commands.assistantResumeSession(id);
      if (result.status !== "ok") {
        toast.error(String(result.error));
      }
    },
    [assistantEnabled, navigate, t],
  );

  // Continue a past conversation from one message onwards. The original row is
  // never touched — the branch is saved as a new conversation — so this is safe to
  // try on something the user wants to keep.
  const branchAssistantSession = useCallback(
    async (id: number, messageIndex: number) => {
      const result = await commands.assistantBranchSession(id, messageIndex);
      if (result.status !== "ok") {
        toast.error(String(result.error));
      }
    },
    [],
  );

  const openRecordingsFolder = async () => {
    try {
      const result = await commands.openRecordingsFolder();
      if (result.status !== "ok") {
        throw new Error(String(result.error));
      }
    } catch (error) {
      console.error("Failed to open recordings folder:", error);
      toast.error(t("settings.history.openFolderError"));
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

  const filteredFeed = useMemo(
    () =>
      feed.filter((item) => {
        if (filter === "recordings") {
          return (
            item.kind === "transcription" && !isFlowHistoryEntry(item.entry)
          );
        }
        if (filter === "flow") {
          return (
            item.kind === "transcription" && isFlowHistoryEntry(item.entry)
          );
        }
        if (filter === "assistant") return item.kind === "assistant";
        return true;
      }),
    [feed, filter],
  );

  const dayGroups = useMemo(
    () =>
      groupFeedByDay(filteredFeed, i18n.language, {
        today: t("historyPage.today"),
        yesterday: t("historyPage.yesterday"),
      }),
    [filteredFeed, i18n.language, t],
  );

  let content: React.ReactNode;

  if (loading || !assistantLoaded) {
    content = (
      <div className="space-y-3" aria-busy="true">
        {[0, 1, 2].map((index) => (
          <div
            key={index}
            className="h-20 animate-pulse rounded-xl bg-surface-strong/60"
          />
        ))}
        <span className="sr-only">{t("settings.history.loading")}</span>
      </div>
    );
  } else if (filteredFeed.length === 0) {
    const emptyKey =
      filter === "recordings"
        ? "settings.history.emptyRecordings"
        : filter === "flow"
          ? "settings.history.emptyFlow"
          : filter === "assistant"
            ? "settings.history.emptyAssistant"
            : "settings.history.empty";
    content = (
      <div className="rounded-xl border border-dashed border-hairline-strong px-6 py-12 text-center">
        <p className="text-sm text-muted">{t(emptyKey)}</p>
      </div>
    );
  } else {
    content = (
      <>
        <div className="space-y-6">
          {dayGroups.map((group) => (
            <section key={group.key} aria-label={group.label}>
              {/* Sticky so a long day keeps its heading in view. The canvas
                  fill hides rows scrolling underneath it. */}
              <h2 className="sticky top-0 z-10 -mx-1 mb-2 bg-canvas/95 px-1 py-1.5 text-sm font-semibold text-muted backdrop-blur-[2px]">
                {group.label}
              </h2>
              <div className="divide-y divide-hairline overflow-visible rounded-xl border border-hairline bg-surface elev-card">
                {group.items.map((item) =>
                  item.kind === "transcription" ? (
                    <HistoryEntryComponent
                      key={`t-${item.entry.id}`}
                      entry={item.entry}
                      onToggleSaved={() => toggleSaved(item.entry.id)}
                      onCopyText={() =>
                        copyToClipboard(
                          item.entry.post_processed_text?.trim()
                            ? item.entry.post_processed_text
                            : item.entry.transcription_text,
                        )
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
                      onToggleExpand={() =>
                        toggleExpandAssistant(item.session.id)
                      }
                      onCopyConversation={() => copyConversation(item.session)}
                      onDelete={() => deleteAssistantSession(item.session.id)}
                      onResume={() =>
                        void resumeAssistantSession(item.session.id)
                      }
                      onBranch={(messageIndex) =>
                        void branchAssistantSession(
                          item.session.id,
                          messageIndex,
                        )
                      }
                    />
                  ),
                )}
              </div>
            </section>
          ))}
        </div>
        {/* Pagination belongs to recordings; assistant sessions are loaded in one page. */}
        {filter !== "assistant" && <div ref={sentinelRef} className="h-1" />}
      </>
    );
  }

  return (
    <div className="w-full">
      <PageHeader
        title={t("sidebar.history")}
        description={t("sectionSubtitles.history")}
        actions={
          <>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => openSettings("privacy")}
            >
              <HardDrive className="h-3.5 w-3.5" aria-hidden="true" />
              {t("historyPage.storage")}
            </Button>
            <OpenRecordingsButton
              onClick={openRecordingsFolder}
              label={t("settings.history.openFolder")}
            />
          </>
        }
      />
      <Tabs
        label={t("settings.history.filters.label")}
        value={filter}
        onChange={setFilter}
        items={(
          [
            ["all", "settings.history.filters.all"],
            ["recordings", "settings.history.filters.recordings"],
            ["flow", "settings.history.filters.flow"],
            ["assistant", "settings.history.filters.assistant"],
          ] as const
        ).map(([value, labelKey]) => ({ id: value, label: t(labelKey) }))}
      />
      <div className="mt-6">{content}</div>
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

/** "4:07 PM" — the day is already the group heading. */
const formatTimeOfDay = (seconds: number, locale: string): string => {
  try {
    return new Intl.DateTimeFormat(locale, {
      hour: "numeric",
      minute: "2-digit",
    }).format(new Date(seconds * 1000));
  } catch {
    return "";
  }
};

/**
 * One dictation. The text that was actually pasted comes first, because that
 * is what people come back for; what the recogniser heard before cleanup or
 * Flow rewrote it is one click away rather than stacked on top. The recording
 * loads only when asked for, instead of every row carrying a full-width player.
 */
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
  const [showOriginal, setShowOriginal] = useState(false);
  const [audioSrc, setAudioSrc] = useState<string | null>(null);
  const [loadingAudio, setLoadingAudio] = useState(false);

  const hasTranscription = entry.transcription_text.trim().length > 0;
  const flowEntry = isFlowHistoryEntry(entry);
  const processedText = entry.post_processed_text?.trim()
    ? entry.post_processed_text
    : null;
  const hasDistinctProcessedText =
    processedText !== null &&
    processedText.trim() !== entry.transcription_text.trim();
  /**
   * Cleanup ran and deliberately changed nothing.
   *
   * `post_processed_text` is present-but-identical in that case, and absent when
   * cleanup never ran, so the two are distinguishable. With a restrained cleanup
   * model that returns already-correct dictation byte for byte (the common case,
   * and the point of a cleanup fine-tune) the feature looked broken every time it
   * worked perfectly unless the row says so.
   */
  const cleanupMadeNoChanges =
    !flowEntry && processedText !== null && !hasDistinctProcessedText;
  // What was pasted, and what it was made from (when those differ).
  const finalText = flowEntry
    ? processedText
    : hasDistinctProcessedText
      ? processedText
      : hasTranscription
        ? entry.transcription_text
        : null;
  const originalText =
    (flowEntry || hasDistinctProcessedText) && hasTranscription
      ? entry.transcription_text
      : null;
  const hasCopyableText = finalText !== null || hasTranscription;

  const handleCopyText = () => {
    if (!hasCopyableText) return;
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

  const toggleAudio = async () => {
    if (audioSrc) {
      // Unmounting the player releases a Linux blob URL, so the next play has
      // to fetch a fresh one.
      setAudioSrc(null);
      return;
    }
    setLoadingAudio(true);
    try {
      const url = await getAudioUrl(entry.file_name);
      if (url) setAudioSrc(url);
      else toast.error(t("historyPage.audioMissing"));
    } finally {
      setLoadingAudio(false);
    }
  };

  const kindLabel = flowEntry
    ? t("settings.history.flowLabel")
    : t("settings.history.recordingLabel");
  const KindIcon = flowEntry ? Sparkles : Mic;

  return (
    <div className="group px-4 py-3.5">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          {retrying && (
            <style>{`
              @keyframes transcribe-pulse {
                0%, 100% { color: color-mix(in srgb, var(--color-text) 40%, transparent); }
                50% { color: color-mix(in srgb, var(--color-text) 90%, transparent); }
              }
            `}</style>
          )}
          <p
            className={`max-w-[75ch] text-sm leading-relaxed ${
              retrying
                ? ""
                : finalText !== null
                  ? "text-ink select-text cursor-text whitespace-pre-wrap break-words"
                  : "text-muted-soft"
            }`}
            style={
              retrying
                ? { animation: "transcribe-pulse 3s ease-in-out infinite" }
                : undefined
            }
          >
            {retrying
              ? t("settings.history.transcribing")
              : finalText !== null
                ? finalText
                : flowEntry && hasTranscription
                  ? t("settings.history.flowNoOutput")
                  : t("settings.history.transcriptionFailed")}
          </p>

          {showOriginal && originalText && (
            <div className="mt-2.5 rounded-lg border border-hairline bg-canvas px-3 py-2.5">
              <p className="mb-1 text-xs font-medium text-muted">
                {t(
                  flowEntry
                    ? "settings.history.flowTranscriptLabel"
                    : "settings.history.originalTranscriptionLabel",
                )}
              </p>
              <p className="select-text whitespace-pre-wrap break-words text-[0.8125rem] leading-relaxed text-body">
                {originalText}
              </p>
            </div>
          )}

          <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted">
            <span className="tabular-nums">
              {formatTimeOfDay(entry.timestamp, i18n.language)}
            </span>
            <span aria-hidden="true" className="text-muted-soft">
              ·
            </span>
            <span className="inline-flex items-center gap-1">
              <KindIcon width={11} height={11} aria-hidden="true" />
              {kindLabel}
            </span>
            {hasDistinctProcessedText && !flowEntry && (
              <>
                <span aria-hidden="true" className="text-muted-soft">
                  ·
                </span>
                <span className="inline-flex items-center gap-1">
                  <Wand2 width={11} height={11} aria-hidden="true" />
                  {t("historyPage.cleaned")}
                </span>
              </>
            )}
            {cleanupMadeNoChanges && (
              <>
                <span aria-hidden="true" className="text-muted-soft">
                  ·
                </span>
                <span className="inline-flex items-center gap-1">
                  <Wand2 width={11} height={11} aria-hidden="true" />
                  {t("settings.history.cleanupNoChanges")}
                </span>
              </>
            )}
            {originalText && (
              <button
                type="button"
                onClick={() => setShowOriginal((value) => !value)}
                aria-expanded={showOriginal}
                className="inline-flex cursor-pointer items-center gap-0.5 rounded font-medium text-accent hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
              >
                {showOriginal
                  ? t("historyPage.hideOriginal")
                  : flowEntry
                    ? t("historyPage.showSaid")
                    : t("historyPage.showOriginal")}
                <ChevronDown
                  className={`h-3 w-3 transition-transform ${showOriginal ? "rotate-180" : ""}`}
                  aria-hidden="true"
                />
              </button>
            )}
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-0.5 opacity-60 transition-opacity duration-150 group-hover:opacity-100 focus-within:opacity-100">
          <IconButton
            onClick={() => void toggleAudio()}
            disabled={loadingAudio || retrying}
            active={audioSrc !== null}
            title={
              audioSrc
                ? t("historyPage.hideRecording")
                : t("historyPage.playRecording")
            }
          >
            {audioSrc ? (
              <Pause width={14} height={14} />
            ) : (
              <Play width={14} height={14} />
            )}
          </IconButton>
          <IconButton
            onClick={handleCopyText}
            disabled={!hasCopyableText || retrying}
            title={t(
              flowEntry && processedText
                ? "settings.history.copyFlowOutput"
                : hasDistinctProcessedText
                  ? "settings.history.copyFinalText"
                  : "settings.history.copyToClipboard",
            )}
          >
            {showCopied ? (
              <Check width={14} height={14} />
            ) : (
              <Copy width={14} height={14} />
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
              width={14}
              height={14}
              fill={entry.saved ? "currentColor" : "none"}
            />
          </IconButton>
          <IconButton
            onClick={handleRetranscribe}
            disabled={retrying}
            title={t("settings.history.retranscribe")}
          >
            <RotateCcw
              width={14}
              height={14}
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
            <Trash2 width={14} height={14} />
          </IconButton>
        </div>
      </div>

      {audioSrc && (
        <AudioPlayer src={audioSrc} autoPlay className="mt-2.5 w-full" />
      )}
    </div>
  );
};

interface AssistantHistoryEntryProps {
  session: AssistantHistoryEntry;
  expanded: boolean;
  onToggleExpand: () => void;
  onCopyConversation: () => void;
  onDelete: () => Promise<void>;
  onResume: () => void;
  /** Continue from one message as a new conversation, leaving this one intact. */
  onBranch: (messageIndex: number) => void;
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
  onResume,
  onBranch,
}) => {
  const { t, i18n } = useTranslation();
  const [showCopied, setShowCopied] = useState(false);

  const formattedDate = formatTimeOfDay(session.updated_at, i18n.language);

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
    <div className="group flex flex-col gap-1.5 px-4 py-3.5">
      {/* The question, and the way back into the chat. Clicking the question
          opens it too: a past conversation is something you pick up again,
          not a transcript to copy. */}
      <div className="flex items-start gap-3">
        <button
          type="button"
          onClick={onResume}
          title={t("historyPage.chat.continueTitle")}
          className="min-w-0 flex-1 cursor-pointer rounded-md text-start focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
        >
          <span
            className={`block break-words text-sm leading-relaxed text-ink transition-colors group-hover:text-accent ${
              expanded ? "" : "line-clamp-2"
            }`}
          >
            {session.title}
          </span>
        </button>
        <Button
          variant="secondary"
          size="sm"
          onClick={onResume}
          title={t("historyPage.chat.continueTitle")}
        >
          {t("historyPage.chat.continue")}
          <ArrowUpRight className="h-3.5 w-3.5" aria-hidden="true" />
        </Button>
      </div>

      {/* Meta row — quiet caption on the left, actions on the right. */}
      <div className="flex items-center justify-between gap-3">
        <span className="inline-flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted">
          <span className="tabular-nums">{formattedDate}</span>
          <span aria-hidden="true" className="text-muted-soft">
            ·
          </span>
          <span className="inline-flex items-center gap-1">
            <MessageCircle width={11} height={11} aria-hidden="true" />
            {t("settings.history.assistantLabel")}
          </span>
          <span aria-hidden="true" className="text-muted-soft">
            ·
          </span>
          <button
            type="button"
            onClick={onToggleExpand}
            aria-expanded={expanded}
            className="inline-flex cursor-pointer items-center gap-0.5 rounded font-medium text-muted transition-colors hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
          >
            {expanded
              ? t("historyPage.chat.hideMessages")
              : t("settings.history.messageCount", {
                  count: session.messages.length,
                })}
            <ChevronDown
              className={`h-3 w-3 transition-transform duration-150 ${expanded ? "rotate-180" : ""}`}
              aria-hidden="true"
            />
          </button>
        </span>
        <div className="flex items-center gap-0.5 opacity-60 transition-opacity duration-150 focus-within:opacity-100 group-hover:opacity-100">
          <IconButton
            onClick={handleCopy}
            title={t("settings.history.copyConversation")}
          >
            {showCopied ? (
              <Check width={14} height={14} />
            ) : (
              <Copy width={14} height={14} />
            )}
          </IconButton>
          <IconButton
            onClick={handleDelete}
            title={t("settings.history.delete")}
          >
            <Trash2 width={14} height={14} />
          </IconButton>
        </div>
      </div>

      {expanded && (
        <div className="flex flex-col gap-2 pt-1.5">
          {session.messages.map((message, index) => {
            const { text, screenshot, files } = cleanMessageContent(
              message.content,
            );
            const isUser = message.role === "user";
            const thumbnails = message.images ?? [];
            return (
              <div
                key={index}
                className={`group/msg flex items-center gap-1.5 ${isUser ? "justify-end" : "justify-start"}`}
              >
                {/* Branch from here. Placed on the message rather than the
                    conversation because the point is to pick the moment to diverge
                    from. Reveals on hover so a long thread stays readable.
                    Non-destructive: this copies the thread up to this message into a
                    new conversation and leaves this one exactly as it is. */}
                {isUser && (
                  <button
                    onClick={() => onBranch(index)}
                    title={t("settings.history.branchConversation")}
                    aria-label={t("settings.history.branchConversation")}
                    className="shrink-0 rounded-md p-1 text-muted-soft opacity-0 transition-opacity duration-150 hover:bg-surface-strong hover:text-ink focus-visible:opacity-100 group-hover/msg:opacity-100"
                  >
                    <GitBranch width={13} height={13} />
                  </button>
                )}
                <div
                  className={
                    isUser
                      ? "max-w-[85%] rounded-xl rounded-br-sm bg-accent px-3 py-2 text-[13px] leading-relaxed text-on-primary select-text whitespace-pre-wrap break-words"
                      : "max-w-[85%] rounded-xl rounded-bl-sm bg-surface-strong px-3 py-2 text-[13px] leading-relaxed text-ink select-text break-words"
                  }
                >
                  {isUser ? (
                    text
                  ) : (
                    <ReactMarkdown components={assistantMarkdown}>
                      {text}
                    </ReactMarkdown>
                  )}
                  {thumbnails.length > 0 ? (
                    <HistoryThumbnails
                      urls={thumbnails}
                      hasScreen={screenshot}
                      isUser={isUser}
                      screenLabel={t("settings.history.screenshotAttached")}
                    />
                  ) : (
                    screenshot && (
                      <span
                        className={`mt-1.5 inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium ${
                          isUser
                            ? "bg-on-primary/20 text-on-primary/90"
                            : "bg-mid-gray/15 text-muted"
                        }`}
                      >
                        <Camera width={10} height={10} />
                        {t("settings.history.screenshotAttached")}
                      </span>
                    )
                  )}
                  {files.map((name) => (
                    <span
                      key={name}
                      className={`mt-1.5 me-1 inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium ${
                        isUser
                          ? "bg-on-primary/20 text-on-primary/90"
                          : "bg-mid-gray/15 text-muted"
                      }`}
                    >
                      <FileText width={10} height={10} />
                      {name}
                    </span>
                  ))}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
