import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowUp,
  AudioLines,
  ChevronDown,
  History,
  Loader2,
  Maximize2,
  MessageSquarePlus,
  Mic,
  MicOff,
  RotateCcw,
  Search,
  SlidersHorizontal,
  Square,
  SquarePen,
  Users,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import {
  commands,
  type AssistantHistoryFilter,
  type AssistantHistorySummary,
  type MeetingAttachment,
} from "@/bindings";
import {
  CONVERSATION_SENSITIVITIES,
  type ConversationSensitivity,
} from "./conversationPolicy";
import type { useVoiceConversation } from "./useVoiceConversation";
import type { CallForm } from "./useCallForm";
import { VoiceOrb } from "./VoiceOrb";

/*
 * The live call, as a floating bar.
 *
 * At rest the call is four controls and nothing else — type, microphone, the
 * orb that hangs up, speaker — drawn small, so a call parked on screen stays
 * out of the way. Hovering it grows it to full size. Above it a bubble says what
 * the assistant is doing — listening, thinking, speaking, or what went wrong —
 * but never the words of the reply, which the voice is already delivering;
 * clicking the bubble opens the conversation (transcript, past conversations,
 * options, New chat).
 *
 * Everything the user can misclick on the bar is gone from it: New chat lives
 * in the expanded view, and there is no separate keyboard or expand button —
 * the pen *is* the keyboard, and the bubble is the way in.
 *
 * Expanded, the same bar sits under the conversation, so the control you just
 * clicked never moves: Rust grows the window upward around the bar's
 * bottom-centre (`assistant::set_conversation_expanded`).
 *
 * Everything visible carries `data-hit-surface`, so the transparent rest of the
 * window passes clicks through to the desktop (`hitRegion.ts`).
 */

type Voice = ReturnType<typeof useVoiceConversation>;

/** Saved conversations fetched per page in the history view. */
const HISTORY_PAGE = 30;

const DEAD_PHASES = ["off", "starting", "error"];

/**
 * How long the text box waits before handing the bar back to voice.
 *
 * Typing is a detour in a call, not a mode you live in, so the bar goes back to
 * voice on its own: shortly after a message is sent, after a quiet spell with
 * nothing typed, and — much later — even with a half-written message, which is
 * kept and comes back the next time the pen is tapped. Speaking while the box is
 * empty also goes straight back.
 */
export const TYPING_RETURN_MS = {
  afterSend: 1_500,
  idleEmpty: 8_000,
  idleDraft: 30_000,
} as const;

/** The delay before an idle text box returns to voice. Pure, for tests. */
export function typingReturnDelay(draft: string, justSent: boolean): number {
  if (draft.trim()) return TYPING_RETURN_MS.idleDraft;
  return justSent ? TYPING_RETURN_MS.afterSend : TYPING_RETURN_MS.idleEmpty;
}

/**
 * The bar changing between voice and typing, as one motion.
 *
 * The two modes hold different controls, so React swaps the bar's children in
 * a single frame — which is what made the switch feel like a jump cut. This is
 * a FLIP on top of that swap: the bar's width glides from the old width to the
 * new one, the orb (the one control both modes share) glides from where it was
 * to where it now lives, and the new controls fade in behind it.
 */
const MORPH_MS = 260;
const MORPH_EASE = "cubic-bezier(0.22, 0.61, 0.36, 1)";

function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

function centreX(element: Element): number {
  const rect = element.getBoundingClientRect();
  return rect.left + rect.width / 2;
}

/** "5 minutes ago" / "yesterday" / "12 Mar", in the UI language. */
function relativeTime(seconds: number, locale: string): string {
  const diff = seconds * 1000 - Date.now();
  const abs = Math.abs(diff);
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;
  try {
    const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
    if (abs < minute) return rtf.format(0, "second");
    if (abs < hour) return rtf.format(Math.round(diff / minute), "minute");
    if (abs < day) return rtf.format(Math.round(diff / hour), "hour");
    if (abs < 7 * day) return rtf.format(Math.round(diff / day), "day");
    return new Intl.DateTimeFormat(locale, {
      month: "short",
      day: "numeric",
    }).format(new Date(seconds * 1000));
  } catch {
    return new Date(seconds * 1000).toLocaleDateString();
  }
}

