import { listen } from "@tauri-apps/api/event";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import {
  ArrowUp,
  Camera,
  Check,
  Copy,
  Eraser,
  Maximize2,
  Mic,
  Minimize2,
  Square,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import { commands, type AppSettings } from "@/bindings";
import { syncLanguageFromSettings } from "@/i18n";
import { useKokoroTts } from "./useKokoroTts";
import "./AssistantPanel.css";

type AssistantState = "idle" | "listening" | "transcribing" | "thinking";

interface DisplayMessage {
  role: "user" | "assistant";
  content: string;
  screenshot?: boolean;
}

/** Must match SCREENSHOT_MARKER in src-tauri/src/assistant.rs */
const SCREENSHOT_MARKER = "[screenshot attached]";

const ACCENTS: Record<string, [string, string]> = {
  violet: ["#6366f1", "#8b5cf6"],
  blue: ["#2563eb", "#06b6d4"],
  emerald: ["#059669", "#34d399"],
  rose: ["#e11d48", "#ec4899"],
  amber: ["#d97706", "#f59e0b"],
  mono: ["#52525b", "#71717a"],
};

const FONT_SIZES: Record<string, string> = {
  small: "12.5px",
  medium: "13.5px",
  large: "15px",
};

function toDisplay(raw: { role: string; content: string }): DisplayMessage {
  const role = raw.role === "assistant" ? "assistant" : "user";
  if (raw.content.endsWith(SCREENSHOT_MARKER)) {
    return {
      role,
      content: raw.content.slice(0, -SCREENSHOT_MARKER.length).trimEnd(),
      screenshot: true,
    };
  }
  return { role, content: raw.content };
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

const AssistantPanel: React.FC = () => {
  const { t } = useTranslation();
  // Conversation snapshots from the backend are the single source of truth;
  // `stream` only holds the in-flight answer between snapshots. This makes
  // rendering idempotent: duplicate events can never duplicate messages.
  const [history, setHistory] = useState<DisplayMessage[]>([]);
  const [stream, setStream] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [state, setState] = useState<AssistantState>("idle");
  const [input, setInput] = useState("");
  const [attachScreen, setAttachScreen] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [locked, setLocked] = useState(false);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const sendingRef = useRef(false);

  const ttsEnabled = settings?.assistant_tts_enabled ?? false;
  const ttsVoice = settings?.assistant_tts_voice ?? "af_heart";
  const screenshotEnabled = settings?.assistant_screenshot_enabled ?? true;
  const tts = useKokoroTts(ttsEnabled, ttsVoice);
  const speakRef = useRef(tts.speak);
  speakRef.current = tts.speak;

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

  // Apply customization (accent, opacity, font size) as CSS variables.
  useEffect(() => {
    if (!settings) return;
    const root = document.documentElement;
    const [from, to] =
      ACCENTS[settings.assistant_accent ?? "violet"] ?? ACCENTS.violet;
    root.style.setProperty("--accent-from", from);
    root.style.setProperty("--accent-to", to);
    root.style.setProperty(
      "--panel-alpha",
      String(settings.assistant_panel_opacity ?? 1),
    );
    root.style.setProperty(
      "--msg-font-size",
      FONT_SIZES[settings.assistant_font_size ?? "medium"] ?? FONT_SIZES.medium,
    );
  }, [settings]);

  // Auto-scroll to bottom on new content
  useEffect(() => {
    const el = listRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }, [history, stream, state, error]);

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
          setState(e.payload.state);
          if (e.payload.state !== "listening") {
            setLocked(false);
          }
          if (e.payload.state !== "idle") {
            setError(null);
          }
        }),
      );

      track(
        await listen<boolean>("recording-locked", (e) => {
          setLocked(e.payload);
        }),
      );

      track(
        await listen<boolean>("assistant-screen-armed", (e) => {
          setAttachScreen(e.payload);
        }),
      );

      track(
        await listen<{ role: string; content: string }[]>(
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
        await listen<string>("assistant-error", (e) => {
          setError(e.payload);
          setStream("");
        }),
      );

      track(
        await listen<string>("assistant-tts", (e) => {
          void speakRef.current(e.payload);
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
    };

    setup();
    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
      unlisteners.length = 0;
    };
  }, [refreshSettings]);

  const busy = state !== "idle";

  const sendText = useCallback(async () => {
    const text = input.trim();
    if (!text || sendingRef.current || busy) return;
    sendingRef.current = true;
    setInput("");
    const withScreen = attachScreen && screenshotEnabled;
    setAttachScreen(false);
    try {
      if (withScreen) {
        // Consume the backend armed flag too, so it doesn't double-fire on
        // a later voice turn.
        await commands.setAssistantScreenArmed(false);
        await commands.assistantSendTextWithScreen(text);
      } else {
        await commands.assistantSendText(text);
      }
    } catch (err) {
      setError(String(err));
    } finally {
      sendingRef.current = false;
    }
  }, [input, busy, attachScreen, screenshotEnabled]);

  const clearConversation = useCallback(async () => {
    tts.stop();
    setError(null);
    setStream("");
    await commands.assistantClearConversation();
  }, [tts]);

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
    await commands.assistantToggleVoice();
  }, []);

  const collapse = useCallback(async (value: boolean) => {
    await commands.setAssistantPanelCollapsed(value);
    setCollapsed(value);
  }, []);

  const showTypingDots = state === "thinking" && stream === "";

  const ttsTitle =
    tts.status === "loading"
      ? t("assistant.tts.loadingShort", { progress: tts.progress })
      : ttsEnabled
        ? t("assistant.tts.disable")
        : t("assistant.tts.enable");

  if (collapsed) {
    return (
      <div className="assistant-pill" data-tauri-drag-region>
        <button
          className={`pill-mic${state === "listening" ? " recording" : ""}${
            state === "transcribing" || state === "thinking" ? " working" : ""
          }`}
          onClick={toggleVoice}
          title={
            state === "listening"
              ? t("assistant.pill.stop")
              : t("assistant.pill.talk")
          }
        >
          {state === "listening" ? <Square size={16} /> : <Mic size={18} />}
        </button>
        <span className="pill-status" data-tauri-drag-region>
          {tts.status === "loading"
            ? t("assistant.tts.loadingShort", { progress: tts.progress })
            : busy
              ? t(`assistant.status.${state}`)
              : t("assistant.pill.idle")}
        </span>
        <button
          className="assistant-icon-button"
          onClick={() => collapse(false)}
          title={t("assistant.pill.expand")}
        >
          <Maximize2 size={14} />
        </button>
      </div>
    );
  }

  return (
    <div className="assistant-panel">
      <div className="assistant-header" data-tauri-drag-region>
        <div className="assistant-title" data-tauri-drag-region>
          <span
            className={`assistant-status-dot${busy ? " busy" : ""}`}
            data-tauri-drag-region
          />
          {t("assistant.title")}
          {busy && (
            <span className={`assistant-state-chip ${state}`}>
              {t(`assistant.status.${state}`)}
            </span>
          )}
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
            <p>{t("assistant.empty")}</p>
          </div>
        )}
        {history.map((message, i) => (
          <div key={i} className={`assistant-message ${message.role}`}>
            <div className="assistant-message-content">
              {message.role === "assistant" ? (
                <ReactMarkdown>{message.content}</ReactMarkdown>
              ) : (
                message.content
              )}
            </div>
            {message.screenshot && (
              <span className="screen-chip">
                <Camera size={11} />
                {t("assistant.screenAttached")}
              </span>
            )}
            {message.role === "assistant" && (
              <CopyButton
                content={message.content}
                title={t("assistant.copy")}
              />
            )}
          </div>
        ))}
        {stream !== "" && (
          <div className="assistant-message assistant">
            <div className="assistant-message-content">
              <ReactMarkdown>{stream}</ReactMarkdown>
            </div>
          </div>
        )}
        {(state === "listening" || state === "transcribing") && (
          <div className={`assistant-listening ${state}`}>
            <span className="listening-ring" />
            {state === "listening" && locked
              ? t("assistant.status.locked")
              : t(`assistant.status.${state}`)}
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
          <div className="assistant-message assistant error">
            <div className="assistant-message-content">{error}</div>
          </div>
        )}
      </div>

      <div className="assistant-input-row">
        {screenshotEnabled && (
          <button
            className={`assistant-attach-button${attachScreen ? " armed" : ""}`}
            onClick={() => {
              const next = !attachScreen;
              setAttachScreen(next);
              // Sync to backend so voice turns (hotkey or pill mic) attach
              // the screenshot too.
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
        <input
          className="assistant-input"
          type="text"
          value={input}
          placeholder={
            attachScreen
              ? t("assistant.inputPlaceholderScreen")
              : t("assistant.inputPlaceholder")
          }
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.repeat) {
              void sendText();
            }
          }}
        />
        <button
          className="assistant-send-button"
          onClick={sendText}
          disabled={!input.trim() || busy}
          title={t("assistant.send")}
        >
          <ArrowUp size={16} strokeWidth={2.5} />
        </button>
      </div>
    </div>
  );
};

export default AssistantPanel;
