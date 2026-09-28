import React from "react";
import {
  ArrowUp,
  Check,
  Copy,
  CornerDownLeft,
  Eye,
  Loader2,
  RotateCcw,
  Square,
  TextSelect,
  X,
} from "lucide-react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { useTranslation } from "react-i18next";
import { voiceEnergy } from "@/components/shared/waveformSignal";
import { quickAskShape, type QuickAskPhase } from "./quickAskState";

/**
 * The quick ask: one surface that is a pill while it waits, a bar while you type,
 * and a card once there is an answer to read.
 *
 * It is one element in three shapes rather than three components swapped in and
 * out, so the pill *becomes* the card — it widens and unfolds in place — instead of
 * one thing vanishing and another appearing somewhere else. The window around it
 * never changes size while this happens (see `ask_placement` in `assistant.rs`),
 * which is what lets the answer start appearing on its first token.
 *
 * The waiting pill is the call's status bubble, deliberately: the same graphite
 * surface, the same three level bars while you speak, the same small spinner
 * while it works, and a few words of status. It used to carry a sparkle badge
 * with a teal ring, a teal edge, a waveform and "Screen" / "Selection" chips —
 * five signals for one fact ("I'm on it") on a surface that should be glanced
 * at, and none of them shared with the call beside it. What the answer was about
 * (a selection, the screen) is said on the card, where it explains the answer.
 *
 * There is no follow-up field. The quick ask is for one job — "what does this
 * mean", "translate this", "write the reply" — and it ends with Insert or Copy. A
 * conversation is what the call is for.
 *
 * Presentational: every action is a prop, so the settings preview and the tests
 * render exactly what the panel renders.
 */

export interface QuickAskProps {
  phase: QuickAskPhase;
  /** What the assistant is doing, in a few words, for the waiting pill. */
  status: string;
  /** Microphone levels while listening; drives the three level bars. */
  levels?: number[];
  /** The question as asked, without the selection it was about. */
  question: string;
  /** The answer, or as much of it as has streamed in. */
  answer: string;
  /** Markdown renderers (code blocks with their own copy button). */
  markdown?: Components;
  /** Characters of selected text this ask is about, 0 for none. */
  selectionChars: number;
  /** The screen was looked at for this ask. */
  screen: boolean;
  /** A failure to show in place of the answer. */
  error: string | null;
  /** A quiet heads-up about the answer, e.g. web search found nothing. */
  notice: string | null;
  /** Whether "Try again" can do anything (there is a question to re-ask). */
  canRetry: boolean;

  /** Typed input, for the prompt. */
  input: string;
  onInputChange: (value: string) => void;
  onSubmit: () => void;
  /** Focus the field when the prompt appears. */
  autoFocus?: boolean;
  /**
   * The field is about to be used. The panel does not take the keyboard when it
   * appears, so that a voice ask cannot steal focus from the app it is about; this
   * is where it asks for it.
   */
  onRequestKeyboard?: () => void;

  /** Put the surface away (nothing in flight). */
  onClose: () => void;
  /** Stop whatever is in flight and put the surface away. */
  onCancel: () => void;
  /** Stop the answer where it is and keep what has been written. */
  onStop: () => void;
  onRetry: () => void;
  /** Write the answer where the user was typing. Resolves true on success. */
  onInsert: (text: string) => Promise<boolean>;
  /** Keeps a button press from starting a window drag. */
  stopDrag?: (event: React.MouseEvent) => void;
}

/** A brief tick after an action, then back to the resting icon. */
function useFlash(): [boolean, () => void] {
  const [on, setOn] = React.useState(false);
  const timer = React.useRef<number | null>(null);
  React.useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );
  const flash = React.useCallback(() => {
    setOn(true);
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setOn(false), 1400);
  }, []);
  return [on, flash];
}

/**
 * What the pill shows before its words: three bars that follow the microphone
 * while listening, a spinner while anything else is under way. These are the
 * call bubble's own classes (`.call-level`, `.call-spin` in `CallBar.css`), not
 * look-alikes, so the two surfaces cannot drift apart.
 */
const Indicator: React.FC<{ phase: QuickAskPhase; levels?: number[] }> = ({
  phase,
  levels,
}) => {
  if (phase === "listening") {
    return (
      <span
        className="call-level"
        aria-hidden="true"
        style={{ "--level": voiceEnergy(levels ?? []) } as React.CSSProperties}
      >
        <span />
        <span />
        <span />
      </span>
    );
  }
  return <Loader2 size={13} className="call-spin" aria-hidden="true" />;
};