function IconButton({
  label,
  onClick,
  pressed,
  disabled,
  tone,
  children,
}: {
  label: string;
  onClick: () => void;
  pressed?: boolean;
  disabled?: boolean;
  tone?: "danger";
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className={`call-icon${tone ? ` ${tone}` : ""}`}
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/** The orb is the call. Hovering it shows how to end it; clicking ends it. */
function CallOrb({ voice, onEnd }: { voice: Voice; onEnd: () => void }) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      className="call-orb"
      onClick={onEnd}
      aria-label={t("assistant.conversation.end")}
      title={t("assistant.conversation.end")}
    >
      <VoiceOrb phase={voice.phase} level={voice.level} />
      <span className="call-orb-end" aria-hidden="true">
        <X size={15} strokeWidth={2.6} />
      </span>
    </button>
  );
}

export function CallBar({
  voice,
  name,
  typing,
  onTypingChange,
  onEnd,
}: {
  voice: Voice;
  name: string;
  typing: boolean;
  onTypingChange: (typing: boolean) => void;
  onEnd: () => void;
}) {
  const { t } = useTranslation();
  // Survives leaving the text box, so a half-written message is still there
  // the next time the pen is tapped.
  const [draft, setDraft] = useState("");
  const [justSent, setJustSent] = useState(false);
  // Bumped by anything that shows the user is still in the text box, so the
  // return-to-voice timer starts over.
  const [activity, setActivity] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const bar = useRef<HTMLDivElement>(null);
  // Where the bar and the orb were just before the mode changed (see
  // `MORPH_MS`). Set by `change`, consumed by the layout effect below.
  const before = useRef<{
    width: number;
    height: number;
    orbX: number;
    orbSize: number;
  } | null>(null);
  const live = !DEAD_PHASES.includes(voice.phase);
  const replying = voice.phase === "responding" || voice.phase === "speaking";
  const { setComposing, phase } = voice;

  // Every way in or out of the text box comes through here, so every one of
  // them — the pen, the voice button, Escape, the timer, speaking — gets the
  // same motion rather than a jump cut.
  const change = useCallback(
    (next: boolean) => {
      const node = bar.current;
      const orb = node?.querySelector(".call-orb");
      if (node && orb && typeof node.animate === "function") {
        const rect = node.getBoundingClientRect();
        before.current = {
          width: rect.width,
          height: rect.height,
          orbX: centreX(orb),
          orbSize: orb.getBoundingClientRect().width,
        };
      }
      onTypingChange(next);
    },
    [onTypingChange],
  );

  useLayoutEffect(() => {
    const from = before.current;
    before.current = null;
    const node = bar.current;
    if (!from || !node || prefersReducedMotion()) return;
    const orb = node.querySelector<HTMLElement>(".call-orb");
    // Settle every size the new mode depends on at once. Left to their own
    // CSS transitions, the icons would still be shrinking after the bar had
    // finished, and the bar would land a few pixels off and then creep.
    node.classList.add("morphing");
    const to = node.getBoundingClientRect();
    // The orb is measured with the bar held at its *old* width, because that
    // is where the orb sits in the new layout on the first frame of the
    // animation. The offset is then the gap between that and where it was.
    node.style.width = `${from.width}px`;
    const orbStart = orb ? centreX(orb) : 0;
    const orbSize = orb?.getBoundingClientRect().width || 1;
    node.style.width = "";
    const timing = { duration: MORPH_MS, easing: MORPH_EASE };
    const morph = node.animate(
      [
        { width: `${from.width}px`, height: `${from.height}px` },
        { width: `${to.width}px`, height: `${to.height}px` },
      ],
      timing,
    );
    if (orb) {
      const dx = from.orbX - orbStart;
      const scale = from.orbSize / orbSize;
      orb.animate(
        [
          { transform: `translateX(${dx}px) scale(${scale})` },
          { transform: "none" },
        ],
        timing,
      );
    }
    // Everything but the orb is new: let it arrive behind the motion.
    for (const child of Array.from(node.children)) {
      if (child === orb) continue;
      child.animate([{ opacity: 0 }, { opacity: 1 }], {
        duration: 170,
        delay: 80,
        easing: "ease-out",
        fill: "backwards",
      });
    }
    const settle = () => node.classList.remove("morphing");
    morph.onfinish = settle;
    morph.oncancel = settle;
  }, [typing]);

  useEffect(() => {
    if (!typing) return;
    setJustSent(false);
    input.current?.focus();
  }, [typing]);

  // Keyboard clatter reaches the microphone as speech. While a message is being
  // written, it is the user's turn already — see `setComposing`.
  useEffect(() => {
    setComposing(typing && draft.trim().length > 0);
  }, [typing, draft, setComposing]);
  useEffect(() => () => setComposing(false), [setComposing]);

  // Back to voice after a while. Every keystroke restarts the clock.
  useEffect(() => {
    if (!typing) return;
    const timer = setTimeout(
      () => change(false),
      typingReturnDelay(draft, justSent),
    );
    return () => clearTimeout(timer);
  }, [typing, draft, justSent, activity, change]);

  // Talking is the other way of saying "I'm done typing". Only with an empty
  // box: speech over a half-written message is ignored (see `setComposing`)
  // and the message is not thrown away.
  useEffect(() => {
    if (typing && phase === "hearing" && !draft.trim()) change(false);
  }, [typing, phase, draft, change]);

  const send = () => {
    const text = draft.trim();
    if (!text || !live) return;
    setDraft("");
    setJustSent(true);
    void voice.sendText(text);
    input.current?.focus();
  };

  if (typing) {
    return (
      <div className="call-bar-slot">
        <div
          ref={bar}
          className="call-bar typing"
          data-hit-surface
          data-tauri-drag-region
        >
          <IconButton
            label={t("assistant.conversation.backToVoice")}
            onClick={() => change(false)}
          >
            <AudioLines />
          </IconButton>
          <input
            ref={input}
            className="call-input"
            type="text"
            value={draft}
            disabled={!live}
            placeholder={t("assistant.conversation.placeholder", { name })}
            aria-label={t("assistant.conversation.type")}
            onChange={(event) => {
              setDraft(event.target.value);
              setJustSent(false);
            }}
            onFocus={() => setActivity((n) => n + 1)}
            onPointerDown={() => setActivity((n) => n + 1)}
            onKeyDown={(event) => {
              setActivity((n) => n + 1);
              if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                event.preventDefault();
                send();
              } else if (event.key === "Escape") {
                // Escape anywhere else on a call hangs up; here it only puts
                // the text box away (clearing a draft first), so a stray press
                // cannot drop the call mid-sentence.
                event.preventDefault();
                if (draft) setDraft("");
                else change(false);
              }
            }}
          />
          {replying && !draft.trim() ? (
            <button
              type="button"
              className="call-send stop"
              onClick={voice.stopReply}
              aria-label={t("assistant.conversation.stop")}
              title={t("assistant.conversation.stop")}
            >
              <Square size={12} strokeWidth={2.6} />
            </button>
          ) : (
            <button
              type="button"
              className="call-send"
              onClick={send}
              disabled={!live || !draft.trim()}
              aria-label={t("assistant.conversation.send")}
              title={t("assistant.conversation.send")}
            >
              <ArrowUp size={16} strokeWidth={2.5} />
            </button>
          )}
          <span className="call-divider" aria-hidden="true" />
          <CallOrb voice={voice} onEnd={onEnd} />
        </div>
      </div>
    );
  }

  return (
    <div className="call-bar-slot">
      <div
        ref={bar}
        className="call-bar"
        data-hit-surface
        data-tauri-drag-region
      >
        <IconButton
          label={t("assistant.conversation.type")}
          onClick={() => change(true)}
          disabled={!live}
        >
          <SquarePen />
        </IconButton>
        <IconButton
          label={t(
            voice.muted
              ? "assistant.conversation.unmute"
              : "assistant.conversation.mute",
          )}
          pressed={voice.muted}
          tone="danger"
          disabled={!live}
          onClick={() => void voice.toggleMute()}
        >
          {voice.muted ? <MicOff /> : <Mic />}
        </IconButton>
        <span className="call-divider" aria-hidden="true" />
        <CallOrb voice={voice} onEnd={onEnd} />
        <span className="call-divider" aria-hidden="true" />
        <IconButton
          label={t(
            voice.speakerOff
              ? "assistant.conversation.speakerOn"
              : "assistant.conversation.speakerOff",
          )}
          pressed={voice.speakerOff}
          disabled={voice.phase === "off" || voice.phase === "error"}
          onClick={() => void voice.toggleSpeaker()}
        >
          {voice.speakerOff ? <VolumeX /> : <Volume2 />}
        </IconButton>
      </div>
    </div>
  );
}

