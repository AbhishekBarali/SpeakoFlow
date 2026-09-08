import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  Maximize2,
  MessageSquare,
  Mic,
  MicOff,
  PhoneOff,
  RotateCcw,
  SlidersHorizontal,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  CONVERSATION_PACES,
  type ConversationPace,
} from "./conversationPolicy";
import type { useVoiceConversation } from "./useVoiceConversation";
import { VoiceOrb } from "./VoiceOrb";

type Voice = ReturnType<typeof useVoiceConversation>;

export function ConversationCallButtons({ voice }: { voice: Voice }) {
  const { t } = useTranslation();
  const muted = voice.phase === "muted";
  const muteLabel = t(`assistant.conversation.${muted ? "unmute" : "mute"}`);
  return (
    <>
      <button
        type="button"
        className={`conversation-control${muted ? " muted" : ""}`}
        disabled={["off", "error", "starting"].includes(voice.phase)}
        onClick={() => void voice.toggleMute()}
        aria-pressed={muted}
        aria-label={muteLabel}
        title={muteLabel}
      >
        {muted ? <MicOff size={20} /> : <Mic size={20} />}
      </button>
      <button
        type="button"
        className="conversation-control end"
        onClick={voice.end}
        aria-label={t("assistant.conversation.end")}
        title={t("assistant.conversation.end")}
      >
        <PhoneOff size={20} />
      </button>
    </>
  );
}

export function ConversationPill({
  voice,
  name,
  onExpand,
  voiceLoading = null,
  voiceFault = null,
}: {
  voice: Voice;
  name: string;
  onExpand: () => void;
  /** See `Props.voiceLoading`. The pill is the default collapsed form during a
   *  call, so the first-call wait has to read the same here. */
  voiceLoading?: number | null;
  /** See `Props.voiceFault`. Same reason: a voice that failed while collapsed
   *  must not be silent here either. The pill has room for one line, so the
   *  fault replaces the phase rather than sitting beside it. */
  voiceFault?: string | null;
}) {
  const { t } = useTranslation();
  const preparingVoice = voiceLoading !== null && voice.phase !== "hearing";
  return (
    <div
      className={`conversation-pill${voiceFault ? " has-error" : ""}`}
      data-tauri-drag-region
    >
      <button
        className="conversation-pill-open"
        type="button"
        onClick={onExpand}
        aria-label={t("assistant.pill.expand")}
        title={voiceFault ?? name}
      >
        <VoiceOrb phase={voice.phase} level={voice.level} />
        <span role={voiceFault ? "alert" : "status"}>
          {voiceFault ??
            (preparingVoice
              ? t("assistant.conversation.preparingVoice", {
                  progress: voiceLoading,
                })
              : t(`assistant.conversation.phase.${voice.phase}`))}
        </span>
        <Maximize2 size={12} />
      </button>
      <ConversationCallButtons voice={voice} />
    </div>
  );
}

interface Props {
  voice: Voice;
  profilePicker: ReactNode;
  answer: string;
  showTranscript: boolean;
  onToggleTranscript: () => void;
  /**
   * Percentage complete while the on-device voice loads, or `null` when it is
   * not loading.
   *
   * A call speaks every reply, so the first one on a fresh install waits on a
   * ~325 MB model download. Without this the orb just said "Thinking" for the
   * length of it, and the panel's own progress chip is in the header that the
   * conversation view replaces.
   */
  voiceLoading?: number | null;
  /**
   * A voice-engine failure, already translated. It reaches the orb view because
   * the panel's error banner lives in the message list, which is hidden behind
   * the orb — so a voice that failed to load left the call silent with the
   * explanation on a surface nobody could see.
   */
  voiceFault?: string | null;
}

