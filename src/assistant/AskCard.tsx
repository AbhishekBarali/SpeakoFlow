import React from "react";
import {
  Check,
  Copy,
  CornerDownLeft,
  Info,
  Loader2,
  Mic,
  X,
} from "lucide-react";
import ReactMarkdown, { type Components } from "react-markdown";
import { useTranslation } from "react-i18next";
import { AudioWaveform } from "@/components/shared";
import { commands } from "@/bindings";

/**
 * The quick-ask card: one question, one answer, nothing else.
 *
 * This is where the assistant's answer lands, and it deliberately shows a single
 * exchange rather than a scrolling thread. The job it exists for is "what does
 * this mean", "translate this", "write that reply" — you ask, you read the
 * answer, you copy or insert it, and it goes away. A message list turns that
 * into a chat app you have to navigate, which is the thing that made the old
 * floating panel feel heavy: the answer you actually wanted was one item in a
 * log, and reading it meant scrolling.
 *
 * The card is also the second half of a two-part surface. Before there is an
 * answer the window is the talking bar (`AskBar`); this unfolds out of it once
 * the reply is complete and the card has been measured, which is why its height
 * is reported back to Rust rather than chosen there — see `fit_ask_card`.
 *
 * There is no title row and no label over the answer. The header used to be a
 * sparkle glyph and the word "Answer" above a bordered box, inside another
 * bordered box, inside the panel's own border: three nested frames and a caption
 * for the one thing on screen that could not be anything else. Everything the
 * chrome carried now lives on the row with the question — a line you can grab
 * the card by, with the two actions that do something and the one that closes
 * it.
 */

export interface AskCardProps {
  /** The question as asked, already stripped of attachment markers. */
  question: string;
  /** The answer, or the partial answer while it streams in. */
  answer: string;
  /** Waiting on transcription or the model. */
  busy: boolean;
  /** Short status line while busy ("Listening", "Transcribing…", …). */
  status: string;
  /**
   * A new question is being spoken or transcribed, so nothing on screen belongs
   * to it yet.
   *
   * This is the difference between the card working and the card looking broken.
   * On a follow-up ask the card is already open, so it keeps rendering the
   * previous question and previous answer for the whole recording — and because
   * a finished answer outranks a spinner, there was no spinner either. So the
   * surface sat there, unchanged and silent, for as long as you spoke: exactly
   * the "I ask something and it is still stuck" this card exists to fix,
   * reproduced one question later.
   */
  capturing?: boolean;
  /** Microphone levels, for the waveform while listening. */
  levels?: number[];
  /** A failure to show in place of the answer. */
  error?: string | null;
  /** A quiet post-turn heads-up, e.g. web search came back with nothing. */
  notice?: string | null;
  /** Characters of selected text that rode along, 0 for none. */
  selectionChars?: number;
  /** Markdown renderers shared with the thread view. */
  markdown: Components;
  /** Send the card away. */
  onClose: () => void;
  /** Keeps a control press from starting a window drag. */
  stopDrag: (event: React.MouseEvent) => void;
}

/** Copy-to-clipboard action, with a brief tick for confirmation. */
const CardCopy: React.FC<{
  content: string;
  title: string;
  stopDrag: (event: React.MouseEvent) => void;
}> = ({ content, title, stopDrag }) => {
  const [copied, setCopied] = React.useState(false);
  return (
    <button
      type="button"
      className="ask-action"
      title={title}
      aria-label={title}
      onMouseDown={stopDrag}
      onClick={() => {
        void navigator.clipboard.writeText(content).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
        });
      }}
    >
      {copied ? <Check size={13} /> : <Copy size={13} />}
    </button>
  );
};

/**
 * Write the answer back into the app you asked from — over your selection if you
 * had one, at the caret otherwise.
 *
 * Explicit by design. Restoring focus does not reliably restore a *selection*:
 * some editors and many `contenteditable` fields collapse it to a caret on blur,
 * and when that happens a replace silently becomes an insert, leaving both the
 * original and the rewrite. That is undetectable beforehand, so it stays a button
 * pressed by someone looking at the result.
 */