type BubbleContent =
  | {
      kind: "error";
      text: string;
      detail?: string;
      retry?: boolean;
      dismiss: () => void;
    }
  | { kind: "status"; text: string; busy?: boolean; level?: boolean }
  | { kind: "hint"; text: string };

interface BubbleProps {
  voice: Voice;
  activity: string | null;
  voiceLoading: number | null;
  voiceFault: string | null;
  onDismissFault: () => void;
  hasConversation: boolean;
}

/**
 * What the bubble above the bar should say right now, if anything.
 *
 * States only — listening, thinking, what a tool is doing, an error — never the
 * words of the reply. The bubble used to stream the answer's tail as it was
 * written, and it could not do that job: a few dozen characters of a sentence
 * that is also being spoken aloud is too little to read and enough to distract,
 * and the voice already delivers the whole thing. The full text is one click
 * away in the conversation.
 */
function useBubbleContent({
  voice,
  activity,
  voiceLoading,
  voiceFault,
  onDismissFault,
  hasConversation,
}: BubbleProps): BubbleContent | null {
  const { t } = useTranslation();
  if (voiceFault)
    return { kind: "error", text: voiceFault, dismiss: onDismissFault };
  if (voice.error)
    return {
      kind: "error",
      text: t(`assistant.conversation.errors.${voice.error.code}`),
      detail: voice.error.detail,
      retry: voice.phase === "error",
      dismiss: voice.clearError,
    };
  const phase = voice.phase;
  if (voiceLoading !== null && phase !== "hearing")
    return {
      kind: "status",
      busy: true,
      text: t("assistant.conversation.preparingVoice", {
        progress: voiceLoading,
      }),
    };
  if (phase === "starting" || phase === "transcribing")
    return {
      kind: "status",
      busy: true,
      text: t(`assistant.conversation.phase.${phase}`),
    };
  if (phase === "hearing")
    return {
      kind: "status",
      level: true,
      text: t("assistant.conversation.phase.hearing"),
    };
  if (phase === "responding")
    return {
      kind: "status",
      busy: true,
      text: activity ?? t("assistant.conversation.phase.responding"),
    };
  if (phase === "speaking")
    return { kind: "status", text: t("assistant.conversation.phase.speaking") };
  if (phase === "muted")
    return { kind: "hint", text: t("assistant.conversation.mutedHint") };
  if (phase === "held")
    return { kind: "hint", text: t("assistant.conversation.heldHint") };
  if (phase === "listening" && !hasConversation)
    return { kind: "hint", text: t("assistant.conversation.startHint") };
  return null;
}

