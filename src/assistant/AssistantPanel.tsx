import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  AlertCircle,
  ArrowUp,
  Camera,
  CameraOff,
  ChevronDown,
  Check,
  Copy,
  Eraser,
  FileText,
  Globe,
  ImagePlus,
  Loader2,
  Lock,
  Maximize2,
  Mic,
  Minimize2,
  Paperclip,
  RotateCcw,
  Scissors,
  Sparkles,
  Square,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import {
  commands,
  type AppSettings,
  type AssistantCharacter,
} from "@/bindings";
import { syncLanguageFromSettings } from "@/i18n";
import { AudioWaveform } from "@/components/shared";
import { FONT_SIZES, errorKind, type AssistantError } from "./appearance";
import { useKokoroTts } from "./useKokoroTts";
import "./AssistantPanel.css";

type AssistantState =
  | "idle"
  | "listening"
  | "transcribing"
  | "searching"
  | "thinking"
  | "speaking";

interface DisplayMessage {
  role: "user" | "assistant";
  content: string;
  screenshot?: boolean;
  images?: number;
  files?: string[];
  /** Display thumbnails (data URLs) for the visuals sent with this message —
   *  the screen capture first (if any), then attached images. Present on new
   *  messages; empty on older history (which falls back to the text chips). */
  thumbnails?: string[];
}

/** Must match the marker constants in src-tauri/src/assistant.rs */
const SCREENSHOT_MARKER = "[screenshot attached]";
const IMAGE_MARKER = "[image attached]";
const FILE_MARKER_PREFIX = "[file attached:";

/** A picture waiting to be sent with the next message. */
interface PendingImage {
  id: string;
  dataUrl: string;
}

/** A text-like file waiting to be sent as context with the next message. */
interface PendingFile {
  id: string;
  name: string;
  content: string;
}

const MAX_PENDING_IMAGES = 4;
const MAX_PENDING_FILES = 4;

let attachmentSeq = 0;
const nextAttachmentId = (): string => `att-${++attachmentSeq}`;

const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/** Small round avatar for a character: the uploaded image, a cat emoji for the
 *  Cat, or the name's first initial. */
const CharacterAvatar: React.FC<{
  character: AssistantCharacter | null;
  size: number;
}> = ({ character, size }) => {
  const dims = { width: size, height: size };
  if (character?.avatar) {
    return (
      <img
        className="assistant-character-avatar"
        src={character.avatar}
        alt=""
        style={dims}
      />
    );
  }
  const fallback =
    character?.kind === "cat"
      ? "🐱"
      : (character?.name.trim()[0] ?? "?").toUpperCase();
  return (
    <span
      className="assistant-character-avatar"
      style={{ ...dims, fontSize: Math.round(size * 0.5) }}
      aria-hidden
    >
      {fallback}
    </span>
  );
};

/** Downscale a pasted image blob to a provider-friendly JPEG data URL (same
 *  1568px budget the backend uses for files picked from disk). */