const CardInsert: React.FC<{
  content: string;
  label: string;
  title: string;
  stopDrag: (event: React.MouseEvent) => void;
}> = ({ content, label, title, stopDrag }) => {
  const [done, setDone] = React.useState(false);
  return (
    <button
      type="button"
      className="ask-action labelled"
      title={title}
      aria-label={title}
      onMouseDown={stopDrag}
      onClick={() => {
        void commands.assistantInsertText(content).then((result) => {
          if (result.status !== "ok") return;
          setDone(true);
          setTimeout(() => setDone(false), 1200);
        });
      }}
    >
      {done ? <Check size={13} /> : <CornerDownLeft size={13} />}
      <span>{label}</span>
    </button>
  );
};

const AskCard: React.FC<AskCardProps> = ({
  question,
  answer,
  busy,
  status,
  capturing = false,
  levels,
  error,
  notice,
  selectionChars = 0,
  markdown,
  onClose,
  stopDrag,
}) => {
  const { t } = useTranslation();
  // Anything already on screen belongs to the previous turn while a new question
  // is still being captured, so it is withheld rather than left to look answered.
  const hasQuestion = !capturing && question.trim().length > 0;
  const hasAnswer = !capturing && answer.trim().length > 0;

  return (
    <div className="ask-card">
      {/* One row: what you asked, what you can do with the reply, and the way
          out. It doubles as the place to grab the card — `useSafeWindowDrag`
          matches the event target itself rather than its ancestors, so marking a
          container makes only its own bare area draggable and every button inside
          keeps working untouched. The question text is marked as well: you said
          it a second ago, so moving the card by it is worth more than selecting
          it. The answer body deliberately is not. */}
      <div className="ask-head" data-tauri-drag-region>
        {hasQuestion ? (
          <>
            <Mic className="ask-head-icon" size={12} aria-hidden="true" />
            <p className="ask-question-text" data-tauri-drag-region>
              {question}
            </p>
          </>
        ) : (
          <span className="ask-head-spacer" data-tauri-drag-region />
        )}
        {hasQuestion && selectionChars > 0 && (
          <span className="ask-selection-chip">
            {t("assistant.selectionAttached", { count: selectionChars })}
          </span>
        )}
        <div className="ask-head-actions">
          {hasAnswer && !error && (
            <>
              <CardCopy
                content={answer}
                title={t("assistant.copy")}
                stopDrag={stopDrag}
              />
              <CardInsert
                content={answer}
                label={
                  selectionChars > 0
                    ? t("assistant.insertReplaceShort")
                    : t("assistant.insertShort")
                }
                title={
                  selectionChars > 0
                    ? t("assistant.insertReplace")
                    : t("assistant.insert")
                }
                stopDrag={stopDrag}
              />
            </>
          )}
          <button
            type="button"
            className="ask-action close"
            onClick={onClose}
            onMouseDown={stopDrag}
            title={t("assistant.hide")}
            aria-label={t("assistant.hide")}
          >
            <X size={14} strokeWidth={2.25} />
          </button>
        </div>
      </div>

      {/* The answer. The one thing on screen that gets room to breathe, and now
          the only thing with a border around it — which is to say, none. */}
      <div className="ask-answer-body">
        {error ? (
          <p className="ask-error" role="alert">
            {error}
          </p>
        ) : capturing ? (
          // A waveform, not a spinner: while the microphone is open the honest
          // signal is the user's own voice moving. Only reachable on a follow-up,
          // since a first ask is still the bar at this point.
          <div className="ask-capture" aria-live="polite">
            <AudioWaveform
              levels={levels ?? []}
              size="md"
              barCount={18}
              mode={levels && levels.length > 0 ? "reactive" : "shimmer"}
              active
            />
            <span>{status}</span>
          </div>
        ) : hasAnswer ? (
          <ReactMarkdown components={markdown}>{answer}</ReactMarkdown>
        ) : busy ? (
          <p className="ask-waiting">
            <Loader2 className="ask-spin" size={13} />
            {status}
          </p>
        ) : (
          // Only reachable after a clear, since the card is not what opens.
          <p className="ask-empty">{t("assistant.askHint")}</p>
        )}
      </div>

      {/* A quiet heads-up that belongs to the answer above it: a web search that
          came back with nothing, or a message held until this reply finishes.

          A neutral glyph on purpose — the notice is not always about search, and
          a globe next to "Queued" states something untrue. */}
      {notice && !error && (
        <p className="ask-notice" role="status">
          <Info size={11} strokeWidth={2} />
          {notice}
        </p>
      )}
    </div>
  );
};

export default AskCard;
