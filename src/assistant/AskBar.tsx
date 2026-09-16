import React from "react";
import { ArrowUp, Eye, Globe, Mic, Sparkles, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { AudioWaveform } from "@/components/shared";
import OverlayProgress from "@/overlay/OverlayProgress";

/**
 * The talking pill: the shape every quick ask opens in, and everything it shows
 * before there is an answer to read.
 *
 * ## Why it looks like the dictation overlay
 *
 * Because it is the same gesture. Press a key, speak, release — the only
 * difference is what happens to the words afterwards, and that difference belongs
 * in the *result*, not in the chrome. Two visual languages for one gesture is a
 * cost paid on every single use: the user has to work out which of the app's
 * surfaces they are looking at before they can read it.
 *
 * So the surface is deliberately the dictation pill: the same 34px lozenge, the
 * same `rgba(24, 22, 20, 0.94)` fill and hairline edge, the same `AudioWaveform`
 * while the microphone is open, and the same `OverlayProgress` sweep for every
 * step that cannot report a fraction. Those are shared components and shared
 * values, not copies — see `RecordingOverlay.css`, whose `.overlay-pill` this
 * mirrors, and keep them in step.
 *
 * Two things are added, and only two, because they carry the one distinction that
 * matters:
 *
 *   • a **state mark** on the left, which says both *this is the assistant* and
 *     *this is what it is doing right now* — a microphone while listening, a
 *     globe while it searches, an eye while it reads the screen. One glyph
 *     answering "what is it doing" is worth more than a label alone, and it is
 *     why the mark changes rather than being a fixed badge.
 *   • a **halo** behind the pill, which is the quietest available way to say
 *     "assistant, not dictation" without a second shape or a second colour.
 *
 * ## Why there are no buttons while it listens
 *
 * The dictation pill has none, and it is not missed: the shortcut that started
 * the recording ends it, and Esc throws it away. Cancel and confirm buttons were
 * two more things to aim at, and they were most of what made this surface look
 * bulkier than the one it is supposed to match.
 *
 * ## Why every working state is the same size
 *
 * Because watching the surface stretch while it works is worse than not knowing
 * the word for what it is doing. It used to put "Transcribing…" and then
 * "Searching the web..." into the row, and each one lengthened the pill under the
 * user's eyes — three widths for one uninterrupted action, none of them asked
 * for. The mark says the same thing in a fixed space, which is the whole reason it
 * changes.
 *
 * So the pill hugs its content and floats centred in a transparent frame, exactly
 * as the dictation overlay does, and listening, transcribing and working
 * deliberately hug to the *same* width. Only the prompt — a resting state nobody
 * watches change — is wider.
 *
 * ## Why a finished answer never appears here
 *
 * It did, briefly, and it was the wrong idea: two lines of clipped prose in a 34px
 * lozenge with a button to see the rest. An answer is the thing the user asked
 * for; it opens the card by itself (see `askAnswerReady` in
 * `AssistantPanel.tsx`). There is nothing to press, and no state in which the
 * reply is crammed into the small shape.
 *
 * An error is the one exception, and stays: it is one short line, the pill says it
 * perfectly well, and opening a card to deliver bad news is the sort of motion
 * that makes an app feel unreliable.
 */

export type AskBarPhase = "listening" | "transcribing" | "working" | "prompt";

export interface AskBarProps {
  phase: AskBarPhase;
  /**
   * Whether the pill is the layer on screen.
   *
   * Both shapes stay mounted — the card has to be in the DOM to be measured, and
   * the pill has to be there to fade out of — so exactly one of them may hold the
   * caret. Without this the pill's field took focus back the moment a reply landed
   * and turned it into the follow-up field, and typing went into a surface at zero
   * opacity.
   */
  active: boolean;
  /**
   * The assistant's own state name (`searching`, `thinking`, …), which picks the
   * mark. Separate from `status` because one is a glyph and the other is prose,
   * and deriving the glyph by matching translated text would break in every
   * language but English.
   */
  state?: string;
  /** What the assistant is doing, in words. Read by assistive technology and used
   * as the progress indicator's label; not drawn while working, because that is
   * what would make the pill grow. */
  status: string;
  /** Microphone levels for the waveform. */
  levels?: number[];
  /** A short failure, shown in place of the status. */
  error?: string | null;
  /** Text being typed, for the prompt phase. */
  input: string;
  onInputChange: (value: string) => void;
  onSubmit: () => void;
  onClose: () => void;
  /** Keeps a control press from starting a window drag. */
  stopDrag: (event: React.MouseEvent) => void;
}

/** The glyph for what the assistant is doing, at the pill's own small size. */
const StateMark: React.FC<{ phase: AskBarPhase; state?: string }> = ({
  phase,
  state,
}) => {
  const size = 13;
  const stroke = 1.9;
  if (phase === "listening" || phase === "transcribing") {
    return <Mic size={size} strokeWidth={stroke} />;
  }
  if (state === "searching") {
    return <Globe size={size} strokeWidth={stroke} />;
  }
  if (state === "screen") {
    return <Eye size={size} strokeWidth={stroke} />;
  }
  return <Sparkles size={size} strokeWidth={stroke} />;
};

const AskBar: React.FC<AskBarProps> = ({
  phase,
  active,
  state,
  status,
  levels,
  error,
  input,
  onInputChange,
  onSubmit,
  onClose,
  stopDrag,
}) => {
  const { t } = useTranslation();
  const inputRef = React.useRef<HTMLInputElement>(null);
  const listening = phase === "listening";
  const prompt = phase === "prompt";
  const typable = prompt && active;
  // Focused imperatively rather than with `autoFocus`, because the field is
  // remounted every time the phase comes back round to a prompt — including on
  // the way *out*, as an answer lands and the card takes over. `autoFocus` fires
  // on that mount too, which put the caret in a pill at zero opacity.
  React.useEffect(() => {
    if (typable) inputRef.current?.focus();
  }, [typable]);

  // Only two things widen the pill, and neither is something the user watches
  // happen: a field to type in, and a failure to read. Every working state keeps
  // the bare lozenge.
  const labeled = prompt || !!error;

  return (
    <div
      className={`ask-pill${labeled ? " labeled" : ""}`}
      data-tauri-drag-region
    >
      <span
        className="ask-pill-mark"
        role="img"
        aria-label={status}
        data-tauri-drag-region
      >
        <StateMark phase={phase} state={state} />
      </span>

      {/* The microphone is open: the honest signal is the user's own voice
          moving. Everything after it is a step that cannot report a fraction, so
          it gets the same indeterminate sweep the dictation overlay uses for
          transcription and cleanup — at the same width, so the swap between them
          does not resize the pill. */}
      {!prompt && !error ? (
        <div className="ask-pill-indicator" data-tauri-drag-region>
          {listening ? (
            <AudioWaveform
              levels={levels ?? []}
              size="sm"
              barCount={14}
              mode={levels && levels.length > 0 ? "reactive" : "shimmer"}
              active
            />
          ) : (
            <OverlayProgress label={status} completed={false} active />
          )}
        </div>
      ) : null}

      {prompt ? (
        <input
          ref={inputRef}
          className="ask-pill-input"
          type="text"
          value={input}
          disabled={!active}
          placeholder={t("assistant.bar.placeholder")}
          onChange={(event) => onInputChange(event.target.value)}
          onMouseDown={stopDrag}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.repeat) onSubmit();
          }}
        />
      ) : error ? (
        <span className="ask-pill-label error" role="alert">
          {error}
        </span>
      ) : null}

      {prompt && (
        <button
          type="button"
          className="ask-pill-btn primary"
          onClick={onSubmit}
          onMouseDown={stopDrag}
          disabled={!typable || !input.trim()}
          title={t("assistant.send")}
          aria-label={t("assistant.send")}
        >
          <ArrowUp size={12} strokeWidth={2.4} />
        </button>
      )}

      {/* Kept off the working states on purpose: the shortcut that started the
          recording ends it and Esc discards it, exactly as with dictation. A
          failure is the other case that needs dismissing, since nothing else will
          come along to replace it. */}
      {(prompt || !!error) && (
        <button
          type="button"
          className="ask-pill-btn quiet"
          onClick={onClose}
          onMouseDown={stopDrag}
          title={t("assistant.hide")}
          aria-label={t("assistant.hide")}
        >
          <X size={12} strokeWidth={2.2} />
        </button>
      )}
    </div>
  );
};

export default AskBar;
