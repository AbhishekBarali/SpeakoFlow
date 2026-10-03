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
  Camera,
  Check,
  Copy,
  Eye,
  EyeOff,
  FileText,
  FolderOpen,
  GitBranch,
  HardDrive,
  MoreHorizontal,
  Pause,
  Play,
  RotateCcw,
  Star,
  TextSelect,
  Trash2,
  Users,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import ReactMarkdown, { type Components } from "react-markdown";
import { toast } from "sonner";
import {
  commands,
  events,
  type AssistantHistorySummary,
  type ChatMessage,
  type HistoryEntry,
  type HistoryUpdatePayload,
} from "@/bindings";
import { useOsType } from "@/hooks/useOsType";
import { useSettings } from "@/hooks/useSettings";
import { AudioPlayer } from "../../ui/AudioPlayer";
import { Button } from "../../ui/Button";
import { MenuButton, type MenuItem } from "../../ui/Menu";
import { PageHeader } from "../../ui/Page";
import { Tabs } from "../../ui/Tabs";
import { useNavigation } from "../../shell/navigation";
import { formatTimeOfDay, groupByDay } from "@/utils/dayGroups";
import {
  buildFeed,
  cleanMessageContent,
  isFlowHistoryEntry,
  matchesFilter,
  rowKind,
  visibleFilters,
  type HistoryFilter,
  type RowKind,
} from "./historyFeed";

/**
 * Markdown styling for assistant replies — mirrors the assistant panel so
 * bold, lists, code, etc. render properly instead of leaking raw markdown.
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

/** Shared look of a row's small icon actions (and the ⋯ menu trigger). The
 *  colour is separate so a button can swap it without two text colours
 *  fighting over the same element. */
const ICON_BUTTON_SHAPE =
  "grid h-7 w-7 cursor-pointer place-items-center rounded-md transition-colors hover:bg-ink/6 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:text-muted-soft/50 disabled:hover:bg-transparent aria-expanded:bg-ink/6 aria-expanded:text-ink";
const ICON_BUTTON_TONE = "text-muted hover:text-ink";
const ICON_BUTTON = `${ICON_BUTTON_SHAPE} ${ICON_BUTTON_TONE}`;

/** Hover/focus reveal for a row's actions. An open menu keeps them shown. */
const REVEAL_ON_HOVER =
  "flex items-center gap-0.5 opacity-0 transition-opacity duration-150 group-hover:opacity-100 focus-within:opacity-100 has-[[aria-expanded=true]]:opacity-100";

/** A quiet inline link inside a row ("Try again", "Recover"). */
const INLINE_LINK =
  "cursor-pointer rounded font-medium text-accent hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50";

const IconButton: React.FC<{
  onClick: () => void;
  title: string;
  disabled?: boolean;
  /** Replaces the default muted colour. */
  tone?: string;
  children: React.ReactNode;
}> = ({ onClick, title, disabled, tone = ICON_BUTTON_TONE, children }) => (
  <button
    type="button"
    onClick={onClick}
    disabled={disabled}
    aria-label={title}
    className={`${ICON_BUTTON_SHAPE} ${tone}`}
    title={title}
  >
    {children}
  </button>
);

/** The kind of a row, named only in the mixed "All" list. A dictation is the
 *  default and goes unnamed: labelling every one of them is what made the list
 *  read as a column of "Dictation". */
const KIND_LABEL_KEYS: Partial<Record<RowKind, string>> = {
  flow: "settings.history.flowLabel",
  ask: "settings.history.kinds.ask",
  call: "settings.history.kinds.call",
};

const FILTER_LABEL_KEYS: Record<HistoryFilter, string> = {
  all: "settings.history.filters.all",
  recordings: "settings.history.filters.recordings",
  flow: "settings.history.filters.flow",
  asks: "settings.history.filters.asks",
  calls: "settings.history.filters.calls",
};

const EMPTY_KEYS: Record<HistoryFilter, string> = {
  all: "settings.history.empty",
  recordings: "settings.history.emptyRecordings",
  flow: "settings.history.emptyFlow",
  asks: "settings.history.emptyAsks",
  calls: "settings.history.emptyCalls",
};

