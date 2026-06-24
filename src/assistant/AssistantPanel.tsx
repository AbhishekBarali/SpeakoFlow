import { listen } from "@tauri-apps/api/event";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  ArrowUp,
  Camera,
  Check,
  Copy,
  Eraser,
  Globe,
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
import {
  applyAssistantTheme,
  type AssistantThemePref,
  type ThemePreference,
} from "@/lib/theme";
import { AudioWaveform } from "@/components/shared";
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
  const [ttsPlaying, setTtsPlaying] = useState(false);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [micLevels, setMicLevels] = useState<number[]>([]);
  const [visionActive, setVisionActive] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  const sendingRef = useRef(false);
  // Tracks whether audio actually began during the current "speaking" phase,
  // so we can detect when playback *ends* and hand the UI back to idle.
  const spokeRef = useRef(false);
  // Set when the user presses Stop; blocks a TTS event that was emitted just
  // before the Stop from slipping through (events can arrive slightly out of
  // order). Cleared as soon as any new turn becomes active, so it never blocks
  // a legitimate next answer.
  const suppressTtsRef = useRef(false);

  const ttsEnabled = settings?.assistant_tts_enabled ?? false;
  const ttsVoice = settings?.assistant_tts_voice ?? "af_heart";
  const ttsDtype = settings?.assistant_tts_kokoro_dtype ?? "fp32";
  const ttsSpeed = settings?.assistant_tts_speed ?? 1;
  const screenshotEnabled = settings?.assistant_screenshot_enabled ?? true;
  const webSearchEnabled = settings?.assistant_web_search_enabled ?? false;
  const tts = useKokoroTts(ttsEnabled, ttsVoice, ttsDtype, ttsSpeed);
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
    // The panel follows the app-wide theme by default; a light/dark choice in
    // settings (or the header toggle) overrides it for the panel only.
    applyAssistantTheme(
      (settings.assistant_panel_theme ?? "auto") as AssistantThemePref,
      (settings.theme ?? "system") as ThemePreference,
    );
  }, [settings]);

  // Re-resolve on OS scheme changes (covers app "system" + panel "auto").
  const appThemePref = (settings?.theme ?? "system") as ThemePreference;
  const panelThemePref = (settings?.assistant_panel_theme ??
    "auto") as AssistantThemePref;
  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = () => applyAssistantTheme(panelThemePref, appThemePref);
    media.addEventListener("change", handler);
    return () => media.removeEventListener("change", handler);
  }, [panelThemePref, appThemePref]);

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
          setState(e.payload.state);
          if (e.payload.state !== "listening") {
            setLocked(false);
          }
          if (e.payload.state !== "idle") {
            setError(null);
            // A new turn is active — allow its eventual spoken reply through,
            // clearing any suppression left by a previous Stop.
            suppressTtsRef.current = false;
          }
          // The turn finished — drop the screen-vision indicator.
          if (e.payload.state === "idle") {
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
    };

    setup();
    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
      unlisteners.length = 0;
    };
  }, [refreshSettings]);

  const busy = state !== "idle";
  const isListening = state === "listening";
  const ttsActive =
    ttsPlaying || tts.status === "speaking" || tts.status === "loading";
  const showStop = busy || ttsActive;

  // The backend parks the turn in "speaking" when a spoken reply is starting,
  // so the pill/panel never flashes its idle "Assistant" affordance in the gap
  // before audio begins. We own the end of that phase: once playback has
  // started and then stopped (ttsActive falls after having risen), return to
  // idle. A safety timeout avoids a stuck "speaking" pill if audio never plays
  // (e.g. a TTS error), and works for both the local Kokoro and remote engines.
  useEffect(() => {
    if (state !== "speaking") {
      spokeRef.current = false;
      return;
    }
    if (ttsActive) {
      spokeRef.current = true;
      return;
    }
    if (spokeRef.current) {
      setState("idle");
      return;
    }
    const timer = window.setTimeout(() => setState("idle"), 10000);
    return () => window.clearTimeout(timer);
  }, [state, ttsActive]);

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

  // Finish a hands-free (tap-to-lock or toggle) voice capture and send it —
  // the keyboard-free equivalent of pressing the hotkey again. Stops the
  // recording and runs the assistant turn on it.
  const finishVoice = useCallback(async () => {
    await commands.commitRecording();
  }, []);

  const toggleWebSearch = useCallback(async () => {
    await commands.setAssistantWebSearchEnabled(!webSearchEnabled);
    await refreshSettings();
  }, [webSearchEnabled, refreshSettings]);

  const collapse = useCallback(async (value: boolean) => {
    await commands.setAssistantPanelCollapsed(value);
    setCollapsed(value);
  }, []);

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

  if (collapsed) {
    // Anything the main panel lets you stop — a generating/transcribing turn
    // or a (possibly long) TTS readout — should be stoppable from the pill too,
    // without expanding it. Listening is excluded: there the tick means
    // "finish and send", not "cancel".
    const pillStop = showStop && !isListening;
    const pillStatus =
      tts.status === "loading"
        ? t("assistant.tts.loadingShort", { progress: tts.progress })
        : busy
          ? t(`assistant.status.${state}`)
          : ttsActive
            ? t("assistant.status.speaking")
            : t("assistant.pill.idle");
    return (
      <div className="assistant-pill" data-tauri-drag-region>
        <div className="pill-mic-wrap">
          <button
            className={`pill-mic${isListening ? " recording" : ""}${
              pillStop ? " stopping" : ""
            }`}
            onClick={
              isListening ? finishVoice : pillStop ? stopTurn : toggleVoice
            }
            title={
              isListening
                ? t("assistant.finish")
                : pillStop
                  ? t("assistant.stop")
                  : t("assistant.pill.talk")
            }
          >
            {isListening ? (
              <Check size={16} strokeWidth={2.75} />
            ) : pillStop ? (
              <Square size={15} strokeWidth={2.5} />
            ) : (
              <Mic size={17} strokeWidth={2} />
            )}
          </button>
          {screenActive && (
            <span
              className="pill-screen-badge"
              title={t("assistant.screenAttached")}
            >
              <Camera size={10} strokeWidth={2.5} />
            </span>
          )}
        </div>
        {isListening ? (
          <div className="pill-wave" data-tauri-drag-region>
            <AudioWaveform
              levels={micLevels}
              size="md"
              barCount={21}
              active={true}
            />
          </div>
        ) : (
          <span className="pill-status" data-tauri-drag-region>
            {pillStatus}
          </span>
        )}
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
                <ReactMarkdown remarkPlugins={[remarkGfm]}>
                  {message.content}
                </ReactMarkdown>
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
              <ReactMarkdown remarkPlugins={[remarkGfm]}>
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
              barCount={29}
              active={state === "listening"}
            />
            <span className="listening-label">
              {screenActive && (
                <Camera size={13} strokeWidth={2} className="listening-cam" />
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
          <div className="assistant-message assistant error">
            <div className="assistant-message-content">{error}</div>
          </div>
        )}
      </div>

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
  );
};

export default AssistantPanel;
