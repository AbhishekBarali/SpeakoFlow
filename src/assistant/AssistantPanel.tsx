import { emit, listen } from "@tauri-apps/api/event";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import {
  Camera,
  Check,
  Copy,
  CornerDownLeft,
  FileText,
  Globe,
  ImagePlus,
  Loader2,
  TextSelect,
} from "lucide-react";
import { commands, type AppSettings, type MeetingAttachment } from "@/bindings";
import { syncLanguageFromSettings } from "@/i18n";
import { FONT_SIZES, type AssistantError } from "./appearance";
import { useKokoroTts } from "./useKokoroTts";
import { localTtsActive } from "./localTts";
import { parseKokoroDevice, useLocalVoiceStatus } from "./localVoice";
import { usePanelHitRegion, useSuppressContextMenu } from "./hitRegion";
import { useVoiceConversation } from "./useVoiceConversation";
import { CallSurface, MeetingStarter } from "./CallBar";
import { useCallForm } from "./useCallForm";
import { AssistantProfilePicker } from "./AssistantProfilePicker";
import QuickAsk from "./QuickAsk";
import MarkdownMessage from "./MarkdownMessage";
import {
  DEFAULT_LAYOUT,
  parseLayout,
  quickAskPhase,
  workingLabelKey,
  type AssistantState,
  type QuickAskLayout,
  type QuickAskPhase,
} from "./quickAskState";
import { VOICE_INTERRUPTED_MARKER } from "./conversationPolicy";
import { useLocalLlmEngineStatus } from "@/hooks/useLocalLlmEngineStatus";
import { beginSafeResize, useSafeWindowDrag } from "@/lib/useSafeWindowDrag";
import "./AssistantPanel.css";
import "@/lib/windowActivity.css";

/** A tool the current turn is running, as reported by the backend. */
interface ToolActivity {
  /** Backend tool id, e.g. `web_search`. */
  name: string;
  /** The one argument worth showing — currently only the search query. */
  detail: string;
  /** How many tools this round called; they run concurrently. */
  count: number;
  /** When the panel first saw it, for the elapsed counter. */
  startedAt: number;
}

/** What the call picks up when it opens (History → Continue, a meeting's
 *  "Discuss in a call"). Exactly one of `id` and `meetingId` is set. */
interface CallContinuation {
  id: number | null;
  messageIndex: number | null;
  meetingId: number | null;
}

interface DisplayMessage {
  role: "user" | "assistant";
  content: string;
  screenshot?: boolean;
  images?: number;
  files?: string[];
  /** Display thumbnails (data URLs) for the visuals sent with this message —
   *  the screen capture first (if any), then attached images. */
  thumbnails?: string[];
  /** How many characters of text the user had selected in another application
   *  when they asked. The block itself is collapsed into a chip rather than
   *  echoed back at them. */
  selectionChars?: number;
}

/** Must match the marker constants in src-tauri/src/assistant.rs */
const SCREENSHOT_MARKER = "[screenshot attached]";
const IMAGE_MARKER = "[image attached]";
const FILE_MARKER_PREFIX = "[file attached:";
/** Delimiters around text the user had selected elsewhere. Must match
 *  `SELECTION_OPEN` / `SELECTION_CLOSE` in src-tauri/src/assistant.rs. */
const SELECTION_OPEN = "<selected_text>";
const SELECTION_CLOSE = "</selected_text>";
/** The two fixed phrases `compose_selection_request` writes around the block.
 *  Stripped for display: the chip already says the answer is about a selection. */
const SELECTION_LEAD_IN =
  "The user has this text selected in another application:";
const SELECTION_REQUEST_PREFIX = "Their request about it: ";

/**
 * How long a voice-opened quick ask that ended up with nothing to show waits
 * before it puts itself away. Long enough to ride over the gap between two
 * pipeline states, short enough that a mis-tap does not leave a prompt behind.
 */
const EMPTY_ASK_DISMISS_MS = 350;

/** How long a voice ask's error stays on screen when nobody is pointing at it. */
const ERROR_DISMISS_MS = 9000;

function toDisplay(raw: {
  role: string;
  content: string;
  images?: string[];
}): DisplayMessage {
  const role = raw.role === "assistant" ? "assistant" : "user";
  let screenshot = false;
  let images = 0;
  const files: string[] = [];
  const kept: string[] = [];
  // A selection can be thousands of characters, and re-reading your own
  // highlighted paragraph above the answer is noise. The block is replaced by a
  // chip saying how much text was attached. Keep these delimiters in sync with
  // assistant.rs.
  let selectionChars = 0;
  let inSelection = false;
  for (const line of raw.content.split("\n")) {
    const trimmed = line.trim();
    if (trimmed === SELECTION_OPEN) {
      inSelection = true;
      continue;
    }
    if (trimmed === SELECTION_CLOSE) {
      inSelection = false;
      continue;
    }
    if (inSelection) {
      selectionChars += line.length;
      continue;
    }
    if (trimmed === SELECTION_LEAD_IN) continue;
    if (trimmed === VOICE_INTERRUPTED_MARKER) continue;
    if (trimmed === SCREENSHOT_MARKER) {
      screenshot = true;
      continue;
    }
    if (trimmed === IMAGE_MARKER) {
      images += 1;
      continue;
    }
    if (trimmed.startsWith(FILE_MARKER_PREFIX) && trimmed.endsWith("]")) {
      files.push(trimmed.slice(FILE_MARKER_PREFIX.length, -1).trim());
      continue;
    }
    kept.push(line);
  }
  const thumbnails = raw.images && raw.images.length ? raw.images : undefined;
  return {
    role,
    content: kept
      .join("\n")
      .trim()
      .replace(SELECTION_REQUEST_PREFIX, "")
      .trim(),
    screenshot: screenshot || undefined,
    images: images || undefined,
    files: files.length ? files : undefined,
    selectionChars: selectionChars || undefined,
    thumbnails,
  };
}