async function downscaleToDataUrl(blob: Blob): Promise<string> {
  const bitmap = await createImageBitmap(blob);
  const maxDim = 1568;
  const scale = Math.min(1, maxDim / Math.max(bitmap.width, bitmap.height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(bitmap.width * scale));
  canvas.height = Math.max(1, Math.round(bitmap.height * scale));
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("canvas 2d context unavailable");
  ctx.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  return canvas.toDataURL("image/jpeg", 0.8);
}

/** How long a transient error / notice lingers on the pill before self-clearing. */
const TRANSIENT_MS = 5000;

/** How long a blocking error (needs a settings/permission fix) lingers before
 *  self-clearing. Longer than a transient hiccup so there's time to read the
 *  fix, but it STILL clears — an always-on-top pill must never get permanently
 *  stuck showing a stale error. */
const BLOCKING_MS = 7000;

/** How long the collapsed pill sits idle (at rest, no hover) before it dims to
 *  a quiet, thin sliver so it stays out of the user's way. Any activity or a
 *  hover brings it straight back. */
const PILL_IDLE_DIM_MS = 6000;

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
  for (const line of raw.content.split("\n")) {
    const trimmed = line.trim();
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
    content: kept.join("\n").trim(),
    screenshot: screenshot || undefined,
    images: images || undefined,
    files: files.length ? files : undefined,
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

/** `<pre>` renderer with a hover copy button, so each code block is
 *  individually copyable (the whole-answer copy stays too). */
const CodeBlock: React.FC<React.HTMLAttributes<HTMLPreElement>> = ({  children,
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

/** Small inline preview thumbnails of the image(s) sent with a message (screen
 *  capture and/or attached pictures). Clicking a thumbnail pops a larger
 *  preview so you can tell exactly what the assistant was sent — the
 *  full-resolution frame goes to the model, these compact copies are stored
 *  with the message and survive restarts. The first thumbnail carries a small
 *  camera badge when it's a screen capture. Click again (or the backdrop, or
 *  Esc) to dismiss. */
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
      // Clicking the open thumbnail again closes it.
      if (cur && cur.url === url) return null;
      const rect = el.getBoundingClientRect();
      const maxW = Math.min(320, window.innerWidth - 24);
      let left = rect.left + rect.width / 2 - maxW / 2;
      left = Math.max(12, Math.min(left, window.innerWidth - maxW - 12));
      // Prefer showing the enlarged preview above the thumbnail; flip below when
      // there isn't enough headroom (e.g. a message near the top of the panel).
      const above = rect.top > 220;
      const top = above ? rect.top - 8 : rect.bottom + 8;
      return { url, left, top, above };
    });
  };

  // Dismiss the popped preview on Escape.
  useEffect(() => {
    if (!preview) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPreview(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
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

/** Invisible edge/corner grips that drive Tauri's native window resize. The
 *  panel window is borderless (no OS resize border — most noticeably on
 *  Windows), so these provide reliable, easy resizing. Only rendered on the
 *  expanded panel; the pill isn't resizable. */
// Mirrors Tauri's (non-exported) ResizeDirection string union so we can type
// the handles without importing an internal type.
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
  const onDown = (e: React.MouseEvent, dir: ResizeDir) => {
    // Primary button only; don't let the grip start a header drag or a text
    // selection while the native resize loop runs.
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    void getCurrentWindow().startResizeDragging(dir);
  };
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
  // Conversation snapshots from the backend are the single source of truth;
  // `stream` only holds the in-flight answer between snapshots. This makes
  // rendering idempotent: duplicate events can never duplicate messages.
  const [history, setHistory] = useState<DisplayMessage[]>([]);
  const [stream, setStream] = useState("");
  const [error, setError] = useState<AssistantError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [state, setState] = useState<AssistantState>("idle");
  const [input, setInput] = useState("");
  const [attachScreen, setAttachScreen] = useState(false);
  const [collapsed, setCollapsed] = useState(true);
  const [locked, setLocked] = useState(false);
  // The collapsed pill dims to a thin, translucent sliver after a spell of
  // inactivity so it doesn't sit in the user's way; hovering it (CSS) or any
  // activity restores it.
  const [dimmed, setDimmed] = useState(false);
  const [ttsPlaying, setTtsPlaying] = useState(false);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [micLevels, setMicLevels] = useState<number[]>([]);
  const [visionActive, setVisionActive] = useState(false);
  const [characterMenuOpen, setCharacterMenuOpen] = useState(false);
  const characterMenuRef = useRef<HTMLDivElement>(null);
  const [mounted, setMounted] = useState(false);
  const [pendingImages, setPendingImages] = useState<PendingImage[]>([]);
  const [pendingFiles, setPendingFiles] = useState<PendingFile[]>([]);
  const [dropActive, setDropActive] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  const sendingRef = useRef(false);
  // Remembers the last pipeline state so we can tell a brand-new turn (idle →
  // active) from a mid-turn transition — used to clear a stale notice only at
  // the start of the next turn, never mid-turn.
  const prevStateRef = useRef<AssistantState>("idle");
  // Tracks whether audio actually began during the current "speaking" phase,
  // so we can detect when playback *ends* and hand the UI back to idle.
  const spokeRef = useRef(false);
  // Set when the user presses Stop; blocks a TTS event that was emitted just
  // before the Stop from slipping through (events can arrive slightly out of
  // order). Cleared as soon as any new turn becomes active, so it never blocks
  // a legitimate next answer.
  const suppressTtsRef = useRef(false);
  // Only surface a local-Kokoro load failure once per failure.
  const kokoroErrorRef = useRef(false);

  const ttsEnabled = settings?.assistant_tts_enabled ?? false;
  const ttsVoice = settings?.assistant_tts_voice ?? "af_heart";
  const ttsDtype = settings?.assistant_tts_kokoro_dtype ?? "fp32";
  const ttsSpeed = settings?.assistant_tts_speed ?? 1;
  const screenshotEnabled = settings?.assistant_screenshot_enabled ?? true;
  const webSearchEnabled = settings?.assistant_web_search_enabled ?? false;
  const characters = settings?.assistant_characters ?? [];
  const activeCharacterId =
    settings?.assistant_active_character_id ?? "default";
  const activeCharacter =
    characters.find((c) => c.id === activeCharacterId) ?? characters[0] ?? null;
  const tts = useKokoroTts(ttsEnabled, ttsVoice, ttsDtype, ttsSpeed);
  const speakRef = useRef(tts.speak);
  speakRef.current = tts.speak;

  useEffect(() => setMounted(true), []);

  const refreshSettings = useCallback(async () => {
    try {
      const result = await commands.getAppSettings();
      if (result.status === "ok") {
        setSettings(result.data);
      }
    } catch {
      // bindings not ready yet
    }
  }, []);

  const selectCharacter = useCallback(
    async (id: string) => {
      setCharacterMenuOpen(false);
      try {
        await commands.setAssistantActiveCharacter(id);
        await refreshSettings();
      } catch {
        // best-effort — the picker just won't change on failure
      }
    },
    [refreshSettings],
  );

  // Close the character switcher when clicking anywhere outside it.
  useEffect(() => {
    if (!characterMenuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (!characterMenuRef.current?.contains(e.target as Node)) {
        setCharacterMenuOpen(false);
      }
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [characterMenuOpen]);

  // Apply text size + surface opacity. The panel is dark-only (like the STT
  // overlay), so there is no theme resolution anymore.
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

  // Auto-scroll to bottom on new content
  useEffect(() => {
    const el = listRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }, [history, stream, state, error, notice]);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: (() => void)[] = [];

    /** Register a listener, immediately disposing it if the effect was
     *  already cleaned up (StrictMode double-mount protection). */
    const track = (unlisten: () => void) => {
      if (cancelled) {
        unlisten();
      } else {
        unlisteners.push(unlisten);
      }
    };

    const setup = async () => {
      await syncLanguageFromSettings();
      await refreshSettings();

      // Sync the pill/expanded state from the backend so a freshly (re)loaded
      // panel renders the right layout. Without this the webview defaults to
      // "expanded" and can show the full panel header inside the pill window.
      try {
        const isCollapsed = await commands.getAssistantPanelCollapsed();
        if (!cancelled) setCollapsed(isCollapsed);
      } catch {
        // bindings not ready yet; keep current state
      }

      // Restore conversation (panel window can be recreated mid-conversation)
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
          // A brand-new turn (idle → active) clears any lingering notice from
          // the previous turn. Mid-turn transitions (e.g. searching → thinking)
          // must NOT clear it, or the "no web results" heads-up would vanish
          // before it's seen. Otherwise the transient timer clears it.
          if (prevStateRef.current === "idle" && next !== "idle") {
            setNotice(null);
          }
          prevStateRef.current = next;
          setState(next);
          if (next !== "listening") {
            setLocked(false);
          }
          if (next !== "idle") {
            setError(null);
            // A new turn is active — allow its eventual spoken reply through,
            // clearing any suppression left by a previous Stop.
            suppressTtsRef.current = false;
          }
          // The turn finished — drop the per-turn indicators. The notice is
          // intentionally kept so it can surface on the collapsed pill after
          // the turn ends (it self-clears via the transient timer).
          if (next === "idle") {
            setVisionActive(false);
          }
        }),
      );

      track(
        await listen<boolean>("recording-locked", (e) => {
          setLocked(e.payload);
        }),
      );

      // Live microphone levels (broadcast to all windows during recording)
      // drive the waveform shown while the assistant is listening.
      track(
        await listen<number[]>("mic-level", (e) => {
          setMicLevels(e.payload);
        }),
      );

      track(
        await listen<boolean>("assistant-screen-armed", (e) => {
          setAttachScreen(e.payload);
        }),
      );

      // Whether the in-flight turn is a screen-vision turn (e.g. the
      // "Assistant + Screen" shortcut), so the panel/pill can show it.
      track(
        await listen<boolean>("assistant-vision-active", (e) => {
          setVisionActive(e.payload);
        }),
      );

      track(
        await listen<{ role: string; content: string; images?: string[] }[]>(
          "assistant-conversation",
          (e) => {
            setHistory(e.payload.map(toDisplay));
            setStream("");
          },
        ),
      );

      track(
        await listen<string>("assistant-token", (e) => {
          setStream((prev) => prev + e.payload);
        }),
      );

      track(
        await listen<{ code: string; detail: string }>(
          "assistant-error",
          (e) => {
            setError({ code: e.payload.code, detail: e.payload.detail });
            setStream("");
          },
        ),
      );

      // Non-blocking notices (the turn keeps going), e.g. web search failed.
      track(
        await listen<string>("assistant-notice", (e) => {
          setNotice(e.payload);
        }),
      );

      track(
        await listen<string>("assistant-tts", (e) => {
          // Ignore a reply that was emitted just before a Stop.
          if (suppressTtsRef.current) return;
          void speakRef.current(e.payload);
        }),
      );

      track(
        await listen("assistant-tts-stop", () => {
          suppressTtsRef.current = true;
          tts.stop();
          // Stopping during the spoken-reply phase ends the turn.
          setState((s) => (s === "speaking" ? "idle" : s));
        }),
      );

      track(
        await listen<boolean>("assistant-tts-playing", (e) => {
          setTtsPlaying(e.payload);
        }),
      );

      track(
        await listen<boolean>("assistant-collapsed", (e) => {
          setCollapsed(e.payload);
        }),
      );

      track(
        await listen("assistant-settings-changed", () => {
          void refreshSettings();
        }),
      );

      // A snipped screen region arrives as a ready-to-send image attachment.
      track(
        await listen<string>("assistant-region-captured", (e) => {
          setPendingImages((prev) =>
            prev.length >= MAX_PENDING_IMAGES
              ? prev
              : [...prev, { id: nextAttachmentId(), dataUrl: e.payload }],
          );
        }),
      );

      // A voice turn (pill mic / hotkey) consumed the staged attachments.
      track(
        await listen("assistant-attachments-consumed", () => {
          setPendingImages([]);
          setPendingFiles([]);
        }),
      );
    };

    setup();
    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
      unlisteners.length = 0;
    };
  }, [refreshSettings]);

  // Every error self-clears so the always-on-top pill can never get stuck
  // showing a stale error forever (the bug where e.g. "Model can't see images"
  // lingered indefinitely). Blocking errors — which carry a fix — linger a bit
  // longer than transient hiccups; both can still be dismissed early with the ×
  // or resolved in settings.
  useEffect(() => {
    if (!error) return;
    const ms = errorKind(error) === "blocking" ? BLOCKING_MS : TRANSIENT_MS;
    const timer = window.setTimeout(() => setError(null), ms);
    return () => window.clearTimeout(timer);
  }, [error]);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), TRANSIENT_MS);
    return () => window.clearTimeout(timer);
  }, [notice]);

  // Surface a local Kokoro failure (§4: TTS errors are not silent), mapping the
  // hook's reason to a precise, actionable message. Only once per failure — the
  // hook keeps the error set until it's retried or cleared. A failure also ends
  // the "speaking" phase so the pill can't hang.
  useEffect(() => {
    const err = tts.error;
    if (err && !kokoroErrorRef.current) {
      kokoroErrorRef.current = true;
      const code =
        err.reason === "blocked"
          ? "tts_blocked"
          : err.reason === "playback"
            ? "tts_playback"
            : "tts_local"; // load / synthesis
      setError({ code, detail: "" });
      setState((s) => (s === "speaking" ? "idle" : s));
    }
    if (!err) {
      kokoroErrorRef.current = false;
    }
  }, [tts.error]);

  // When local voice audio was blocked from auto-playing, the very next click in
  // the panel is exactly the user gesture the system was waiting for — replay
  // the held clip on it, so "tap the panel to hear it" just works.
  useEffect(() => {
    if (tts.error?.reason !== "blocked") return;
    const onGesture = () => tts.retry();
    window.addEventListener("pointerdown", onGesture, { once: true });
    return () => window.removeEventListener("pointerdown", onGesture);
  }, [tts.error, tts.retry]);

  const busy = state !== "idle";
  const isListening = state === "listening";

  /** Route a dropped/picked path to the right reader by extension. */
  const addPath = useCallback(async (path: string) => {
    const ext = path.split(".").pop()?.toLowerCase() ?? "";
    try {
      if (IMAGE_EXTENSIONS.includes(ext)) {
        const result = await commands.assistantReadImage(path);
        if (result.status === "ok") {
          setPendingImages((prev) =>
            prev.length >= MAX_PENDING_IMAGES
              ? prev
              : [...prev, { id: nextAttachmentId(), dataUrl: result.data }],
          );
        } else {
          setError({ code: "file_read", detail: result.error });
        }
      } else {
        const result = await commands.assistantReadFile(path);
        if (result.status === "ok") {
          const { name, content } = result.data;
          setPendingFiles((prev) =>
            prev.length >= MAX_PENDING_FILES
              ? prev
              : [...prev, { id: nextAttachmentId(), name, content }],
          );
        } else {
          setError({ code: "file_read", detail: result.error });
        }
      }
    } catch (err) {
      setError({ code: "file_read", detail: String(err) });
    }
  }, []);

  // Mirror the staged chips into the backend so VOICE turns (pill mic or
  // hotkey — they run entirely in Rust) send the attachments too.
  useEffect(() => {
    void commands
      .assistantSetPendingAttachments(
        pendingImages.map((image) => image.dataUrl),
        pendingFiles.map(({ name, content }) => ({ name, content })),
      )
      .catch(() => {
        // bindings not ready — the next change re-syncs
      });
  }, [pendingImages, pendingFiles]);

  // Paste an image (screenshot in the clipboard etc.) anywhere in the panel.
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const items = e.clipboardData?.items;
      if (!items) return;
      for (const item of items) {
        if (item.type.startsWith("image/")) {
          const blob = item.getAsFile();
          if (blob) {
            e.preventDefault();
            void downscaleToDataUrl(blob)
              .then((dataUrl) =>
                setPendingImages((prev) =>
                  prev.length >= MAX_PENDING_IMAGES
                    ? prev
                    : [...prev, { id: nextAttachmentId(), dataUrl }],
                ),
              )
              .catch((err) =>
                setError({ code: "file_read", detail: String(err) }),
              );
          }
          return;
        }
      }
    };
    document.addEventListener("paste", onPaste);
    return () => document.removeEventListener("paste", onPaste);
  }, []);

  // Drag & drop files onto the panel (Tauri surfaces native drops as events).
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "enter" || event.payload.type === "over") {
          setDropActive(true);
        } else if (event.payload.type === "drop") {
          setDropActive(false);
          for (const path of event.payload.paths) {
            void addPath(path);
          }
        } else {
          setDropActive(false);
        }
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [addPath]);

  // File picker (paperclip button).
  const pickFiles = useCallback(async () => {
    try {
      const picked = await openFileDialog({ multiple: true });
      if (!picked) return;
      const paths = Array.isArray(picked) ? picked : [picked];
      for (const path of paths) {
        await addPath(path);
      }
    } catch (err) {
      setError({ code: "file_read", detail: String(err) });
    }
  }, [addPath]);

  const beginSnip = useCallback(async () => {
    try {
      await commands.assistantBeginRegionSnip();
    } catch (err) {
      setError({ code: "screen_capture", detail: String(err) });
    }
  }, []);

  // Audio is actually coming out of the speakers right now vs. the voice
  // still being prepared (model loading / synthesis / fetch). The distinction
  // drives the pill: preparing shows a spinner (feedback, not dead air),
  // audible shows the speaker + flowing wave.
  const ttsAudible = ttsPlaying || tts.status === "speaking";
  const ttsActive = ttsAudible || tts.status === "loading";
  const showStop = busy || ttsActive;

  // The backend parks the turn in "speaking" when a spoken reply is starting.
  // We own the end of that phase: once playback has started and then stopped
  // (audio fell after having risen), return to idle. While the voice model is
  // still loading there is no timeout (the spinner shows honest progress);
  // otherwise a generous safety timeout prevents a stuck "speaking" pill if
  // audio never materialises.
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
    if (tts.status === "loading") return; // legit long prep — spinner shows
    const timer = window.setTimeout(() => setState("idle"), 20000);
    return () => window.clearTimeout(timer);
  }, [state, ttsAudible, tts.status]);

  // Idle-dim the collapsed pill: after a spell at rest (idle, no error/notice,
  // no voice playing) fade and thin it to a quiet sliver so it stays out of the
  // way. Any activity flips it back here; a hover restores it via CSS. Only the
  // pill dims — the expanded panel never does.
  useEffect(() => {
    if (!collapsed) {
      setDimmed(false);
      return;
    }
    const atRest = state === "idle" && !error && !notice && !ttsActive;
    if (!atRest) {
      setDimmed(false);
      return;
    }
    const timer = window.setTimeout(() => setDimmed(true), PILL_IDLE_DIM_MS);
    return () => window.clearTimeout(timer);
  }, [collapsed, state, error, notice, ttsActive]);

  const sendText = useCallback(async () => {
    const text = input.trim();
    if (!text || sendingRef.current || busy) return;
    sendingRef.current = true;
    setInput("");
    // The one slash command: /summarize compacts the conversation.
    if (text.toLowerCase() === "/summarize") {
      try {
        await commands.assistantSummarize();
      } catch (err) {
        setError({ code: null, detail: String(err) });
      } finally {
        sendingRef.current = false;
      }
      return;
    }
    const withScreen = attachScreen && screenshotEnabled;
    const images = pendingImages.map((image) => image.dataUrl);
    const files = pendingFiles.map(({ name, content }) => ({ name, content }));
    try {
      if (images.length > 0 || files.length > 0 || withScreen) {
        // Screen vision is sticky: it stays armed for the following turns
        // until the user switches it off (camera toggle or pill badge).
        await commands.assistantSendComposed(text, images, files, withScreen);
        setPendingImages([]);
        setPendingFiles([]);
      } else {
        await commands.assistantSendText(text);
      }
    } catch (err) {
      setError({ code: null, detail: String(err) });
    } finally {
      sendingRef.current = false;
    }
  }, [
    input,
    busy,
    attachScreen,
    screenshotEnabled,
    pendingImages,
    pendingFiles,
  ]);

  const clearConversation = useCallback(async () => {
    tts.stop();
    setError(null);
    setNotice(null);
    setStream("");
    await commands.assistantClearConversation();
  }, [tts]);

  const stopTurn = useCallback(async () => {
    // Block any reply that was just emitted, then stop local + remote TTS.
    suppressTtsRef.current = true;
    tts.stop();
    setTtsPlaying(false);
    try {
      await commands.assistantStop();
    } catch {
      // ignore — stop is best-effort
    }
  }, [tts]);

  // Cancel an in-flight voice capture (recording/transcribing) without
  // sending it — the pill's hover-reveal ×, like the STT overlay.
  const cancelVoice = useCallback(async () => {
    try {
      await commands.cancelOperation();
    } catch {
      // best-effort
    }
  }, []);

  const hidePanel = useCallback(async () => {
    await commands.hideAssistantPanel();
  }, []);

  const toggleTts = useCallback(async () => {
    if (ttsEnabled) {
      tts.stop();
    }
    await commands.setAssistantTtsEnabled(!ttsEnabled);
    await refreshSettings();
  }, [ttsEnabled, tts, refreshSettings]);

  const toggleVoice = useCallback(async () => {
    setError(null);
    await commands.assistantToggleVoice();
  }, []);

  // Finish a hands-free (tap-to-lock or toggle) voice capture and send it —
  // the keyboard-free equivalent of pressing the hotkey again. Stops the
  // recording and runs the assistant turn on it.
  const finishVoice = useCallback(async () => {
    await commands.commitRecording();
  }, []);

  // Keep a click on a pill button from being swallowed by the window drag
  // region: the whole pill is draggable (data-tauri-drag-region), so a plain
  // mousedown on a button starts a native window-move and the button's click
  // never lands. Stopping propagation on mousedown lets onClick fire reliably.
  const stopDrag = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
  }, []);

  const toggleWebSearch = useCallback(async () => {
    await commands.setAssistantWebSearchEnabled(!webSearchEnabled);
    await refreshSettings();
  }, [webSearchEnabled, refreshSettings]);

  const collapse = useCallback(async (value: boolean) => {
    await commands.setAssistantPanelCollapsed(value);
    setCollapsed(value);
  }, []);

  // Toggle screen-vision straight from the collapsed pill's hover controls, so
  // it can be armed AND disarmed without opening the panel — one control, shown
  // only on hover (no separate always-on badge). Derived from the raw state,
  // not `screenActive`, which is declared later in render.
  const toggleScreen = useCallback(async () => {
    const next = !(visionActive || attachScreen);
    setAttachScreen(next);
    await commands.setAssistantScreenArmed(next);
  }, [visionActive, attachScreen]);

  /** Localized primary message for a structured error (falls back to the raw
   *  backend detail for unknown codes / webview-side failures). */
  const errorPrimary = useCallback(
    (err: AssistantError): string =>
      err.code
        ? t(`assistant.errors.${err.code}`, {
            defaultValue: err.detail || t("assistant.errors.generic"),
          })
        : err.detail || t("assistant.errors.generic"),
    [t],
  );

  /** Pill-sized variant of the same message. */
  const errorShort = useCallback(
    (err: AssistantError): string =>
      err.code
        ? t(`assistant.errors.${err.code}Short`, {
            defaultValue: errorPrimary(err),
          })
        : errorPrimary(err),
    [t, errorPrimary],
  );

  const showTypingDots =
    (state === "thinking" || state === "searching") && stream === "";

  // Screen-vision is active either because the user armed the camera, or
  // because this turn came from the "Assistant + Screen" shortcut.
  const screenActive = visionActive || attachScreen;

  const ttsTitle =
    tts.status === "loading"
      ? t("assistant.tts.loadingShort", { progress: tts.progress })
      : ttsEnabled
        ? t("assistant.tts.disable")
        : t("assistant.tts.enable");

  const shellClass = `assistant-scope assistant-shell${
    collapsed ? "" : " expanded"
  }${mounted ? " fade-in" : ""}`;

  if (collapsed) {
    // ---- The voice pill: state carried by the waveform -------------------
    const isSearchingPhase = state === "searching";
    // Voice reply actually audible vs. still being prepared (model load /
    // synthesis / fetch). Preparing shows a spinner — honest feedback instead
    // of a silent "speaking" wave.
    const isVoicePreparing =
      tts.status === "loading" || (state === "speaking" && !ttsAudible);
    const isWorkingPhase =
      state === "thinking" || state === "transcribing" || isVoicePreparing;
    const showError = !!error && !busy;
    const pillBusy = showStop && !showError;
    // A quiet post-turn heads-up (e.g. web search found nothing), shown only at
    // rest so it never competes with the working waveform. Self-clears.
    const showNotice = !!notice && !showStop && !showError;

    const pillStatus = showError
      ? errorShort(error)
      : tts.status === "loading"
        ? t("assistant.tts.loadingShort", { progress: tts.progress })
        : busy
          ? t(`assistant.status.${state}`)
          : ttsActive
            ? t("assistant.status.speaking")
            : t("assistant.pill.idle");

    const waveMode: "reactive" | "shimmer" | "flow" = isListening
      ? "reactive"
      : ttsAudible
        ? "flow"
        : "shimmer";

    return (
      <div className={shellClass} data-tauri-drag-region>
        <div
          className={`apill${isListening ? " listening" : ""}${
            showError ? " error" : ""
          }${dimmed ? " dimmed" : ""}`}
          data-tauri-drag-region
          role="status"
          aria-label={pillStatus}
          title={showError ? error.detail || undefined : undefined}
        >
          {showError ? (
            <>
              <AlertCircle size={14} className="apill-error-icon" />
              <span className="apill-error-text" data-tauri-drag-region>
                {errorShort(error)}
              </span>
              <button
                className="apill-cancel"
                onClick={() => setError(null)}
                onMouseDown={stopDrag}
                title={t("assistant.pill.dismiss")}
                aria-label={t("assistant.pill.dismiss")}
              >
                <X size={13} strokeWidth={2.5} />
              </button>
            </>
          ) : isListening && locked ? (
            // Hands-free lock: inline Cancel · wave · Done, like the STT
            // overlay's locked layout. The ✓ finishes and sends. This pill is
            // told apart from the STT overlay's locked pill by its soft accent
            // ring (.apill.listening) + rounded-rect shape — a quiet cue rather
            // than an extra leading glyph that would lengthen the chip.
            // onMouseDown stops the drag-region from swallowing the click (the
            // whole pill is a drag surface), so ✗/✓ reliably register.
            <>
              <button
                className="apill-btn danger"
                onClick={cancelVoice}
                onMouseDown={(e) => e.stopPropagation()}
                title={t("assistant.pill.cancel")}
                aria-label={t("assistant.pill.cancel")}
              >
                <X size={13} strokeWidth={2.5} />
              </button>
              <div className="apill-wave" data-tauri-drag-region>
                <AudioWaveform
                  levels={micLevels}
                  size="sm"
                  barCount={12}
                  mode="reactive"
                  active
                />
              </div>
              <button
                className="apill-btn apill-done"
                onClick={finishVoice}
                onMouseDown={(e) => e.stopPropagation()}
                title={t("assistant.finish")}
                aria-label={t("assistant.finish")}
              >
                <Check size={13} strokeWidth={2.75} />
              </button>
            </>
          ) : pillBusy || isListening ? (
            // Busy phases: one living waveform, a small side glyph for the
            // phases worth calling out. Hovering reveals expand + cancel — the
            // user is never locked out of the full panel while it works.
            <>
              {/* Assistant identity anchor: a sparkle leads the pill whenever a
                  phase-specific glyph (search / speaking) isn't showing, so the
                  listening / thinking states never collapse to "just a
                  waveform" — which is what made this chip indistinguishable
                  from the STT recording overlay. The accent tint lives here
                  (see .apill-glyph.identity) rather than being smeared across
                  the whole chip. */}
              {!isSearchingPhase && !ttsAudible && (
                <span
                  className="apill-glyph identity"
                  data-tauri-drag-region
                  aria-hidden="true"
                >
                  <Sparkles size={13} strokeWidth={2} />
                </span>
              )}
              {isSearchingPhase && (
                <span className="apill-glyph" data-tauri-drag-region>
                  <Globe size={13} strokeWidth={2} />
                </span>
              )}
              {ttsAudible && (
                <span className="apill-glyph" data-tauri-drag-region>
                  <Volume2 size={13} strokeWidth={2} />
                </span>
              )}
              <div className="apill-wave" data-tauri-drag-region>
                <AudioWaveform
                  levels={isListening ? micLevels : []}
                  size="sm"
                  barCount={12}
                  mode={waveMode}
                  active={isListening}
                />
              </div>
              {isWorkingPhase && (
                <Loader2 size={12} strokeWidth={2.5} className="apill-spin" />
              )}
              {/* Stop: a clear, always-visible square while the assistant is
                  thinking, searching, or speaking, so the turn can be stopped
                  without hunting for a hover-only control. During voice capture
                  (listening/transcribing) the hover ✗ cancels instead. */}
              {showStop && !isListening && state !== "transcribing" && (
                <button
                  className="apill-btn danger apill-stop"
                  onClick={stopTurn}
                  onMouseDown={stopDrag}
                  title={t("assistant.stop")}
                  aria-label={t("assistant.stop")}
                >
                  <Square size={11} strokeWidth={2.75} />
                </button>
              )}
              <div className="apill-reveal quick">
                <button
                  className="apill-btn"
                  onClick={() => collapse(false)}
                  onMouseDown={stopDrag}
                  title={t("assistant.pill.expand")}
                  aria-label={t("assistant.pill.expand")}
                >
                  <Maximize2 size={11} strokeWidth={2.25} />
                </button>
                {(isListening || state === "transcribing") && (
                  <button
                    className="apill-btn danger"
                    onClick={cancelVoice}
                    onMouseDown={stopDrag}
                    title={t("assistant.pill.cancelTurn")}
                    aria-label={t("assistant.pill.cancelTurn")}
                  >
                    <X size={12} strokeWidth={2.5} />
                  </button>
                )}
              </div>
            </>
          ) : (
            // Idle: a quiet mic + resting wave; hovering reveals expand/close.
            <>
              <button
                className="apill-btn apill-mic"
                onClick={toggleVoice}
                onMouseDown={stopDrag}
                title={t("assistant.pill.talk")}
                aria-label={t("assistant.pill.talk")}
              >
                <Mic size={13} strokeWidth={2.25} />
              </button>
              {showNotice ? (
                <span
                  className="apill-notice"
                  role="status"
                  data-tauri-drag-region
                  title={t(`assistant.notices.${notice}`, {
                    defaultValue: t("assistant.notices.web_search_failed"),
                  })}
                >
                  <Globe
                    size={11}
                    strokeWidth={2}
                    className="apill-notice-icon"
                  />
                  <span className="apill-notice-text">
                    {t("assistant.notices.web_search_failedShort", {
                      defaultValue: t("assistant.notices.web_search_failed"),
                    })}
                  </span>
                </span>
              ) : (
                <div className="apill-wave rest" data-tauri-drag-region>
                  <AudioWaveform
                    levels={[]}
                    size="sm"
                    barCount={8}
                    mode="reactive"
                    active={false}
                  />
                </div>
              )}
              <div className="apill-reveal">
                <button
                  className="apill-btn"
                  onClick={() => collapse(false)}
                  onMouseDown={stopDrag}
                  title={t("assistant.pill.expand")}
                  aria-label={t("assistant.pill.expand")}
                >
                  <Maximize2 size={12} strokeWidth={2.25} />
                </button>
                <button
                  className="apill-btn danger"
                  onClick={hidePanel}
                  onMouseDown={stopDrag}
                  title={t("assistant.pill.close")}
                  aria-label={t("assistant.pill.close")}
                >
                  <X size={13} strokeWidth={2.5} />
                </button>
              </div>
            </>
          )}
          {/* Screen-vision toggle: a small top-right badge. When armed it stays
              visible in every state (so you always know capture is on); when off
              it's hidden until you hover the pill, so it's there to enable but
              never clutters. One control — click to arm, click to disarm. */}
          {screenshotEnabled && !showError && (
            <button
              className={`apill-screen${screenActive ? " armed" : ""}`}
              onClick={toggleScreen}
              onMouseDown={stopDrag}
              title={
                screenActive
                  ? t("assistant.pill.disarmScreen")
                  : t("assistant.pill.armScreen")
              }
              aria-label={
                screenActive
                  ? t("assistant.pill.disarmScreen")
                  : t("assistant.pill.armScreen")
              }
              aria-pressed={screenActive}
            >
              {screenActive ? (
                <Camera size={9} strokeWidth={2.5} />
              ) : (
                <CameraOff size={9} strokeWidth={2.5} />
              )}
            </button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className={shellClass}>
      <div className="assistant-panel">
        <ResizeHandles />
        {dropActive && (
          <div className="assistant-drop-hint">
            <Paperclip size={14} />
            {t("assistant.attach.dropHint")}
          </div>
        )}
        <div className="assistant-header" data-tauri-drag-region>
          <div className="assistant-title" data-tauri-drag-region>
            <span
              className={`assistant-status-dot${busy ? " busy" : ""}`}
              data-tauri-drag-region
              title={busy ? t(`assistant.status.${state}`) : undefined}
            />
            <div className="assistant-character" ref={characterMenuRef}>
              <button
                type="button"
                className="assistant-character-switch"
                onClick={() => setCharacterMenuOpen((v) => !v)}
                title={t("assistant.character.switch")}
              >
                <CharacterAvatar character={activeCharacter} size={18} />
                <span className="assistant-character-name">
                  {activeCharacter?.name ?? t("assistant.title")}
                </span>
                <ChevronDown size={12} className="as-chevron" />
              </button>
              {characterMenuOpen && (
                <div className="assistant-character-menu">
                  {characters.map((character) => (
                    <button
                      key={character.id}
                      type="button"
                      className={`assistant-character-item${
                        character.id === activeCharacterId ? " active" : ""
                      }`}
                      onClick={() => selectCharacter(character.id)}
                    >
                      <CharacterAvatar character={character} size={18} />
                      <span className="assistant-character-name">
                        {character.name}
                      </span>
                      {character.id === activeCharacterId && (
                        <Check size={13} className="as-check" />
                      )}
                    </button>
                  ))}
                </div>
              )}
            </div>
          </div>
          <div className="assistant-header-actions">
            <button
              className={`assistant-icon-button${ttsEnabled ? " active" : ""}${
                tts.status === "loading" ? " pulsing" : ""
              }`}
              onClick={toggleTts}
              title={ttsTitle}
            >
              {ttsEnabled ? <Volume2 size={14} /> : <VolumeX size={14} />}
            </button>
            <button
              className="assistant-icon-button"
              onClick={clearConversation}
              title={t("assistant.clear")}
            >
              <Eraser size={14} />
            </button>
            <button
              className="assistant-icon-button"
              onClick={() => collapse(true)}
              title={t("assistant.pill.collapse")}
            >
              <Minimize2 size={14} />
            </button>
            <button
              className="assistant-icon-button close"
              onClick={hidePanel}
              title={t("assistant.hide")}
            >
              <X size={15} />
            </button>
          </div>
        </div>

        <div className="assistant-messages" ref={listRef}>
          {history.length === 0 && state === "idle" && !error && (
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
                  <ReactMarkdown
                    remarkPlugins={[remarkGfm]}
                    components={MD_COMPONENTS}
                  >
                    {message.content}
                  </ReactMarkdown>
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
              {message.role === "assistant" && (
                <CopyButton
                  content={message.content}
                  title={t("assistant.copy")}
                />
              )}
              {message.role === "assistant" &&
                i === history.length - 1 &&
                !busy &&
                stream === "" && (
                  <div className="assistant-last-actions">
                    <button
                      onClick={() => void commands.assistantRegenerate()}
                      title={t("assistant.regenerate")}
                      aria-label={t("assistant.regenerate")}
                    >
                      <RotateCcw size={12.5} />
                    </button>
                  </div>
                )}
            </div>
          ))}
          {notice && (
            <div className="assistant-notice" role="status">
              <Globe size={12} strokeWidth={2} />
              {t(`assistant.notices.${notice}`, {
                defaultValue: t("assistant.notices.web_search_failed"),
              })}
            </div>
          )}
          {stream !== "" && (
            <div className="assistant-message assistant">
              <div className="assistant-message-content">
                <ReactMarkdown
                  remarkPlugins={[remarkGfm]}
                  components={MD_COMPONENTS}
                >
                  {stream}
                </ReactMarkdown>
              </div>
            </div>
          )}
          {(state === "listening" || state === "transcribing") && (
            <div className={`assistant-listening ${state}`}>
              <AudioWaveform
                levels={micLevels}
                size="md"
                barCount={16}
                active={state === "listening"}
              />
              <span className="listening-label">
                {screenActive && (
                  <Camera size={13} strokeWidth={2} className="listening-cam" />
                )}
                {state === "listening" && locked && (
                  <Lock size={13} strokeWidth={2} className="listening-lock" />
                )}
                {state === "listening" && locked
                  ? t("assistant.status.locked")
                  : t(`assistant.status.${state}`)}
              </span>
            </div>
          )}
          {showTypingDots && (
            <div className="assistant-message assistant typing">
              <span className="typing-dot" />
              <span className="typing-dot" />
              <span className="typing-dot" />
            </div>
          )}
          {error && (
            <div className="assistant-message error" role="alert">
              <AlertCircle size={14} className="assistant-error-icon" />
              <div className="assistant-error-body">
                <div>{errorPrimary(error)}</div>
                {error.detail && error.detail !== errorPrimary(error) && (
                  <div className="assistant-error-detail">{error.detail}</div>
                )}
              </div>
              <button
                className="assistant-error-dismiss"
                onClick={() => setError(null)}
                title={t("assistant.pill.dismiss")}
                aria-label={t("assistant.pill.dismiss")}
              >
                <X size={13} strokeWidth={2.5} />
              </button>
            </div>
          )}
        </div>

        {(pendingImages.length > 0 || pendingFiles.length > 0) && (
          <div className="assistant-attachments">
            {pendingImages.map((image) => (
              <span className="attachment-chip" key={image.id}>
                <img src={image.dataUrl} alt="" />
                <span className="chip-name">{t("assistant.attach.image")}</span>
                <button
                  className="chip-remove"
                  onClick={() =>
                    setPendingImages((prev) =>
                      prev.filter((i) => i.id !== image.id),
                    )
                  }
                  title={t("assistant.attach.remove")}
                >
                  <X size={11} strokeWidth={2.5} />
                </button>
              </span>
            ))}
            {pendingFiles.map((file) => (
              <span className="attachment-chip" key={file.id}>
                <FileText size={13} />
                <span className="chip-name">{file.name}</span>
                <button
                  className="chip-remove"
                  onClick={() =>
                    setPendingFiles((prev) =>
                      prev.filter((f) => f.id !== file.id),
                    )
                  }
                  title={t("assistant.attach.remove")}
                >
                  <X size={11} strokeWidth={2.5} />
                </button>
              </span>
            ))}
          </div>
        )}

        <div className="assistant-input-row">
          <button
            className={`assistant-attach-button${webSearchEnabled ? " armed" : ""}`}
            onClick={toggleWebSearch}
            title={
              webSearchEnabled
                ? t("assistant.webSearch.disable")
                : t("assistant.webSearch.enable")
            }
          >
            <Globe size={15} />
          </button>
          {screenshotEnabled && (
            <button
              className={`assistant-attach-button${attachScreen ? " armed" : ""}`}
              onClick={() => {
                const next = !attachScreen;
                setAttachScreen(next);
                // Sync to backend so voice turns (hotkey or pill mic) attach
                // the screenshot too. Sticky: stays on until toggled off.
                void commands.setAssistantScreenArmed(next);
              }}
              title={
                attachScreen
                  ? t("assistant.detachScreen")
                  : t("assistant.attachScreen")
              }
            >
              <Camera size={15} />
            </button>
          )}
          {screenshotEnabled && (
            <button
              className="assistant-attach-button"
              onClick={beginSnip}
              title={t("assistant.attach.snip")}
            >
              <Scissors size={15} />
            </button>
          )}
          <button
            className="assistant-attach-button"
            onClick={pickFiles}
            title={t("assistant.attach.file")}
          >
            <Paperclip size={15} />
          </button>
          <input
            className="assistant-input"
            type="text"
            value={input}
            placeholder={
              attachScreen
                ? t("assistant.inputPlaceholderScreen")
                : t("assistant.inputPlaceholder")
            }
            onChange={(e) => {
              setInput(e.target.value);
              // Typing a new message clears any lingering error so the user
              // isn't blocked by (or waiting out) a stale failure notice.
              if (error) setError(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.repeat) {
                void sendText();
              }
            }}
          />
          <button
            className="assistant-send-button"
            onClick={isListening ? finishVoice : showStop ? stopTurn : sendText}
            disabled={!isListening && !showStop && !input.trim()}
            title={
              isListening
                ? t("assistant.finish")
                : showStop
                  ? t("assistant.stop")
                  : t("assistant.send")
            }
          >
            {isListening ? (
              <Check size={16} strokeWidth={2.75} />
            ) : showStop ? (
              <Square size={15} strokeWidth={2.5} />
            ) : (
              <ArrowUp size={16} strokeWidth={2.5} />
            )}
          </button>
        </div>
      </div>
    </div>
  );
};

export default AssistantPanel;