export function ConversationView({
  voice,
  profilePicker,
  answer,
  showTranscript,
  onToggleTranscript,
  voiceLoading = null,
  voiceFault = null,
}: Props) {
  const { t } = useTranslation();
  const [optionsOpen, setOptionsOpen] = useState(false);
  const options = useRef<HTMLDivElement>(null);
  const optionsButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!optionsOpen) return;
    const outside = (event: PointerEvent) => {
      if (!options.current?.contains(event.target as Node))
        setOptionsOpen(false);
    };
    window.addEventListener("pointerdown", outside);
    return () => window.removeEventListener("pointerdown", outside);
  }, [optionsOpen]);
  const fault = voiceFault;
  // What the user is doing beats what the app is doing: while they are actually
  // speaking the orb says so, and the voice-loading line waits for the gap.
  const preparingVoice = voiceLoading !== null && voice.phase !== "hearing";

  return (
    <section
      className={`assistant-conversation${showTranscript ? " compact" : ""}${voice.error || fault ? " has-error" : ""}`}
      aria-label={t("assistant.conversation.title")}
    >
      <div className="conversation-profile-row">{profilePicker}</div>
      <div className="conversation-stage">
        <VoiceOrb phase={voice.phase} level={voice.level} />
        <div className="conversation-status" role="status" aria-live="polite">
          {preparingVoice
            ? t("assistant.conversation.preparingVoice", {
                progress: voiceLoading,
              })
            : t(`assistant.conversation.phase.${voice.phase}`)}
        </div>
      </div>
      {!showTranscript && !voice.error && !fault && answer && (
        <button
          type="button"
          className="conversation-caption"
          onClick={onToggleTranscript}
          title={t("assistant.conversation.transcript")}
        >
          <span>{answer}</span>
          <span className="conversation-caption-link">
            {t("assistant.conversation.readTranscript")}
          </span>
        </button>
      )}
      {fault && (
        <div className="conversation-error" role="alert">
          <p>{fault}</p>
        </div>
      )}
      {voice.error && !fault && (
        <div className="conversation-error" role="alert">
          <p>{t(`assistant.conversation.errors.${voice.error.code}`)}</p>
          {voice.error.detail && (
            <details>
              <summary>{t("assistant.conversation.details")}</summary>
              <p>{voice.error.detail}</p>
            </details>
          )}
          {/* A recoverable turn error leaves the session listening, so Retry
              (which restarts the whole session) belongs only to a dead one. */}
          {voice.phase === "error" && (
            <button
              type="button"
              className="conversation-retry"
              onClick={() => void voice.start()}
            >
              <RotateCcw size={15} />
              {t("assistant.conversation.retry")}
            </button>
          )}
        </div>
      )}
      <div className="conversation-footer">
        <div className="conversation-controls">
          <button
            type="button"
            className="conversation-secondary"
            aria-pressed={showTranscript}
            aria-label={t(
              showTranscript
                ? "assistant.conversation.focusView"
                : "assistant.conversation.transcript",
            )}
            title={t(
              showTranscript
                ? "assistant.conversation.focusView"
                : "assistant.conversation.transcript",
            )}
            onClick={onToggleTranscript}
          >
            <MessageSquare size={18} />
          </button>
          <ConversationCallButtons voice={voice} />
          <div
            className="conversation-options"
            ref={options}
            onBlur={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget))
                setOptionsOpen(false);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape" && optionsOpen) {
                event.preventDefault();
                event.stopPropagation();
                setOptionsOpen(false);
                optionsButton.current?.focus();
              }
            }}
          >
            <button
              ref={optionsButton}
              type="button"
              className="conversation-secondary"
              aria-expanded={optionsOpen}
              aria-label={t("assistant.conversation.options")}
              title={t("assistant.conversation.options")}
              onClick={() => setOptionsOpen(!optionsOpen)}
            >
              <SlidersHorizontal size={18} />
            </button>
            {optionsOpen && (
              <div className="conversation-options-popover">
                <label htmlFor="conversation-pace">
                  {t("assistant.conversation.pace")}
                </label>
                <p>{t("assistant.conversation.paceHint")}</p>
                <select
                  id="conversation-pace"
                  value={voice.pace}
                  onChange={(event) =>
                    voice.setPace(event.target.value as ConversationPace)
                  }
                >
                  {CONVERSATION_PACES.map((pace) => (
                    <option key={pace} value={pace}>
                      {t(`assistant.conversation.paces.${pace}`)}
                    </option>
                  ))}
                </select>
              </div>
            )}
          </div>
        </div>
        <p className="conversation-footnote">
          {t(
            voice.phase === "muted"
              ? "assistant.conversation.mutedHint"
              : voice.phase === "error"
                ? "assistant.conversation.errorHint"
                : "assistant.conversation.interruptHint",
          )}
        </p>
      </div>
    </section>
  );
}