const CopyButton: React.FC<{ content: string; title: string }> = ({
  content,
  title,
}) => {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    await navigator.clipboard.writeText(content);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  return (
    <button className="bubble-copy" onClick={handleCopy} title={title}>
      {copied ? <Check size={13} /> : <Copy size={13} />}
    </button>
  );
};

/** Writes an answer from the call transcript into the app it was asked from.
 *  Always an explicit click, never automatic. */
const InsertButton: React.FC<{ content: string; title: string }> = ({
  content,
  title,
}) => {
  const [inserted, setInserted] = useState(false);

  const handleInsert = async () => {
    const result = await commands.assistantInsertText(content);
    if (result.status === "ok") {
      setInserted(true);
      setTimeout(() => setInserted(false), 1200);
    }
  };

  return (
    <button
      className="bubble-insert"
      onClick={() => void handleInsert()}
      title={title}
      aria-label={title}
    >
      {inserted ? <Check size={13} /> : <CornerDownLeft size={13} />}
    </button>
  );
};

/** `<pre>` renderer with a hover copy button, so each code block is
 *  individually copyable (the whole-answer copy stays too). */
const CodeBlock: React.FC<React.HTMLAttributes<HTMLPreElement>> = ({
  children,
  ...rest
}) => {
  const { t } = useTranslation();
  const preRef = useRef<HTMLPreElement>(null);
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    const text = preRef.current?.innerText ?? "";
    if (!text) return;
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  return (
    <div className="code-block">
      <pre ref={preRef} {...rest}>
        {children}
      </pre>
      <button
        className="code-copy"
        onClick={handleCopy}
        title={t("assistant.copyCode")}
      >
        {copied ? <Check size={12} /> : <Copy size={12} />}
      </button>
    </div>
  );
};

/** Small inline preview thumbnails of the image(s) sent with a call message.
 *  Clicking one pops a larger preview; click again, the backdrop, or Esc to
 *  dismiss. */