/** The status bubble above the bar. Clicking it opens the conversation. */
function CallBubble({
  onOpen,
  errorsOnly = false,
  ...props
}: BubbleProps & { onOpen?: () => void; errorsOnly?: boolean }) {
  const { t } = useTranslation();
  const content = useBubbleContent(props);
  const openLabel = t("assistant.conversation.expand");

  if (!content) {
    if (errorsOnly || !onOpen) return null;
    // Nothing to say, so nothing is shown — but the way into the conversation
    // must not disappear with it. Hovering the call brings an "Open
    // conversation" chip back for as long as the pointer is there.
    return (
      <div className="call-bubble peek" data-hit-surface>
        <button
          type="button"
          className="call-bubble-open"
          onClick={onOpen}
          aria-label={openLabel}
          title={openLabel}
        >
          <span>{openLabel}</span>
          <Maximize2 size={12} className="call-bubble-go" aria-hidden="true" />
        </button>
      </div>
    );
  }
  if (errorsOnly && content.kind !== "error") return null;

  if (content.kind === "error") {
    return (
      <div
        className="call-bubble error"
        role="alert"
        data-hit-surface
        data-tauri-drag-region
      >
        <p>{content.text}</p>
        {content.detail && (
          <details>
            <summary>{t("assistant.conversation.details")}</summary>
            <p>{content.detail}</p>
          </details>
        )}
        {content.retry && (
          <button
            type="button"
            className="call-bubble-action"
            onClick={() => void props.voice.start()}
          >
            <RotateCcw size={13} />
            {t("assistant.conversation.retry")}
          </button>
        )}
        <button
          type="button"
          className="call-bubble-dismiss"
          onClick={content.dismiss}
          aria-label={t("assistant.conversation.dismiss")}
          title={t("assistant.conversation.dismiss")}
        >
          <X size={13} strokeWidth={2.5} />
        </button>
      </div>
    );
  }

  return (
    <div
      className={`call-bubble ${content.kind}`}
      role="status"
      aria-live="polite"
      data-hit-surface
    >
      <button
        type="button"
        className="call-bubble-open"
        onClick={onOpen}
        disabled={!onOpen}
        aria-label={`${content.text} — ${openLabel}`}
        title={openLabel}
      >
        {content.kind === "status" && content.busy && (
          <Loader2 size={13} className="call-spin" aria-hidden="true" />
        )}
        {content.kind === "status" && content.level && (
          <span
            className="call-level"
            aria-hidden="true"
            style={{ "--level": props.voice.level } as CSSProperties}
          >
            <span />
            <span />
            <span />
          </span>
        )}
        <span>{content.text}</span>
        {onOpen && (
          <Maximize2 size={12} className="call-bubble-go" aria-hidden="true" />
        )}
      </button>
    </div>
  );
}