/**
 * Every row's frame: the time in a column of its own on the left, the content
 * beside it, and a few quiet actions on the right. The time column is what
 * lets a row be just its text — there is no caption line under each entry to
 * repeat the time, the kind and an icon.
 */
const RowFrame: React.FC<{
  time: string;
  /** Named in the mixed list only; see `KIND_LABEL_KEYS`. */
  kindLabel?: string;
  actions: React.ReactNode;
  children: React.ReactNode;
}> = ({ time, kindLabel, actions, children }) => (
  <div className="group flex items-baseline gap-4 px-4 py-3 transition-colors hover:bg-surface-muted/70">
    <div className="w-[4.75rem] shrink-0 text-xs text-muted">
      <span className="block whitespace-nowrap tabular-nums">{time}</span>
      {kindLabel && (
        <span className="mt-0.5 block truncate text-[11px] text-muted-soft">
          {kindLabel}
        </span>
      )}
    </div>
    <div className="min-w-0 flex-1">{children}</div>
    <div className="-my-1 flex shrink-0 items-center gap-0.5 self-start">
      {actions}
    </div>
  </div>
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

export const HistorySettings: React.FC = () => {
  const { t, i18n } = useTranslation();
  const { openSettings, navigate } = useNavigation();
  const { getSetting } = useSettings();
  const assistantEnabled = getSetting("assistant_enabled") ?? true;
  const flowEnabled = getSetting("flow_enabled") ?? false;
  const osType = useOsType();
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [selectedFilter, setFilter] = useState<HistoryFilter>("all");
  const [hasMore, setHasMore] = useState(true);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const entriesRef = useRef<HistoryEntry[]>([]);
  const loadingRef = useRef(false);
  const pageGenerationRef = useRef(0);

  const filters = useMemo(
    () => visibleFilters({ flowEnabled, assistantEnabled }),
    [flowEnabled, assistantEnabled],
  );
  // A tab whose feature was switched off while it was selected falls back to
  // All rather than leaving the page on a filter with no tab.
  const filter = filters.includes(selectedFilter) ? selectedFilter : "all";

  // Assistant conversations are stored separately from transcriptions, so we
  // load them as their own list and merge for display. The list carries only
  // what a row shows (no messages), and the backend caps it, so one fetch of
  // the whole list stays small. It is not paged like recordings: the feed
  // orders conversations by last activity, while the id cursor pages them by
  // creation, so an old conversation continued today would sit on a page not
  // yet loaded. A row loads its messages when expanded.
  const [assistantSessions, setAssistantSessions] = useState<
    AssistantHistorySummary[]
  >([]);
  const [assistantLoaded, setAssistantLoaded] = useState(false);
  const assistantGenerationRef = useRef(0);
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
    // A first-page reload (retention sweep, which runs after every save under a
    // keep-N policy) supersedes any page still in flight. Without the
    // generation check, that older page was appended after the fresh first
    // page and the list showed duplicates.
    if (isFirstPage) pageGenerationRef.current += 1;
    const generation = pageGenerationRef.current;
    loadingRef.current = true;

    if (isFirstPage) setLoading(true);

    try {
      const result = await commands.getHistoryEntries(
        cursor ?? null,
        PAGE_SIZE,
      );
      if (generation !== pageGenerationRef.current) return;
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
      if (generation === pageGenerationRef.current) {
        setLoading(false);
        loadingRef.current = false;
      }
    }
  }, []);

  const loadAssistantSessions = useCallback(async () => {
    // A reload that resolves after a newer one must not put its older list back.
    assistantGenerationRef.current += 1;
    const generation = assistantGenerationRef.current;
    try {
      const result = await commands.getAssistantHistoryEntries(null, null);
      if (generation !== assistantGenerationRef.current) return;
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
    // Coalesced: one turn emits this more than once.
    let timer: ReturnType<typeof setTimeout> | null = null;
    const unlisten = listen("assistant-history-updated", () => {
      if (timer !== null) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        loadAssistantSessions();
      }, 400);
    });
    return () => {
      if (timer !== null) clearTimeout(timer);
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

  const recoverHistoryEntry = async (id: number) => {
    const result = await commands.recoverHistoryEntry(id);
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
    (messages: ChatMessage[]) => {
      const text = messages
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

  /** Load a past conversation into a call and open it. */
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

  const feed = useMemo(
    () => buildFeed(entries, assistantSessions),
    [entries, assistantSessions],
  );

  const filteredFeed = useMemo(
    () => feed.filter((item) => matchesFilter(item, filter)),
    [feed, filter],
  );

  const dayGroups = useMemo(
    () =>
      groupByDay(filteredFeed, (item) => item.sortTime, i18n.language, {
        today: t("historyPage.today"),
        yesterday: t("historyPage.yesterday"),
      }),
    [filteredFeed, i18n.language, t],
  );

  /** Only the mixed list needs to say what each row is. */
  const kindLabelFor = (kind: RowKind): string | undefined => {
    if (filter !== "all") return undefined;
    const key = KIND_LABEL_KEYS[kind];
    return key ? t(key) : undefined;
  };

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
    content = (
      <div className="rounded-xl border border-dashed border-hairline-strong px-6 py-12 text-center">
        <p className="text-sm text-muted">{t(EMPTY_KEYS[filter])}</p>
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
                      kindLabel={kindLabelFor(rowKind(item))}
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
                      recoverDismissed={recoverHistoryEntry}
                    />
                  ) : (
                    <AssistantHistoryEntryComponent
                      key={`a-${item.session.id}`}
                      session={item.session}
                      kindLabel={kindLabelFor(rowKind(item))}
                      expanded={expandedAssistant.has(item.session.id)}
                      onToggleExpand={() =>
                        toggleExpandAssistant(item.session.id)
                      }
                      onCopyText={copyToClipboard}
                      onCopyConversation={copyConversation}
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
        {/* Pagination belongs to recordings; conversations are loaded in one page. */}
        {filter !== "asks" && filter !== "calls" && (
          <div ref={sentinelRef} className="h-1" />
        )}
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
            <Button
              onClick={openRecordingsFolder}
              variant="secondary"
              size="sm"
              className="flex items-center gap-2"
              title={t("settings.history.openFolder")}
            >
              <FolderOpen className="h-4 w-4" />
              <span>{t("settings.history.openFolder")}</span>
            </Button>
          </>
        }
      />
      <Tabs
        label={t("settings.history.filters.label")}
        value={filter}
        onChange={setFilter}
        items={filters.map((value) => ({
          id: value,
          label: t(FILTER_LABEL_KEYS[value]),
        }))}
      />
      <div className="mt-6">{content}</div>
    </div>
  );
};

interface HistoryEntryProps {
  entry: HistoryEntry;
  kindLabel?: string;
  onToggleSaved: () => void;
  onCopyText: () => void;
  getAudioUrl: (fileName: string) => Promise<string | null>;
  deleteAudio: (id: number) => Promise<void>;
  retryTranscription: (id: number) => Promise<void>;
  /** Bring back a dismissed dictation (Esc, or the tray's Cancel). */
  recoverDismissed: (id: number) => Promise<void>;
}

/**
 * One dictation, which is its text and nothing else. What the recogniser heard
 * before cleanup or Flow rewrote it, the recording, and the rest live in the ⋯
 * menu: they are occasional, and a link, a wand and a label under every row
 * read as noise in a list that is mostly dictations.
 */
const HistoryEntryComponent: React.FC<HistoryEntryProps> = ({
  entry,
  kindLabel,
  onToggleSaved,
  onCopyText,
  getAudioUrl,
  deleteAudio,
  retryTranscription,
  recoverDismissed,
}) => {
  const { t, i18n } = useTranslation();
  const [showCopied, setShowCopied] = useState(false);
  const [retrying, setRetrying] = useState(false);
  const [showOriginal, setShowOriginal] = useState(false);
  const [audioSrc, setAudioSrc] = useState<string | null>(null);
  const [loadingAudio, setLoadingAudio] = useState(false);

  /** Cancelled by the user. The row keeps its audio (and any transcript that
   * was finished) so it can be brought back, but it does not show text the
   * user threw away until they ask for it. */
  const dismissed = entry.dismissed;
  const hasTranscription = entry.transcription_text.trim().length > 0;
  const flowEntry = isFlowHistoryEntry(entry);
  const processedText = entry.post_processed_text?.trim()
    ? entry.post_processed_text
    : null;
  const hasDistinctProcessedText =
    processedText !== null &&
    processedText.trim() !== entry.transcription_text.trim();
  // What was pasted, and what it was made from (when those differ).
  const finalText = flowEntry
    ? processedText
    : hasDistinctProcessedText
      ? processedText
      : hasTranscription
        ? entry.transcription_text
        : null;
  const originalText =
    !dismissed && (flowEntry || hasDistinctProcessedText) && hasTranscription
      ? entry.transcription_text
      : null;
  const hasCopyableText =
    !dismissed && (finalText !== null || hasTranscription);

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

  const handleRecover = async () => {
    try {
      setRetrying(true);
      await recoverDismissed(entry.id);
    } catch (error) {
      console.error("Failed to recover a dismissed transcription:", error);
      toast.error(t("historyPage.recoverError"));
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

  const failed = !retrying && !dismissed && finalText === null;
  const copyTitle = t(
    flowEntry && processedText
      ? "settings.history.copyFlowOutput"
      : hasDistinctProcessedText
        ? "settings.history.copyFinalText"
        : "settings.history.copyToClipboard",
  );

  const menuItems: MenuItem[] = [
    ...(originalText
      ? [
          {
            id: "original",
            label: showOriginal
              ? t("historyPage.hideOriginal")
              : flowEntry
                ? t("historyPage.showSaid")
                : t("historyPage.showOriginal"),
            icon: showOriginal ? EyeOff : Eye,
            onSelect: () => setShowOriginal((value) => !value),
          },
        ]
      : []),
    {
      id: "play",
      label: audioSrc
        ? t("historyPage.hideRecording")
        : t("historyPage.playRecording"),
      icon: audioSrc ? Pause : Play,
      disabled: loadingAudio || retrying,
      onSelect: () => void toggleAudio(),
    },
    {
      id: "save",
      label: entry.saved
        ? t("settings.history.unsave")
        : t("settings.history.save"),
      icon: Star,
      disabled: retrying,
      onSelect: onToggleSaved,
    },
    {
      id: "retry",
      label: dismissed
        ? t("historyPage.recover")
        : t("settings.history.retranscribe"),
      icon: RotateCcw,
      disabled: retrying,
      onSelect: () => void (dismissed ? handleRecover() : handleRetranscribe()),
    },
    {
      id: "delete",
      label: t("settings.history.delete"),
      icon: Trash2,
      tone: "danger",
      separated: true,
      disabled: retrying,
      onSelect: () => void handleDeleteEntry(),
    },
  ];

  let body: React.ReactNode;
  if (retrying) {
    body = (
      <p
        className="text-sm leading-relaxed"
        style={{ animation: "transcribe-pulse 3s ease-in-out infinite" }}
      >
        <style>{`
          @keyframes transcribe-pulse {
            0%, 100% { color: color-mix(in srgb, var(--color-text) 40%, transparent); }
            50% { color: color-mix(in srgb, var(--color-text) 90%, transparent); }
          }
        `}</style>
        {t("settings.history.transcribing")}
      </p>
    );
  } else if (dismissed) {
    // Said plainly and in the same quiet grey as a failed row: a dismissal is
    // the user's own choice, not an error to flag.
    body = (
      <p className="text-sm leading-relaxed text-muted">
        {t("historyPage.dismissed")}{" "}
        <button
          type="button"
          onClick={() => void handleRecover()}
          title={t("historyPage.recoverTitle")}
          className={INLINE_LINK}
        >
          {t("historyPage.recover")}
        </button>
      </p>
    );
  } else if (finalText !== null) {
    body = (
      <p className="max-w-[75ch] cursor-text select-text whitespace-pre-wrap break-words text-[0.9375rem] leading-relaxed text-ink">
        {finalText}
      </p>
    );
  } else if (flowEntry && hasTranscription) {
    body = (
      <p className="text-sm leading-relaxed text-muted">
        {t("settings.history.flowNoOutput")}
      </p>
    );
  } else {
    body = (
      <p className="text-sm leading-relaxed text-muted">
        {t("historyPage.failed")}
        {failed && (
          <>
            {" "}
            <button
              type="button"
              onClick={() => void handleRetranscribe()}
              className={INLINE_LINK}
            >
              {t("historyPage.retry")}
            </button>
          </>
        )}
      </p>
    );
  }

  return (
    <RowFrame
      time={formatTimeOfDay(entry.timestamp, i18n.language)}
      kindLabel={kindLabel}
      actions={
        <>
          {/* A saved entry keeps its star in view. */}
          {entry.saved && (
            <IconButton
              onClick={onToggleSaved}
              disabled={retrying}
              title={t("settings.history.unsave")}
              tone="text-amber-500 hover:text-amber-600"
            >
              <Star width={14} height={14} fill="currentColor" />
            </IconButton>
          )}
          <div className={REVEAL_ON_HOVER}>
            <IconButton
              onClick={handleCopyText}
              disabled={!hasCopyableText || retrying}
              title={copyTitle}
            >
              {showCopied ? (
                <Check width={14} height={14} className="text-success" />
              ) : (
                <Copy width={14} height={14} />
              )}
            </IconButton>
            <MenuButton
              items={menuItems}
              width={220}
              ariaLabel={t("historyPage.more")}
              title={t("historyPage.more")}
              className={ICON_BUTTON}
            >
              {retrying ? (
                <RotateCcw
                  width={14}
                  height={14}
                  style={{ animation: "spin 1s linear infinite reverse" }}
                />
              ) : (
                <MoreHorizontal width={15} height={15} />
              )}
            </MenuButton>
          </div>
        </>
      }
    >
      {body}

      {showOriginal && originalText && (
        <div className="mt-2.5 max-w-[75ch] rounded-lg border border-hairline bg-canvas px-3 py-2.5">
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

      {audioSrc && (
        <AudioPlayer src={audioSrc} autoPlay className="mt-2.5 w-full" />
      )}
    </RowFrame>
  );
};

interface AssistantHistoryEntryProps {
  session: AssistantHistorySummary;
  kindLabel?: string;
  expanded: boolean;
  onToggleExpand: () => void;
  onCopyText: (text: string) => Promise<void>;
  onCopyConversation: (messages: ChatMessage[]) => void;
  onDelete: () => Promise<void>;
  onResume: () => void;
  /** Continue from one message as a new conversation, leaving this one intact. */
  onBranch: (messageIndex: number) => void;
}

/** One conversation's messages; `null` when it was deleted in the meantime. */
const fetchConversationMessages = async (
  id: number,
): Promise<ChatMessage[] | null> => {
  const result = await commands.getAssistantHistoryEntry(id);
  if (result.status !== "ok") throw new Error(String(result.error));
  return result.data?.messages ?? null;
};

/** The answer a quick ask got: its last reply, as stored. */
const lastAnswer = (messages: ChatMessage[]): string | null => {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === "assistant") {
      return cleanMessageContent(messages[i].content).text || null;
    }
  }
  return null;
};

/**
 * One quick ask or one call.
 *
 * A quick ask is a question and its answer, so the row is the question and the
 * start of the answer; opening it shows the whole answer. A call is a
 * conversation, so the row is how it began and how long it ran; opening it
 * shows the thread. Neither repeats what it is on every row — the tab already
 * says, and the "All" list names it in the time column.
 */
const AssistantHistoryEntryComponent: React.FC<AssistantHistoryEntryProps> = ({
  session,
  kindLabel,
  expanded,
  onToggleExpand,
  onCopyText,
  onCopyConversation,
  onDelete,
  onResume,
  onBranch,
}) => {
  const { t, i18n } = useTranslation();
  const [showCopied, setShowCopied] = useState(false);
  // Only an expanded row holds its messages. They are fetched again when the
  // conversation gains a turn while open, and let go on collapse, so the page
  // never accumulates every thread it has shown.
  const [messages, setMessages] = useState<ChatMessage[] | null>(null);
  const isCall = session.kind === "call";

  useEffect(() => {
    if (!expanded) {
      setMessages(null);
      return;
    }
    let cancelled = false;
    fetchConversationMessages(session.id)
      .then((loaded) => {
        if (!cancelled) setMessages(loaded ?? []);
      })
      .catch((error) => {
        console.error("Failed to load assistant conversation:", error);
        if (!cancelled) setMessages([]);
      });
    return () => {
      cancelled = true;
    };
  }, [expanded, session.id, session.updated_at, session.message_count]);

  const flashCopied = () => {
    setShowCopied(true);
    setTimeout(() => setShowCopied(false), 2000);
  };

  const handleCopy = async () => {
    try {
      const loaded = messages ?? (await fetchConversationMessages(session.id));
      if (!loaded) return;
      if (isCall) {
        onCopyConversation(loaded);
      } else {
        const answer = lastAnswer(loaded);
        if (!answer) return;
        await onCopyText(answer);
      }
      flashCopied();
    } catch (error) {
      console.error("Failed to copy assistant conversation:", error);
    }
  };

  const handleDelete = async () => {
    try {
      await onDelete();
    } catch (error) {
      console.error("Failed to delete assistant conversation:", error);
      toast.error(t("settings.history.deleteAssistantError"));
    }
  };

  const canCopy = isCall || session.preview !== null;
  const copyLabel = isCall
    ? t("settings.history.copyConversation")
    : t("settings.history.copyAnswer");

  const menuItems: MenuItem[] = [
    {
      id: "continue",
      label: t("historyPage.chat.continueInCall"),
      icon: ArrowUpRight,
      onSelect: onResume,
    },
    {
      id: "copy",
      label: copyLabel,
      icon: Copy,
      disabled: !canCopy,
      onSelect: () => void handleCopy(),
    },
    {
      id: "delete",
      label: t("settings.history.delete"),
      icon: Trash2,
      tone: "danger",
      separated: true,
      onSelect: () => void handleDelete(),
    },
  ];

  // A quick ask that grew past one exchange (possible before calls owned
  // follow-ups) reads as a thread, like a call.
  const showAsThread = isCall || (messages !== null && messages.length > 2);

  return (
    <RowFrame
      time={formatTimeOfDay(session.updated_at, i18n.language)}
      kindLabel={kindLabel}
      actions={
        <div className={REVEAL_ON_HOVER}>
          {isCall ? (
            <IconButton
              onClick={onResume}
              title={t("historyPage.chat.continueInCall")}
            >
              <ArrowUpRight width={15} height={15} />
            </IconButton>
          ) : (
            <IconButton
              onClick={() => void handleCopy()}
              disabled={!canCopy}
              title={copyLabel}
            >
              {showCopied ? (
                <Check width={14} height={14} className="text-success" />
              ) : (
                <Copy width={14} height={14} />
              )}
            </IconButton>
          )}
          <MenuButton
            items={menuItems}
            width={240}
            ariaLabel={t("historyPage.more")}
            title={t("historyPage.more")}
            className={ICON_BUTTON}
          >
            {isCall && showCopied ? (
              <Check width={14} height={14} className="text-success" />
            ) : (
              <MoreHorizontal width={15} height={15} />
            )}
          </MenuButton>
        </div>
      }
    >
      {/* The whole summary opens the row. Picking a conversation back up is in
          the actions; reading it again is what a click on it is for. */}
      <button
        type="button"
        onClick={onToggleExpand}
        aria-expanded={expanded}
        className="block w-full max-w-[75ch] cursor-pointer rounded-md text-start focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
      >
        {/* Collapsed, a row is one line of question and one of answer: enough
            to recognise it, and no taller than a two-line dictation. Two lines
            of each turned the Assistant list into a wall of text. */}
        <span
          className={`block break-words text-sm leading-relaxed text-ink ${
            expanded ? "" : "line-clamp-1"
          }`}
        >
          {session.title.trim() || t("assistant.conversation.history.untitled")}
        </span>
        {!isCall && !expanded && session.preview && (
          <span className="line-clamp-1 block break-words text-[0.8125rem] leading-relaxed text-muted">
            {session.preview}
          </span>
        )}
        {isCall && (
          <span className="mt-0.5 flex flex-wrap items-center gap-x-2 text-xs text-muted">
            <span>
              {t("settings.history.messageCount", {
                count: session.message_count,
              })}
            </span>
            {session.meeting_id !== null && (
              <span className="inline-flex min-w-0 items-center gap-1">
                <Users width={11} height={11} aria-hidden="true" />
                <span className="truncate">
                  {session.meeting_title?.trim() ||
                    t("assistant.conversation.meeting.untitled")}
                </span>
              </span>
            )}
          </span>
        )}
      </button>

      {expanded && messages === null && (
        <div
          className="mt-2 h-9 animate-pulse rounded-xl bg-surface-strong/60"
          aria-busy="true"
        />
      )}

      {expanded &&
        messages !== null &&
        (showAsThread ? (
          <ConversationThread messages={messages} onBranch={onBranch} />
        ) : (
          <AskAnswer messages={messages} title={session.title} />
        ))}
    </RowFrame>
  );
};

/** Chips for what rode along with a user message: a selection, files. */
const AttachmentChips: React.FC<{
  selectionChars: number;
  files: string[];
  onAccent: boolean;
}> = ({ selectionChars, files, onAccent }) => {
  const { t } = useTranslation();
  const chip = `mt-1.5 me-1 inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium ${
    onAccent
      ? "bg-on-primary/20 text-on-primary/90"
      : "bg-mid-gray/15 text-muted"
  }`;
  return (
    <>
      {selectionChars > 0 && (
        <span className={chip}>
          <TextSelect width={10} height={10} />
          {t("assistant.selectionAttached", { count: selectionChars })}
        </span>
      )}
      {files.map((name) => (
        <span key={name} className={chip}>
          <FileText width={10} height={10} />
          {name}
        </span>
      ))}
    </>
  );
};

/**
 * An opened quick ask: what it was asked about, then the answer. The question
 * is already the row's title, so it is repeated only when the title had to be
 * cut short.
 */
const AskAnswer: React.FC<{ messages: ChatMessage[]; title: string }> = ({
  messages,
  title,
}) => {
  const { t } = useTranslation();
  const question = messages.find((m) => m.role === "user");
  const asked = question ? cleanMessageContent(question.content) : null;
  const answer = lastAnswer(messages);
  const thumbnails = question?.images ?? [];
  const showQuestion =
    asked !== null && asked.text !== "" && title.trimEnd().endsWith("…");

  return (
    <div className="mt-2 max-w-[75ch]">
      {showQuestion && (
        <p className="mb-2 select-text whitespace-pre-wrap break-words text-sm leading-relaxed text-ink">
          {asked.text}
        </p>
      )}
      {asked && (asked.selectionChars > 0 || asked.files.length > 0) && (
        <div className="-mt-1 mb-1">
          <AttachmentChips
            selectionChars={asked.selectionChars}
            files={asked.files}
            onAccent={false}
          />
        </div>
      )}
      {thumbnails.length > 0 && (
        <div className="mb-2">
          <HistoryThumbnails
            urls={thumbnails}
            hasScreen={asked?.screenshot}
            isUser={false}
            screenLabel={t("settings.history.screenshotAttached")}
          />
        </div>
      )}
      {answer ? (
        <div className="select-text break-words text-[0.8125rem] leading-relaxed text-body">
          <ReactMarkdown components={assistantMarkdown}>{answer}</ReactMarkdown>
        </div>
      ) : (
        <p className="text-[0.8125rem] text-muted">
          {t("settings.history.noAnswer")}
        </p>
      )}
    </div>
  );
};

/** An opened call: the turn-by-turn transcript, with a branch point on each
 *  of the user's messages. */
const ConversationThread: React.FC<{
  messages: ChatMessage[];
  onBranch: (messageIndex: number) => void;
}> = ({ messages, onBranch }) => {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-2 pt-2.5">
      {messages.map((message, index) => {
        const { text, screenshot, files, selectionChars } = cleanMessageContent(
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
                type="button"
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
              <AttachmentChips
                selectionChars={selectionChars}
                files={files}
                onAccent={isUser}
              />
            </div>
          </div>
        );
      })}
    </div>
  );
};