const QuickAsk: React.FC<QuickAskProps> = ({
  phase,
  status,
  levels,
  question,
  answer,
  markdown,
  selectionChars,
  screen,
  error,
  notice,
  canRetry,
  input,
  onInputChange,
  onSubmit,
  autoFocus = false,
  onRequestKeyboard,
  onClose,
  onCancel,
  onStop,
  onRetry,
  onInsert,
  stopDrag,
}) => {
  const { t } = useTranslation();
  const shape = quickAskShape(phase);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const [copied, flashCopied] = useFlash();
  const [inserted, flashInserted] = useFlash();

  // Focused imperatively, and only on the way *into* the prompt, so a surface on
  // its way out cannot pull the caret back.
  React.useEffect(() => {
    if (phase === "prompt" && autoFocus) inputRef.current?.focus();
  }, [phase, autoFocus]);

  const inFlight =
    phase === "listening" ||
    phase === "transcribing" ||
    phase === "working" ||
    phase === "answering";
  const replace = selectionChars > 0;

  // What the answer was about. Only on the card: while the pill waits, it is
  // noise — the user knows what they selected a second ago.
  const chips = shape === "card" && phase !== "error" && (
    <>
      {selectionChars > 0 && (
        <span
          className="qa-chip"
          title={t("assistant.selectionAttached", { count: selectionChars })}
        >
          <TextSelect size={11} strokeWidth={2} aria-hidden="true" />
          <span>{t("assistant.quick.selection")}</span>
        </span>
      )}
      {screen && (
        <span className="qa-chip">
          <Eye size={11} strokeWidth={2} aria-hidden="true" />
          <span>{t("assistant.screenAttached")}</span>
        </span>
      )}
    </>
  );

  const closeButton = (
    <button
      type="button"
      className="qa-icon-btn qa-close"
      onClick={inFlight ? onCancel : onClose}
      onMouseDown={stopDrag}
      title={inFlight ? t("assistant.cancel") : t("common.close")}
      aria-label={inFlight ? t("assistant.cancel") : t("common.close")}
    >
      <X size={shape === "pill" ? 13 : 14} strokeWidth={2.4} />
    </button>
  );

  const copy = () => {
    void navigator.clipboard?.writeText(answer).then(flashCopied, () => {});
  };

  const insert = () => {
    void onInsert(answer).then((ok) => {
      if (ok) flashInserted();
    });
  };

  return (
    <section
      className="qa-surface"
      data-shape={shape}
      data-phase={phase}
      aria-label={t("assistant.title")}
      data-tauri-drag-region
    >
      <div className="qa-row" data-tauri-drag-region>
        {phase === "prompt" ? (
          <>
            <input
              ref={inputRef}
              className="qa-input"
              type="text"
              value={input}
              placeholder={t("assistant.bar.placeholder")}
              aria-label={t("assistant.bar.placeholder")}
              onChange={(event) => onInputChange(event.target.value)}
              onPointerDown={onRequestKeyboard}
              onFocus={onRequestKeyboard}
              onMouseDown={stopDrag}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.repeat && !event.shiftKey)
                  onSubmit();
              }}
            />
            <button
              type="button"
              className="qa-send"
              onClick={onSubmit}
              onMouseDown={stopDrag}
              disabled={!input.trim()}
              title={t("assistant.send")}
              aria-label={t("assistant.send")}
            >
              <ArrowUp size={15} strokeWidth={2.5} />
            </button>
          </>
        ) : shape === "pill" ? (
          <>
            <Indicator phase={phase} levels={levels} />
            <span className="qa-status" role="status" aria-live="polite">
              {status}
            </span>
          </>
        ) : (
          <>
            <p
              className="qa-question"
              role={phase === "error" ? "alert" : undefined}
              data-tauri-drag-region
              title={phase === "error" ? undefined : question}
            >
              {phase === "error"
                ? error
                : question || (phase === "answering" ? status : "")}
            </p>
            {chips}
          </>
        )}

        {closeButton}
      </div>

      {shape === "card" && !(phase === "error" && !canRetry) && (
        <div className="qa-body">
          {phase !== "error" && (
            <div
              className="qa-answer ask-answer-body"
              aria-live="polite"
              aria-busy={phase === "answering"}
            >
              <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdown}>
                {answer}
              </ReactMarkdown>
            </div>
          )}

          {notice && phase !== "error" && (
            <p className="qa-notice" role="status">
              {notice}
            </p>
          )}

          <div className="qa-actions">
            {phase === "answering" ? (
              <button
                type="button"
                className="qa-btn"
                onClick={onStop}
                onMouseDown={stopDrag}
              >
                <Square size={11} strokeWidth={2.6} aria-hidden="true" />
                <span>{t("assistant.stop")}</span>
              </button>
            ) : phase === "error" ? (
              canRetry && (
                <button
                  type="button"
                  className="qa-btn"
                  onClick={onRetry}
                  onMouseDown={stopDrag}
                >
                  <RotateCcw size={12} strokeWidth={2.2} aria-hidden="true" />
                  <span>{t("assistant.conversation.retry")}</span>
                </button>
              )
            ) : (
              <>
                {canRetry && (
                  <button
                    type="button"
                    className="qa-icon-btn"
                    onClick={onRetry}
                    onMouseDown={stopDrag}
                    title={t("assistant.conversation.retry")}
                    aria-label={t("assistant.conversation.retry")}
                  >
                    <RotateCcw size={13} strokeWidth={2.2} />
                  </button>
                )}
                <span className="qa-actions-gap" />
                <button
                  type="button"
                  className="qa-btn"
                  onClick={copy}
                  onMouseDown={stopDrag}
                  title={t("assistant.copy")}
                >
                  {copied ? (
                    <Check size={13} strokeWidth={2.4} aria-hidden="true" />
                  ) : (
                    <Copy size={13} strokeWidth={2.2} aria-hidden="true" />
                  )}
                  <span>
                    {copied ? t("assistant.quick.copied") : t("assistant.copy")}
                  </span>
                </button>
                <button
                  type="button"
                  className="qa-btn primary"
                  onClick={insert}
                  onMouseDown={stopDrag}
                  title={
                    replace
                      ? t("assistant.insertReplace")
                      : t("assistant.insert")
                  }
                >
                  {inserted ? (
                    <Check size={13} strokeWidth={2.4} aria-hidden="true" />
                  ) : (
                    <CornerDownLeft
                      size={13}
                      strokeWidth={2.2}
                      aria-hidden="true"
                    />
                  )}
                  <span>
                    {replace
                      ? t("assistant.insertReplaceShort")
                      : t("assistant.insertShort")}
                  </span>
                </button>
              </>
            )}
          </div>
        </div>
      )}
    </section>
  );
};

export default QuickAsk;