/** Which day bucket a conversation's last activity falls in. */
export type HistoryGroup = "today" | "yesterday" | "week" | "earlier";

/**
 * The day bucket for `seconds`, against local calendar days (so "yesterday"
 * is the day before today, not the 24 hours before now). Pure, for tests.
 */
export function historyGroup(
  seconds: number,
  now: Date = new Date(),
): HistoryGroup {
  const day = (offset: number) =>
    new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate() - offset,
    ).getTime();
  const at = seconds * 1000;
  if (at >= day(0)) return "today";
  if (at >= day(1)) return "yesterday";
  if (at >= day(6)) return "week";
  return "earlier";
}

/** Consecutive runs of entries in the same bucket, in the list's own order. */
export function groupConversations<
  T extends { updated_at: number; timestamp: number },
>(
  entries: T[],
  now: Date = new Date(),
): { group: HistoryGroup; entries: T[] }[] {
  const groups: { group: HistoryGroup; entries: T[] }[] = [];
  for (const entry of entries) {
    const group = historyGroup(entry.updated_at || entry.timestamp, now);
    const last = groups[groups.length - 1];
    if (last && last.group === group) last.entries.push(entry);
    else groups.push({ group, entries: [entry] });
  }
  return groups;
}

type HistoryScope = "all" | "meetings";
const HISTORY_SCOPES: HistoryScope[] = ["all", "meetings"];
/** How long typing in the search box pauses before the list is asked again. */
const SEARCH_DEBOUNCE_MS = 220;

/**
 * Past conversations, openable inside the live call.
 *
 * Searchable (titles, the meeting a conversation was about, and everything
 * said), filterable to the ones about meetings, and grouped by when they were
 * last active — a list ordered by when each started put a week-old
 * conversation continued this morning at the bottom.
 */