const MessageThumbnails: React.FC<{
  urls: string[];
  hasScreen?: boolean;
  screenLabel: string;
}> = ({ urls, hasScreen, screenLabel }) => {
  const [preview, setPreview] = useState<{
    url: string;
    left: number;
    top: number;
    above: boolean;
  } | null>(null);

  const toggle = (el: HTMLElement, url: string) => {
    setPreview((cur) => {
      if (cur && cur.url === url) return null;
      const rect = el.getBoundingClientRect();
      const maxW = Math.min(320, window.innerWidth - 24);
      let left = rect.left + rect.width / 2 - maxW / 2;
      left = Math.max(12, Math.min(left, window.innerWidth - maxW - 12));
      const above = rect.top > 220;
      const top = above ? rect.top - 8 : rect.bottom + 8;
      return { url, left, top, above };
    });
  };

  // Escape closes the enlarged image and nothing else: the call's own Escape
  // (hang up) listens on the same window.
  useEffect(() => {
    if (!preview) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopImmediatePropagation();
      setPreview(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [preview]);

  return (
    <div className="msg-thumbs">
      {urls.map((url, i) => (
        <button
          key={i}
          type="button"
          className="msg-thumb"
          onClick={(e) => toggle(e.currentTarget, url)}
          aria-label={hasScreen && i === 0 ? screenLabel : undefined}
        >
          <img src={url} alt="" draggable={false} />
          {hasScreen && i === 0 && (
            <span className="msg-thumb-badge" aria-hidden>
              <Camera size={9} strokeWidth={2.5} />
            </span>
          )}
        </button>
      ))}
      {preview && (
        <>
          <div
            className="msg-thumb-backdrop"
            onClick={() => setPreview(null)}
          />
          <div
            className={`msg-thumb-preview${preview.above ? " above" : ""}`}
            style={{ left: preview.left, top: preview.top }}
            onClick={() => setPreview(null)}
          >
            <img
              className="msg-thumb-preview-img"
              src={preview.url}
              alt=""
              draggable={false}
            />
          </div>
        </>
      )}
    </div>
  );
};

/** Shared react-markdown renderers (module scope — stable identity). */
const MD_COMPONENTS = { pre: CodeBlock };

/** Invisible edge/corner grips that drive Tauri's native window resize for the
 *  expanded call — the only form of this window the user resizes. */
// Mirrors Tauri's (non-exported) ResizeDirection string union.
type ResizeDir =
  | "North"
  | "South"
  | "East"
  | "West"
  | "NorthEast"
  | "NorthWest"
  | "SouthEast"
  | "SouthWest";

const RESIZE_HANDLES: { cls: string; dir: ResizeDir }[] = [
  { cls: "n", dir: "North" },
  { cls: "s", dir: "South" },
  { cls: "e", dir: "East" },
  { cls: "w", dir: "West" },
  { cls: "ne", dir: "NorthEast" },
  { cls: "nw", dir: "NorthWest" },
  { cls: "se", dir: "SouthEast" },
  { cls: "sw", dir: "SouthWest" },
];

const ResizeHandles: React.FC = () => {
  // Threshold-gated: a resize begun on the press itself could outlive a quick
  // click and leave the window resizing with the cursor (see beginSafeResize).
  const onDown = (e: React.MouseEvent, dir: ResizeDir) =>
    beginSafeResize(e, dir);
  return (
    <>
      {RESIZE_HANDLES.map(({ cls, dir }) => (
        <div
          key={cls}
          className={`assistant-resize ${cls}`}
          onMouseDown={(e) => onDown(e, dir)}
        />
      ))}
    </>
  );
};

const AssistantPanel: React.FC = () => {
  const { t } = useTranslation();
  // Window dragging with a movement threshold, so a click on the surface is a
  // click rather than the start of Windows' modal move loop.
  useSafeWindowDrag();
  // Conversation snapshots from the backend are the single source of truth;
  // `stream` only holds the in-flight answer between snapshots.
  const [history, setHistory] = useState<DisplayMessage[]>([]);
  const [stream, setStream] = useState("");
  const [error, setError] = useState<AssistantError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [state, setState] = useState<AssistantState>("idle");
  // Background rolling-summary pass (long calls).
  const [summarizing, setSummarizing] = useState(false);
  const [input, setInput] = useState("");
  // Is the panel window actually on screen? It is built hidden at launch and
  // only shown on a hotkey/turn, so anything expensive (the local TTS weights)
  // waits for this rather than loading into a window nobody has opened.
  const [panelVisible, setPanelVisible] = useState(false);
  // Whether this window is on screen because the user asked for it (tray,
  // History, the call key) rather than as a voice ask.
  const [userOpened, setUserOpened] = useState(false);
  // A call just ended in this window and nothing has taken it over yet. The
  // window goes away a moment later (hanging up closes the panel), and until
  // then it must not fall back to drawing the idle quick ask's typing bar.
  const [callEnded, setCallEnded] = useState(false);
  // Characters of selected text the current recording picked up, reported while
  // the user is still speaking. The message itself carries the count once sent.
  const [selectionCaptured, setSelectionCaptured] = useState(0);
  const [ttsPlaying, setTtsPlaying] = useState(false);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [micLevels, setMicLevels] = useState<number[]>([]);
  // `mic-level` is emitted app-wide ~30x/s during *every* dictation, while this
  // window is usually hidden, and Tauri only wakes a webview that holds a
  // listener for it. Only a listening ask renders the levels, so the panel
  // subscribes for exactly that span instead of for its whole life.
  const listening = state === "listening";
  useEffect(() => {
    if (!listening) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    listen<number[]>("mic-level", (e) => {
      setMicLevels(e.payload);
    })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => {
        // Not running inside Tauri (tests / plain browser): no levels.
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [listening]);
  const [tool, setTool] = useState<ToolActivity | null>(null);
  // The meeting the conversation is about ("Discuss in a call"), or null.
  const [meeting, setMeeting] = useState<MeetingAttachment | null>(null);
  const [toolElapsed, setToolElapsed] = useState(0);
  const [layout, setLayout] = useState<QuickAskLayout>(DEFAULT_LAYOUT);
  // The pointer is on the quick-ask surface, which holds back the error timeout.
  const [hovered, setHovered] = useState(false);

  // Built-in (llama.cpp) engine setup progress, so the first-run download reads
  // as "Setting up the local engine…" rather than a silent "Thinking…".
  const engine = useLocalLlmEngineStatus();
  const engineSetupActive = engine.active;
  const engineSetupLabel = engineSetupActive
    ? engine.phase === "extracting"
      ? t("assistant.engineSetup.extracting")
      : engine.total > 0
        ? t("assistant.engineSetup.downloading", { percent: engine.pct })
        : t("assistant.engineSetup.preparing")
    : "";
  const listRef = useRef<HTMLDivElement>(null);
  const sendingRef = useRef(false);
  // Streaming is smoothed: raw tokens accumulate in a buffer and are flushed to
  // React state at most once per animation frame, so a fast provider cannot
  // re-parse the whole growing markdown reply per token.
  const streamBufferRef = useRef("");
  const streamRafRef = useRef<number | null>(null);
  // Auto-scroll follows new content only while the user is already near the
  // bottom, so a streaming reply never yanks them down while they scroll up.
  const stickToBottomRef = useRef(true);
  const prevHistoryLenRef = useRef(0);
  const prevStateRef = useRef<AssistantState>("idle");
  // Tracks whether audio actually began during the current "speaking" phase,
  // so we can detect when playback *ends* and hand the UI back to idle.
  const spokeRef = useRef(false);
  // Set when the user presses Stop; blocks a TTS event that was emitted just
  // before the Stop from slipping through.
  const suppressTtsRef = useRef(false);
  const kokoroErrorRef = useRef(false);
  const localVoiceRef = useRef<ReturnType<typeof useKokoroTts> | null>(null);
  // Hanging up needs `hidePanel`, which is declared below because it needs
  // `voice`. The indirection breaks that cycle.
  const hangUpRef = useRef<() => void>(() => {});
  const voice = useVoiceConversation({
    stopLocal: () => localVoiceRef.current?.stop(),
    beginLocal: async (epoch) => {
      await localVoiceRef.current?.beginStream(epoch);
    },
    pushLocal: (text) => localVoiceRef.current?.pushText(text),
    endLocal: () => localVoiceRef.current?.endStream(),
    microphone: settings?.selected_microphone,
    outputDevice: settings?.selected_output_device,
    // The assistant's own voice volume, not the feedback-beep slider — see
    // `assistant_tts_volume` in settings.rs.
    volume: settings?.assistant_tts_volume,
    // No pace: the call always waits the middle length of pause.
    pace: null,
    onPaceChange: () => {},
    sensitivity: settings?.assistant_conversation_sensitivity,
    onSensitivityChange: (sensitivity) => {
      void commands.setAssistantConversationSensitivity(sensitivity);
    },
    speakerOn: settings?.assistant_tts_enabled ?? true,
    onHangUp: () => hangUpRef.current(),
  });

  // The call has two forms — the floating bar, and the bar under the open
  // conversation — and the window size belongs to the form (see
  // `assistant::set_conversation_expanded`).
  const callForm = useCallForm(voice.open);
  // Notice the call ending before the browser paints the frame without it. In
  // a passive effect the first frame after hang-up was the idle quick ask — its
  // typing bar — drawn in the call's place until the window went away.
  const callWasOpenRef = useRef(false);
  useLayoutEffect(() => {
    if (voice.open) {
      callWasOpenRef.current = true;
      setCallEnded(false);
    } else if (callWasOpenRef.current) {
      callWasOpenRef.current = false;
      setCallEnded(true);
    }
  }, [voice.open]);
  const callExpanded = callForm.expanded;
  const resetCallFormRef = useRef(callForm.reset);
  resetCallFormRef.current = callForm.reset;
  const expandCallRef = useRef(callForm.expand);
  expandCallRef.current = callForm.expand;

  // Speaking belongs to the call and nothing else: a quick answer is read, not
  // listened to, and `run_assistant_turn_inner` decides it the same way.
  const ttsEnabled =
    voice.open &&
    voice.phase !== "error" &&
    (!voice.speakerOff || (settings?.assistant_tts_enabled ?? true));
  const ttsEngine = settings?.assistant_tts_engine ?? "kokoro";
  const ttsVoice = settings?.assistant_tts_voice ?? "af_heart";
  const ttsDtype = settings?.assistant_tts_kokoro_dtype ?? "fp32";
  const ttsSpeed = settings?.assistant_tts_speed ?? 1;
  const kokoroDevice = parseKokoroDevice(settings?.assistant_tts_kokoro_device);
  // Only Kokoro can synthesize inside this WebView, and only while Rust routes
  // it here; see `localTts.ts` / `localVoice.ts`. On the processor route the
  // weights stay unloaded. The WebGPU probe runs once, when a call could
  // actually use it, so Automatic knows where to speak before the first reply.
  const localVoice = useLocalVoiceStatus({
    enabled: ttsEngine === "kokoro",
    probe: localTtsActive(ttsEnabled, ttsEngine) && kokoroDevice === "auto",
  });
  const kokoroEnabled =
    localTtsActive(ttsEnabled, ttsEngine) && localVoice?.route !== "native";
  const kokoroNativeReady = localVoice?.kokoro_native_ready ?? false;
  // Don't prepare the in-app model before Rust knows where Kokoro will speak:
  // on a machine that ends up on the processor it would load ~100–300 MB only
  // to drop it. The probe answers within milliseconds of a call opening, and a
  // reply that arrives first still loads the model on demand.
  const kokoroRouteKnown =
    !isTauri() ||
    (localVoice !== null &&
      !(kokoroDevice === "auto" && localVoice.webgpu === "unknown"));
  const characters = settings?.assistant_characters ?? [];
  const activeCharacterId =
    settings?.assistant_active_character_id ?? "default";
  const activeCharacter =
    characters.find((c) => c.id === activeCharacterId) ?? characters[0] ?? null;
  const tts = useKokoroTts(
    kokoroEnabled,
    ttsVoice,
    ttsDtype,
    ttsSpeed,
    (panelVisible || voice.open) && kokoroRouteKnown,
    voice.open ? voice.browserSink : undefined,
    kokoroDevice,
  );
  localVoiceRef.current = tts;
  // The event listeners are registered once on mount, so anything they call has
  // to be reached through a ref that always points at the latest value.
  const voiceRef = useRef(voice);
  voiceRef.current = voice;
  const speakRef = useRef(tts.speak);
  speakRef.current = tts.speak;
  const beginStreamRef = useRef(tts.beginStream);
  beginStreamRef.current = tts.beginStream;
  const pushTextRef = useRef(tts.pushText);
  pushTextRef.current = tts.pushText;
  const endStreamRef = useRef(tts.endStream);
  endStreamRef.current = tts.endStream;

  // Seed the visibility flag from the real window state, for a webview created
  // (or reloaded) while the panel is already on screen.
  useEffect(() => {
    let active = true;
    void getCurrentWindow()
      .isVisible()
      .then((visible) => {
        if (active && visible) setPanelVisible(true);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, []);

  const refreshSettings = useCallback(async () => {
    try {
      const result = await commands.getAppSettings();
      if (result.status === "ok") setSettings(result.data);
    } catch {
      // bindings not ready yet
    }
  }, []);

  // Clear the in-flight stream and cancel any pending frame flush, so no buffered
  // tokens leak from a finished turn into the next.
  const resetStream = useCallback(() => {
    if (streamRafRef.current !== null) {
      cancelAnimationFrame(streamRafRef.current);
      streamRafRef.current = null;
    }
    streamBufferRef.current = "";
    setStream("");
  }, []);

  const selectCharacter = useCallback(
    async (id: string) => {
      const result = await commands.setAssistantActiveCharacter(id);
      if (result.status === "error") throw new Error(String(result.error));
      await refreshSettings();
    },
    [refreshSettings],
  );

  // Text size and surface opacity, from settings.
  useEffect(() => {
    if (!settings) return;
    const root = document.documentElement;
    root.style.setProperty(
      "--as-msg-font",
      FONT_SIZES[settings.assistant_font_size ?? "medium"] ?? FONT_SIZES.medium,
    );
    root.style.setProperty(
      "--as-alpha",
      String(settings.assistant_panel_opacity ?? 1),
    );
  }, [settings]);

  // Call transcript auto-scroll: follow new content while the user sits near
  // the bottom; a freshly-added message of their own always scrolls in.
  useEffect(() => {
    const el = listRef.current;
    if (!el) return;
    const grew = history.length > prevHistoryLenRef.current;
    const lastIsUser = history[history.length - 1]?.role === "user";
    prevHistoryLenRef.current = history.length;
    if (grew && lastIsUser) stickToBottomRef.current = true;
    if (stickToBottomRef.current) el.scrollTop = el.scrollHeight;
  }, [history, stream, state, error, notice, callExpanded]);

  // Re-pin after the browser has actually laid the content out (markdown
  // reflow, decoded thumbnails, a window that was hidden measuring as zero).
  useEffect(() => {
    const el = listRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const pin = () => {
      if (stickToBottomRef.current) el.scrollTop = el.scrollHeight;
    };
    const observer = new ResizeObserver(pin);
    observer.observe(el);
    for (const child of Array.from(el.children)) observer.observe(child);
    return () => observer.disconnect();
  }, [history.length, callExpanded]);

  useEffect(() => {
    if (callExpanded) stickToBottomRef.current = true;
  }, [callExpanded]);

  const handleMessagesScroll = useCallback(() => {
    const el = listRef.current;
    if (!el) return;
    stickToBottomRef.current =
      el.scrollHeight - el.scrollTop - el.clientHeight < 64;
  }, []);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: (() => void)[] = [];

    /** Register a listener, immediately disposing it if the effect was
     *  already cleaned up (StrictMode double-mount protection). */
    const track = (unlisten: () => void) => {
      if (cancelled) unlisten();
      else unlisteners.push(unlisten);
    };

    const setup = async () => {
      await syncLanguageFromSettings();
      await refreshSettings();

      // Restore the conversation (the window can be recreated mid-call).
      try {
        const result = await commands.assistantGetConversation();
        if (result.status === "ok" && !cancelled) {
          setHistory(result.data.map(toDisplay));
        }
      } catch {
        // bindings not ready; fresh conversation
      }

      track(
        await listen<{ state: AssistantState }>("assistant-state", (e) => {
          const next = e.payload.state;
          // A brand-new turn clears any lingering notice from the previous one.
          // Mid-turn transitions must not, or the "no web results" heads-up
          // would vanish before it is seen.
          if (prevStateRef.current === "idle" && next !== "idle") {
            setNotice(null);
          }
          prevStateRef.current = next;
          setState(next);
          if (next !== "idle") {
            setError(null);
            suppressTtsRef.current = false;
          }
          if (next === "idle") setTool(null);
        }),
      );

      track(
        await listen<{ name: string; detail: string; count: number } | null>(
          "assistant-tool",
          (e) => {
            const payload = e.payload;
            setTool(
              payload
                ? {
                    name: payload.name,
                    detail: payload.detail,
                    count: payload.count,
                    startedAt: Date.now(),
                  }
                : null,
            );
          },
        ),
      );

      track(
        await listen<boolean>("assistant-summarizing", (e) => {
          setSummarizing(e.payload);
        }),
      );

      track(
        await listen<{ role: string; content: string; images?: string[] }[]>(
          "assistant-conversation",
          (e) => {
            setHistory(e.payload.map(toDisplay));
            resetStream();
          },
        ),
      );

      track(
        await listen<string>("assistant-token", (e) => {
          streamBufferRef.current += e.payload;
          if (streamRafRef.current === null) {
            streamRafRef.current = window.requestAnimationFrame(() => {
              streamRafRef.current = null;
              const buffered = streamBufferRef.current;
              streamBufferRef.current = "";
              if (buffered) setStream((prev) => prev + buffered);
            });
          }
        }),
      );

      track(
        await listen<{ code: string; detail: string }>(
          "assistant-error",
          (e) => {
            setError({ code: e.payload.code, detail: e.payload.detail });
            resetStream();
          },
        ),
      );

      track(
        await listen<string>("assistant-notice", (e) => {
          setNotice(e.payload);
        }),
      );

      track(
        await listen<string>("assistant-tts", (e) => {
          if (suppressTtsRef.current) return;
          void speakRef.current(e.payload);
        }),
      );

      track(
        await listen<number>("assistant-tts-begin", (e) => {
          if (suppressTtsRef.current) return;
          void beginStreamRef.current(e.payload);
        }),
      );

      track(
        await listen<string>("assistant-tts-chunk", (e) => {
          if (suppressTtsRef.current) return;
          pushTextRef.current(e.payload);
        }),
      );

      track(
        await listen("assistant-tts-end", () => {
          endStreamRef.current();
        }),
      );

      track(
        await listen("assistant-tts-stop", () => {
          suppressTtsRef.current = true;
          // The backend already invalidated the playback epoch; echoing Stop
          // back could invalidate a newer reply.
          localVoiceRef.current?.stop(false);
          setState((s) => (s === "speaking" ? "idle" : s));
        }),
      );

      track(
        await listen<boolean>("assistant-tts-playing", (e) => {
          setTtsPlaying(e.payload);
        }),
      );

      // Which edge of the frame the quick ask grows from. Sent on every show.
      track(
        await listen("assistant-ask-layout", (e) => {
          setLayout(parseLayout(e.payload));
        }),
      );

      // A new quick ask is starting: nothing on screen belongs to it yet.
      track(
        await listen("assistant-quick-ask", () => {
          resetStream();
          setError(null);
          setNotice(null);
          setSelectionCaptured(0);
          setInput("");
          setHovered(false);
          // A voice ask, whatever opened the window before it (a call, the
          // tray): it has no text field to show.
          setUserOpened(false);
          setCallEnded(false);
        }),
      );

      // The recording picked up the user's selection. Reported while they are
      // still speaking, so the pill can show it before the answer arrives.
      track(
        await listen<number>("assistant-selection-captured", (e) => {
          setSelectionCaptured(typeof e.payload === "number" ? e.payload : 0);
        }),
      );

      // The call key starts a call. History's Continue starts one with a saved
      // conversation in it, or loads that conversation into a call already live.
      // A meeting's "Discuss in a call" starts one about that meeting.
      track(
        await listen<MeetingAttachment | null>(
          "assistant-conversation-meeting",
          (e) => setMeeting(e.payload ?? null),
        ),
      );
      void commands
        .assistantConversationMeeting()
        .then((current) => {
          if (!cancelled) setMeeting(current ?? null);
        })
        .catch(() => {});
      track(
        await listen<CallContinuation | null>(
          "assistant-start-conversation",
          (e) => {
            const continuation = e.payload;
            void (async () => {
              const current = voiceRef.current;
              if (!current.open || current.phase === "error") {
                resetCallFormRef.current();
                await current.start();
              }
              if (!continuation) return;
              const live = voiceRef.current;
              let opened = false;
              if (
                continuation.meetingId !== null &&
                continuation.meetingId !== undefined
              ) {
                opened = await live.discussMeeting(continuation.meetingId);
              } else if (
                continuation.id !== null &&
                continuation.id !== undefined
              ) {
                opened =
                  continuation.messageIndex === null ||
                  continuation.messageIndex === undefined
                    ? await live.loadConversation(continuation.id)
                    : await live.branchConversation(
                        continuation.id,
                        continuation.messageIndex,
                      );
              }
              if (opened) expandCallRef.current();
            })();
          },
        ),
      );

      track(
        await listen<boolean>("assistant-panel-shown", (e) => {
          setPanelVisible(true);
          setUserOpened(e.payload === true);
          setCallEnded(false);
        }),
      );

      // Off screen: the quick ask is over, so nothing it showed may be there the
      // next time the window appears.
      track(
        await listen("assistant-panel-hidden", () => {
          setPanelVisible(false);
          setUserOpened(false);
          setCallEnded(false);
          setError(null);
          setNotice(null);
          setInput("");
          setSelectionCaptured(0);
          setHovered(false);
          resetStream();
        }),
      );

      track(
        await listen("assistant-settings-changed", () => {
          void refreshSettings();
        }),
      );
    };

    setup();
    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
      unlisteners.length = 0;
      if (streamRafRef.current !== null) {
        cancelAnimationFrame(streamRafRef.current);
        streamRafRef.current = null;
      }
    };
  }, [refreshSettings, resetStream]);

  // A notice is a heads-up, not a state: it clears itself.
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 6000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  // Surface a local Kokoro failure once per failure, and end the "speaking"
  // phase so the call cannot hang on it.
  useEffect(() => {
    const err = tts.error;
    if (err && !kokoroErrorRef.current) {
      kokoroErrorRef.current = true;
      const code =
        err.reason === "blocked"
          ? "tts_blocked"
          : err.reason === "playback"
            ? "tts_playback"
            : err.reason === "gpu"
              ? kokoroDevice === "gpu"
                ? "tts_gpu_forced"
                : kokoroNativeReady
                  ? "tts_gpu"
                  : "tts_gpu_slow"
              : "tts_local";
      setError({ code, detail: "" });
      setState((s) => (s === "speaking" ? "idle" : s));
    }
    if (!err) kokoroErrorRef.current = false;
  }, [tts.error, kokoroDevice, kokoroNativeReady]);

  // When local voice audio was blocked from auto-playing, the next click in the
  // panel is exactly the user gesture the system was waiting for.
  useEffect(() => {
    if (tts.error?.reason !== "blocked") return;
    const onGesture = () => tts.retry();
    window.addEventListener("pointerdown", onGesture, { once: true });
    return () => window.removeEventListener("pointerdown", onGesture);
  }, [tts.error, tts.retry]);

  const busy = state !== "idle";

  // Tick a seconds counter while a tool runs, so a long wait reads as progress.
  useEffect(() => {
    if (!tool) {
      setToolElapsed(0);
      return;
    }
    const startedAt = tool.startedAt;
    const tick = () =>
      setToolElapsed(Math.max(0, Math.round((Date.now() - startedAt) / 1000)));
    tick();
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, [tool]);

  /** What the call's tool chip reads: the action, then what it's acting on. */
  const toolLabel = useMemo(() => {
    if (!tool) return null;
    const action =
      tool.name === "web_search"
        ? t("assistant.tool.search")
        : tool.name === "capture_screen"
          ? t("assistant.tool.screen")
          : tool.name === "search_meeting"
            ? t("assistant.tool.meetingSearch")
            : tool.name === "read_meeting"
              ? t("assistant.tool.meetingRead")
              : t("assistant.tool.working");
    return tool.detail ? `${action} · ${tool.detail}` : action;
  }, [tool, t]);

  const ttsAudible = ttsPlaying || tts.status === "speaking";

  // The backend parks a spoken turn in "speaking"; we own the end of that phase.
  useEffect(() => {
    if (state !== "speaking") {
      spokeRef.current = false;
      return;
    }
    if (ttsAudible) {
      spokeRef.current = true;
      return;
    }
    if (spokeRef.current) {
      setState("idle");
      return;
    }
    if (tts.status === "loading") return;
    const timer = window.setTimeout(() => setState("idle"), 20000);
    return () => window.clearTimeout(timer);
  }, [state, ttsAudible, tts.status]);

  const dispatchText = useCallback(async (text: string) => {
    sendingRef.current = true;
    try {
      await commands.assistantSendText(text);
    } catch (err) {
      setError({ code: null, detail: String(err) });
    } finally {
      sendingRef.current = false;
    }
  }, []);

  const sendText = useCallback(async () => {
    const text = input.trim();
    if (!text || sendingRef.current || busy) return;
    setInput("");
    setError(null);
    await dispatchText(text);
  }, [input, busy, dispatchText]);

  const stopTurn = useCallback(async () => {
    suppressTtsRef.current = true;
    tts.stop();
    setTtsPlaying(false);
    try {
      await commands.assistantStop();
    } catch {
      // best-effort
    }
  }, [tts]);

  const hidePanel = useCallback(async () => {
    // Stop drawing first, in the same render that drops the call: otherwise
    // the frame between the two is the idle quick ask, typing bar and all.
    setPanelVisible(false);
    // Closing the panel hangs up. The backend ends the session too, but doing it
    // here first releases the microphone on the click.
    if (voice.open) voice.end();
    await commands.hideAssistantPanel();
  }, [voice]);

  // Back out of a quick ask that is still working: stop the recording,
  // transcription or reply, *then* put the surface away. Hiding alone would
  // leave the microphone open behind an invisible window.
  const dismissAsk = useCallback(async () => {
    try {
      await commands.cancelOperation();
    } catch {
      // best-effort
    }
    await hidePanel();
  }, [hidePanel]);

  // Hang up. The window *is* the call, so ending the call takes the window.
  const endCall = useCallback(() => {
    void hidePanel();
  }, [hidePanel]);
  hangUpRef.current = endCall;

  const stopDrag = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
  }, []);

  /** Localized message for a structured error (falls back to the raw detail). */
  const errorPrimary = useCallback(
    (err: AssistantError): string =>
      err.code
        ? t(`assistant.errors.${err.code}`, {
            defaultValue: err.detail || t("assistant.errors.generic"),
          })
        : err.detail || t("assistant.errors.generic"),
    [t],
  );

  /** A notice code as words; an unknown code degrades to itself. */
  const noticeText = useCallback(
    (code: string): string =>
      t(`assistant.notices.${code}`, { defaultValue: code }),
    [t],
  );

  // A voice-engine failure during a call, in the wording the panel would use.
  const voiceFault =
    voice.open && error?.code?.startsWith("tts") ? errorPrimary(error) : null;

  // ---- The quick ask ---------------------------------------------------------
  //
  // One question and its answer, paired by position so a question still being
  // transcribed never appears above the previous answer.
  let lastUserIndex = -1;
  let lastAssistantIndex = -1;
  history.forEach((message, index) => {
    if (message.role === "user") lastUserIndex = index;
    else if (message.role === "assistant") lastAssistantIndex = index;
  });
  const lastUserMessage =
    lastUserIndex >= 0 ? history[lastUserIndex] : undefined;
  const question = lastUserMessage?.content ?? "";
  const finishedAnswer =
    lastAssistantIndex > lastUserIndex
      ? (history[lastAssistantIndex]?.content ?? "")
      : "";
  const phase = quickAskPhase({
    state,
    stream,
    answer: finishedAnswer,
    hasError: !!error,
  });
  const selectionChars =
    lastUserMessage?.selectionChars ?? (question ? 0 : selectionCaptured);
  // The typing bar ("prompt") belongs to a quick ask the user opened to type
  // into (the tray). A voice ask passes through idle too — before "listening"
  // lands, between two pipeline stages, and in the moment before an empty ask
  // puts itself away — and drawing the bar there flashed a text field in the
  // middle of a voice-only flow (and focused it, taking the keyboard). It keeps
  // the waiting pill instead. `phase` itself is unchanged for the logic below.
  const shownPhase: QuickAskPhase =
    phase === "prompt" && !userOpened ? "transcribing" : phase;
  const status = engineSetupActive
    ? engineSetupLabel || t("assistant.engineSetup.short")
    : // No search query appended: the pill says what is happening in a few
      // words, and a query is a sentence the user cannot read at a glance.
      t(workingLabelKey(phase, state, tool));

  // A voice ask that ended with nothing to show — a tap too short to record, or
  // silence — puts itself away instead of leaving an empty prompt on screen.
  const capturedRef = useRef(false);
  useEffect(() => {
    if (voice.open) return;
    if (phase === "listening" || phase === "transcribing") {
      capturedRef.current = true;
      return;
    }
    if (phase !== "prompt") {
      capturedRef.current = false;
      return;
    }
    if (!capturedRef.current || userOpened || !panelVisible) return;
    const timer = window.setTimeout(() => {
      capturedRef.current = false;
      void commands.hideAssistantPanel();
    }, EMPTY_ASK_DISMISS_MS);
    return () => window.clearTimeout(timer);
  }, [phase, voice.open, userOpened, panelVisible]);

  // A call that ended without the window being put away or taken over by a
  // quick ask leaves nothing to show: the window is the call. Put it away
  // rather than leave an invisible frame on screen.
  useEffect(() => {
    if (!callEnded || !panelVisible) return;
    const timer = window.setTimeout(
      () => void commands.hideAssistantPanel(),
      EMPTY_ASK_DISMISS_MS,
    );
    return () => window.clearTimeout(timer);
  }, [callEnded, panelVisible]);

  // A failed voice ask stays long enough to read, then puts itself away unless the
  // pointer is on it: an always-on-top error nobody asked to keep must not sit
  // over the user's work forever. One the user opened from the tray stays.
  useEffect(() => {
    if (phase !== "error" || userOpened || hovered || voice.open) return;
    const timer = window.setTimeout(
      () => void commands.hideAssistantPanel(),
      ERROR_DISMISS_MS,
    );
    return () => window.clearTimeout(timer);
  }, [phase, userOpened, hovered, voice.open]);

  // Escape puts the quick ask away — cancelling whatever is in flight first —
  // whenever the panel has the keyboard. The call owns Escape while it runs.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.repeat || voice.open) return;
      e.preventDefault();
      if (busy) void dismissAsk();
      else void hidePanel();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [voice.open, busy, dismissAsk, hidePanel]);

  const requestKeyboard = useCallback(() => {
    void emit("assistant-ask-keyboard", { want: true });
  }, []);

  /**
   * Tell Rust which part of the window is drawn, so the transparent rest of the
   * fixed frame passes clicks through to the app underneath (see
   * `hitRegion.ts`).
   */
  usePanelHitRegion(panelVisible);
  useSuppressContextMenu();

  const shellClass = `assistant-scope assistant-shell${
    panelVisible && !callEnded ? "" : " native-window-hidden"
  }`;

  // ---- The live call: a floating bar, or the bar under the conversation -----
  if (voice.open) {
    const showTypingDots =
      (state === "thinking" || state === "searching") && stream === "";
    const transcript = (
      <div
        className="assistant-messages"
        ref={listRef}
        onScroll={handleMessagesScroll}
      >
        {history.length === 0 && stream === "" && meeting && (
          <MeetingStarter
            meeting={meeting}
            onAsk={(text) => void voice.sendText(text)}
            disabled={
              voice.phase === "off" ||
              voice.phase === "starting" ||
              voice.phase === "error"
            }
          />
        )}
        {history.length === 0 && stream === "" && !meeting && (
          <div className="assistant-empty">
            <p>
              {activeCharacter?.greeting?.trim()
                ? activeCharacter.greeting
                : t("assistant.empty")}
            </p>
          </div>
        )}
        {history.map((message, i) => (
          <div key={i} className={`assistant-message ${message.role}`}>
            <div className="assistant-message-content">
              {message.role === "assistant" ? (
                <MarkdownMessage
                  content={message.content}
                  components={MD_COMPONENTS}
                />
              ) : (
                message.content
              )}
            </div>
            {message.thumbnails ? (
              <MessageThumbnails
                urls={message.thumbnails}
                hasScreen={message.screenshot}
                screenLabel={t("assistant.screenAttached")}
              />
            ) : (
              <>
                {message.screenshot && (
                  <span className="screen-chip">
                    <Camera size={11} />
                    {t("assistant.screenAttached")}
                  </span>
                )}
                {(message.images ?? 0) > 0 && (
                  <span className="screen-chip">
                    <ImagePlus size={11} />
                    {t("assistant.attach.imageCount", {
                      count: message.images,
                    })}
                  </span>
                )}
              </>
            )}
            {message.files?.map((name) => (
              <span className="screen-chip" key={name}>
                <FileText size={11} />
                {name}
              </span>
            ))}
            {(message.selectionChars ?? 0) > 0 && (
              <span className="screen-chip">
                <TextSelect size={11} />
                {t("assistant.selectionAttached", {
                  count: message.selectionChars,
                })}
              </span>
            )}
            {message.role === "assistant" && (
              <CopyButton
                content={message.content}
                title={t("assistant.copy")}
              />
            )}
            {message.role === "assistant" && (
              <InsertButton
                content={message.content}
                title={t("assistant.insert")}
              />
            )}
          </div>
        ))}
        {notice && (
          <div className="assistant-notice" role="status">
            <Globe size={12} strokeWidth={2} />
            {noticeText(notice)}
          </div>
        )}
        {summarizing && (
          <div className="assistant-notice" role="status">
            <Loader2 size={12} strokeWidth={2} className="as-spin" />
            {t("assistant.summarizing")}
          </div>
        )}
        {engineSetupActive && (
          <div className="assistant-notice" role="status" aria-live="polite">
            <Loader2 size={12} strokeWidth={2} className="as-spin" />
            {engineSetupLabel}
          </div>
        )}
        {stream !== "" && (
          <div className="assistant-message assistant">
            <div className="assistant-message-content">
              <MarkdownMessage content={stream} components={MD_COMPONENTS} />
            </div>
          </div>
        )}
        {toolLabel && (
          <div className="assistant-tool-chip" aria-live="polite">
            <Loader2 className="as-spin" size={12} aria-hidden="true" />
            <span className="assistant-tool-text">{toolLabel}</span>
            {toolElapsed >= 3 && (
              <span className="assistant-tool-elapsed">
                {t("assistant.tool.elapsed", { seconds: toolElapsed })}
              </span>
            )}
          </div>
        )}
        {showTypingDots && (
          <div className="assistant-message assistant typing">
            <span className="typing-dot" />
            <span className="typing-dot" />
            <span className="typing-dot" />
          </div>
        )}
      </div>
    );
    return (
      <div className={shellClass}>
        <CallSurface
          voice={voice}
          form={callForm.form}
          onExpand={callForm.expand}
          onCollapse={callForm.collapse}
          onEnd={endCall}
          name={activeCharacter?.name ?? t("assistant.title")}
          profilePicker={
            <AssistantProfilePicker
              profiles={characters}
              activeId={activeCharacterId}
              onSelect={selectCharacter}
              conversation={voice.open}
            />
          }
          transcript={transcript}
          meeting={meeting}
          resizeHandles={callExpanded ? <ResizeHandles /> : null}
          hasConversation={history.length > 0}
          activity={
            toolLabel ??
            (engineSetupActive
              ? engineSetupLabel || t("assistant.engineSetup.short")
              : null)
          }
          voiceLoading={tts.status === "loading" ? tts.progress : null}
          voiceFault={voiceFault}
          onDismissFault={() => setError(null)}
        />
      </div>
    );
  }

  return (
    <div className={shellClass}>
      <div
        className="qa-frame"
        data-align={layout.align}
        data-justify={layout.justify}
        onPointerEnter={() => setHovered(true)}
        onPointerLeave={() => setHovered(false)}
      >
        <QuickAsk
          phase={shownPhase}
          status={status}
          levels={listening ? micLevels : undefined}
          question={question}
          answer={stream || finishedAnswer}
          markdown={MD_COMPONENTS}
          selectionChars={selectionChars}
          // The model looked, or is looking: the tool is running, or the
          // question already carries the screenshot marker it left behind.
          screen={
            tool?.name === "capture_screen" || !!lastUserMessage?.screenshot
          }
          error={error ? errorPrimary(error) : null}
          notice={notice ? noticeText(notice) : null}
          canRetry={question.trim().length > 0 && !busy}
          input={input}
          onInputChange={(value) => {
            setInput(value);
            if (error) setError(null);
          }}
          onSubmit={() => void sendText()}
          autoFocus
          onRequestKeyboard={requestKeyboard}
          onClose={() => void hidePanel()}
          onCancel={() => void dismissAsk()}
          onStop={() => void stopTurn()}
          onRetry={() => {
            setError(null);
            void commands.assistantRegenerate();
          }}
          onInsert={async (text) => {
            if (!text.trim()) return false;
            // The window is hidden at once for the paste (no time for the
            // usual clear-then-hide), so stop drawing first and let that empty
            // frame reach the screen: the next show must not reveal this card
            // as a stale frame.
            setPanelVisible(false);
            await new Promise((done) => window.setTimeout(done, 50));
            const result = await commands.assistantInsertText(text);
            return result.status === "ok";
          }}
          stopDrag={stopDrag}
        />
      </div>
    </div>
  );
};

export default AssistantPanel;