function CallHistory({
  activeId,
  onOpen,
  onClose,
}: {
  activeId: number | null;
  onOpen: (id: number) => Promise<boolean>;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [entries, setEntries] = useState<AssistantHistorySummary[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [status, setStatus] = useState<"loading" | "ready" | "failed">(
    "loading",
  );
  const [opening, setOpening] = useState<number | null>(null);
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  const [scope, setScope] = useState<HistoryScope>("all");
  const entriesRef = useRef<AssistantHistorySummary[]>([]);
  const filterRef = useRef<AssistantHistoryFilter>({
    query: null,
    meetings_only: false,
    meeting_id: null,
  });
  const busyRef = useRef(false);
  const generationRef = useRef(0);

  useEffect(() => {
    entriesRef.current = entries;
  }, [entries]);

  useEffect(() => {
    const timer = setTimeout(() => setSearch(query.trim()), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [query]);

  /**
   * `fresh` starts over (a new search or filter), `refresh` re-reads what is
   * showing (a turn was saved), `more` appends the next page.
   */
  const load = useCallback(async (mode: "fresh" | "refresh" | "more") => {
    if (mode === "more" && busyRef.current) return;
    // A reload supersedes a page still in flight, which would otherwise be
    // appended after the fresh list.
    if (mode !== "more") generationRef.current += 1;
    const generation = generationRef.current;
    busyRef.current = true;
    if (mode === "fresh") setStatus("loading");
    const offset = mode === "more" ? entriesRef.current.length : 0;
    // A refresh asks for as many rows as are showing, so a turn saved while
    // the user is further down does not fold the list back to its first page.
    const limit =
      mode === "refresh"
        ? Math.max(HISTORY_PAGE, entriesRef.current.length)
        : HISTORY_PAGE;
    try {
      const result = await commands.listAssistantConversations(
        filterRef.current,
        offset,
        limit,
      );
      if (generation !== generationRef.current) return;
      if (result.status !== "ok") throw new Error(result.error);
      setEntries((previous) =>
        mode === "more"
          ? [...previous, ...result.data.entries]
          : result.data.entries,
      );
      setHasMore(result.data.has_more);
      setStatus("ready");
    } catch {
      if (generation === generationRef.current) setStatus("failed");
    } finally {
      if (generation === generationRef.current) busyRef.current = false;
    }
  }, []);

  useEffect(() => {
    filterRef.current = {
      query: search || null,
      meetings_only: scope === "meetings",
      meeting_id: null,
    };
    void load("fresh");
  }, [search, scope, load]);

  // Every turn saves the conversation it belongs to, so the list is kept
  // current while it is open rather than only when it is first shown.
  useEffect(() => {
    const unlisten = listen("assistant-history-updated", () => {
      void load("refresh");
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [load]);

  const open = async (id: number) => {
    setOpening(id);
    try {
      await onOpen(id);
    } finally {
      setOpening(null);
    }
  };

  const groups = groupConversations(entries);
  const narrowed = search !== "" || scope !== "all";
  const emptyKey = search
    ? "assistant.conversation.history.noMatches"
    : scope === "meetings"
      ? "assistant.conversation.history.noMeetings"
      : "assistant.conversation.history.empty";

  return (
    <div className="call-history">
      <div className="call-history-head">
        <h2>{t("assistant.conversation.history.title")}</h2>
        <p>{t("assistant.conversation.history.hint")}</p>
      </div>
      <div className="call-history-tools">
        <label className="call-history-search">
          <Search size={13} aria-hidden="true" />
          <input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              // Escape anywhere else on a call hangs up. Here it clears the
              // search, then closes the list.
              if (event.key !== "Escape") return;
              event.preventDefault();
              if (query) setQuery("");
              else onClose();
            }}
            placeholder={t("assistant.conversation.history.search")}
            aria-label={t("assistant.conversation.history.search")}
            spellCheck={false}
          />
          {query && (
            <button
              type="button"
              className="call-history-clear"
              onClick={() => setQuery("")}
              aria-label={t("assistant.conversation.history.clearSearch")}
              title={t("assistant.conversation.history.clearSearch")}
            >
              <X size={12} />
            </button>
          )}
        </label>
        <div
          className="call-history-scope"
          role="group"
          aria-label={t("assistant.conversation.history.scopeLabel")}
        >
          {HISTORY_SCOPES.map((value) => (
            <button
              key={value}
              type="button"
              aria-pressed={scope === value}
              onClick={() => setScope(value)}
            >
              {value === "meetings" && <Users size={11} aria-hidden="true" />}
              {t(`assistant.conversation.history.scopes.${value}`)}
            </button>
          ))}
        </div>
      </div>
      {status === "loading" && (
        <p className="call-history-note">
          {t("assistant.conversation.history.loading")}
        </p>
      )}
      {status === "failed" && (
        <p className="call-history-note error">
          {t("assistant.conversation.history.failed")}
        </p>
      )}
      {status === "ready" && entries.length === 0 && (
        <p className="call-history-note">{t(emptyKey)}</p>
      )}
      {groups.map(({ group, entries: rows }) => (
        <section className="call-history-group" key={`${group}-${rows[0].id}`}>
          <h3>{t(`assistant.conversation.history.groups.${group}`)}</h3>
          <ul className="call-history-list">
            {rows.map((entry) => {
              const current = entry.id === activeId;
              const meta = [
                relativeTime(
                  entry.updated_at || entry.timestamp,
                  i18n.language,
                ),
                t("assistant.conversation.history.messages", {
                  count: entry.message_count,
                }),
              ].join(" · ");
              return (
                <li key={entry.id}>
                  <button
                    type="button"
                    className={`call-history-item${current ? " current" : ""}`}
                    aria-current={current || undefined}
                    disabled={opening !== null}
                    onClick={() => void open(entry.id)}
                  >
                    <span className="call-history-title">
                      {entry.title.trim() ||
                        t("assistant.conversation.history.untitled")}
                    </span>
                    {entry.meeting_id !== null && (
                      <span className="call-history-meeting">
                        <Users size={11} aria-hidden="true" />
                        <span>
                          {entry.meeting_title?.trim() ||
                            t("assistant.conversation.meeting.untitled")}
                        </span>
                      </span>
                    )}
                    <span className="call-history-meta">
                      {current
                        ? `${t("assistant.conversation.history.current")} · ${meta}`
                        : meta}
                    </span>
                    {opening === entry.id && (
                      <Loader2
                        size={14}
                        className="call-spin call-history-spin"
                        aria-hidden="true"
                      />
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        </section>
      ))}
      {hasMore && status === "ready" && (
        <button
          type="button"
          className="call-history-more"
          onClick={() => void load("more")}
        >
          {t("assistant.conversation.history.more")}
        </button>
      )}
      {narrowed && status === "ready" && entries.length > 0 && !hasMore && (
        <p className="call-history-note end">
          {t("assistant.conversation.history.matchCount", {
            count: entries.length,
          })}
        </p>
      )}
    </div>
  );
}

/** Things worth asking straight after a meeting, as chips on an empty call. */
const MEETING_PROMPTS = ["recap", "next", "followUp", "missed"] as const;

/**
 * What an empty call about a meeting shows: which meeting, and a few ways in.
 * A chip sends its question as a typed message, so it is answered (and spoken)
 * exactly as if the user had said it.
 */
export function MeetingStarter({
  meeting,
  onAsk,
  disabled,
}: {
  meeting: MeetingAttachment;
  onAsk: (text: string) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div className="call-meeting-start">
      <p className="call-meeting-start-title">
        {t("assistant.conversation.meeting.startTitle", {
          title:
            meeting.title.trim() ||
            t("assistant.conversation.meeting.untitled"),
        })}
      </p>
      <p className="call-meeting-start-hint">
        {t("assistant.conversation.meeting.startHint")}
      </p>
      <div className="call-meeting-prompts">
        {MEETING_PROMPTS.map((key) => {
          const text = t(`assistant.conversation.meeting.prompts.${key}`);
          return (
            <button
              key={key}
              type="button"
              disabled={disabled}
              onClick={() => onAsk(text)}
            >
              {text}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/**
 * How readily the call hears speech.
 *
 * This popover used to carry a second dial, "time to finish speaking" (how long
 * a pause ends your turn). It did work — it set the voice detector's silence
 * window to 450, 700 or 1100 ms — but it asked people to tune a number they
 * have no way to reason about, and a pause mid-thought is already handled
 * without it: speech that resumes carries the first fragment forward
 * (`voice_conversation::carry`). The call now always uses the middle value.
 */
function CallOptions({ voice }: { voice: Voice }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", outside);
    return () => window.removeEventListener("pointerdown", outside);
  }, [open]);

  return (
    <div
      className="call-options"
      ref={root}
      onKeyDown={(event) => {
        if (event.key === "Escape" && open) {
          event.preventDefault();
          event.stopPropagation();
          setOpen(false);
          trigger.current?.focus();
        }
      }}
    >
      <button
        ref={trigger}
        type="button"
        className="call-icon small"
        aria-expanded={open}
        aria-label={t("assistant.conversation.options")}
        title={t("assistant.conversation.options")}
        onClick={() => setOpen(!open)}
      >
        <SlidersHorizontal />
      </button>
      {open && (
        <div
          className="call-options-popover"
          role="group"
          aria-label={t("assistant.conversation.options")}
        >
          <label htmlFor="call-sensitivity">
            {t("assistant.conversation.sensitivity")}
          </label>
          <p>{t("assistant.conversation.sensitivityHint")}</p>
          <select
            id="call-sensitivity"
            value={voice.sensitivity}
            onChange={(event) =>
              voice.setSensitivity(
                event.target.value as ConversationSensitivity,
              )
            }
          >
            {CONVERSATION_SENSITIVITIES.map((level) => (
              <option key={level} value={level}>
                {t(`assistant.conversation.sensitivities.${level}`)}
              </option>
            ))}
          </select>
        </div>
      )}
    </div>
  );
}

export interface CallSurfaceProps {
  voice: Voice;
  /**
   * Which form the call is in (see `useCallForm`): the bar, the conversation
   * open above it, or the conversation folding back down into the bar.
   */
  form: CallForm;
  /** Open the conversation (the bubble, the "Open conversation" chip). */
  onExpand: () => void;
  /** Fold the conversation back down into the bar. */
  onCollapse: () => void;
  /** Hang up — the window goes with the call. */
  onEnd: () => void;
  /** The active profile's name, for the message placeholder. */
  name: string;
  profilePicker: ReactNode;
  /** The rendered message list, shown when expanded. */
  transcript: ReactNode;
  /** The meeting this conversation is about, if any. */
  meeting?: MeetingAttachment | null;
  /** Edge grips for the expanded window. */
  resizeHandles: ReactNode;
  /** The conversation has at least one message. */
  hasConversation: boolean;
  /** What a running tool is doing ("Searching the web · …"). */
  activity: string | null;
  /** Percent while the on-device voice loads, else `null`. */
  voiceLoading: number | null;
  /** A voice-engine failure, already translated. */
  voiceFault: string | null;
  onDismissFault: () => void;
}

export function CallSurface({
  voice,
  form,
  onExpand,
  onCollapse,
  onEnd,
  name,
  profilePicker,
  transcript,
  meeting = null,
  resizeHandles,
  hasConversation,
  activity,
  voiceLoading,
  voiceFault,
  onDismissFault,
}: CallSurfaceProps) {
  const { t } = useTranslation();
  const [typing, setTyping] = useState(false);
  const [showHistory, setShowHistory] = useState(false);
  // Which saved conversation is open in the call, when one was opened from the
  // list. The backend does not report it; only this view needs it.
  const [activeHistoryId, setActiveHistoryId] = useState<number | null>(null);
  const { newConversation, loadConversation } = voice;

  // An empty conversation is not a saved one, whatever emptied it (New chat,
  // a meeting opened for discussion, a call that started fresh).
  useEffect(() => {
    if (!hasConversation) setActiveHistoryId(null);
  }, [hasConversation]);

  // A meeting brought in for discussion is what the user wants to see.
  const meetingId = meeting?.meetingId ?? null;
  useEffect(() => {
    if (meetingId !== null) setShowHistory(false);
  }, [meetingId]);

  const newChat = useCallback(async () => {
    if (await newConversation()) {
      setActiveHistoryId(null);
      setShowHistory(false);
    }
  }, [newConversation]);

  const openSaved = useCallback(
    async (id: number) => {
      const opened = await loadConversation(id);
      if (opened) {
        setActiveHistoryId(id);
        setShowHistory(false);
      }
      return opened;
    },
    [loadConversation],
  );

  const bubble = {
    voice,
    activity,
    voiceLoading,
    voiceFault,
    onDismissFault,
    hasConversation,
  };

  const bar = (
    <CallBar
      voice={voice}
      name={name}
      typing={typing}
      onTypingChange={setTyping}
      onEnd={onEnd}
    />
  );

  if (form === "bar") {
    return (
      <div className="call-frame">
        <div className={`call-stack${typing ? " typing" : ""}`}>
          <CallBubble {...bubble} onOpen={onExpand} />
          {bar}
        </div>
      </div>
    );
  }

  const status =
    activity && voice.phase === "responding"
      ? activity
      : t(`assistant.conversation.phase.${voice.phase}`);
  const live = !DEAD_PHASES.includes(voice.phase);

  return (
    <div className="call-frame expanded">
      <section
        className={`call-panel${form === "closing" ? " closing" : ""}${
          typing ? " typing" : ""
        }`}
        data-hit-surface
        aria-label={t("assistant.conversation.title")}
      >
        {resizeHandles}
        <header className="call-panel-head" data-tauri-drag-region>
          <div className="call-panel-profile">{profilePicker}</div>
          <span
            className={`call-panel-status ${voice.phase}`}
            role="status"
            aria-live="polite"
            data-tauri-drag-region
          >
            {status}
          </span>
          <div
            className="call-panel-drag"
            data-tauri-drag-region
            aria-hidden="true"
          />
          <IconButton
            label={t("assistant.conversation.newChat")}
            onClick={() => void newChat()}
            disabled={!live || !hasConversation}
          >
            <MessageSquarePlus />
          </IconButton>
          <IconButton
            label={t(
              showHistory
                ? "assistant.conversation.historyClose"
                : "assistant.conversation.historyOpen",
            )}
            pressed={showHistory}
            onClick={() => setShowHistory(!showHistory)}
          >
            <History />
          </IconButton>
          <CallOptions voice={voice} />
          <IconButton
            label={t("assistant.conversation.collapse")}
            onClick={onCollapse}
          >
            <ChevronDown />
          </IconButton>
        </header>
        <div className="call-panel-body">
          {meeting && !showHistory && (
            <div
              className="call-meeting-chip"
              title={t("assistant.conversation.meeting.chipHint")}
            >
              <Users size={12} aria-hidden="true" />
              <span>
                {t("assistant.conversation.meeting.about", {
                  title:
                    meeting.title.trim() ||
                    t("assistant.conversation.meeting.untitled"),
                })}
              </span>
            </div>
          )}
          {/* Kept mounted behind the history list, so its scroll position and
              the panel's stick-to-bottom tracking survive a look at history. */}
          <div className="call-panel-transcript" hidden={showHistory}>
            {transcript}
          </div>
          {showHistory && (
            <CallHistory
              activeId={activeHistoryId}
              onOpen={openSaved}
              onClose={() => setShowHistory(false)}
            />
          )}
        </div>
        <div className={`call-panel-foot call-stack${typing ? " typing" : ""}`}>
          <CallBubble {...bubble} errorsOnly />
          {bar}
        </div>
      </section>
    </div>
  );
}
