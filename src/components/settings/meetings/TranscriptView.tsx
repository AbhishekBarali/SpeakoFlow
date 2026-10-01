import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, HelpCircle, Pencil, X } from "lucide-react";
import type { SettingTone } from "@/components/ui/tones";
import type { MeetingSpeaker } from "./api";
import {
  formatClock,
  groupIntoTurns,
  otherSpeakerOrder,
  speakerInitial,
  speakerNameResolver,
  speakerTone,
  type SpeakerTurn,
  type TranscriptItem,
} from "./speakers";
import { useWindowedList } from "./useWindowedList";

interface TranscriptViewProps {
  items: readonly TranscriptItem[];
  speakers: readonly MeetingSpeaker[];
  /** Omitted while recording: renaming a speaker mid-call is churn, and the
   *  diarization pass that creates the real speakers has not run yet. */
  onRenameSpeaker?: (speakerKey: string, displayName: string) => void;
  /** Follow new turns to the bottom. The live view wants this; a transcript
   *  someone is reading does not. */
  stickToBottom?: boolean;
  /** Height classes for the scroll area — the live card and the detail tab want
   *  different ones. */
  heightClassName?: string;
  emptyLabel: string;
  /** Rendered under the last turn: a "loading more" line, usually. */
  footer?: React.ReactNode;
  /** Discards cached row heights when the underlying list is a different one. */
  resetKey?: string | number;
}

/** Rows are one line or twenty; this is only the first guess for a row that has
 *  never been on screen, and the scrollbar corrects as rows are measured. */
const ESTIMATED_TURN_HEIGHT = 104;

/**
 * The avatar's tint per speaker. The colour lives only here, on a small
 * circle: names are ordinary ink, so a transcript reads as text rather than as
 * a row of coloured badges — which is what the uppercase rainbow tags it
 * replaced looked like.
 */
const AVATAR: Record<SettingTone, string> = {
  teal: "bg-accent/12 text-accent",
  violet: "bg-violet-500/12 text-violet-700 dark:text-violet-300",
  amber: "bg-amber-500/14 text-amber-700 dark:text-amber-300",
  sky: "bg-sky-500/12 text-sky-700 dark:text-sky-300",
  rose: "bg-rose-500/12 text-rose-700 dark:text-rose-300",
  indigo: "bg-indigo-500/12 text-indigo-700 dark:text-indigo-300",
  emerald: "bg-emerald-500/12 text-emerald-700 dark:text-emerald-300",
};

/**
 * A meeting transcript: consecutive utterances grouped into speaker turns, one
 * colour per speaker, names editable in place.
 *
 * The list is windowed (`useWindowedList`) because an hour of speech is a
 * thousand-odd segments, and the DOM cost of that is felt long before the query
 * is.
 *
 * The two streams are never merged into one anonymous voice: every turn carries
 * the speaker key it was captured under, and `mic` versus `system` is what
 * decides whether it reads as the user or as the room.
 */
export const TranscriptView: React.FC<TranscriptViewProps> = ({
  items,
  speakers,
  onRenameSpeaker,
  stickToBottom = false,
  heightClassName = "max-h-[420px]",
  emptyLabel,
  footer,
  resetKey,
}) => {
  const { t } = useTranslation();
  const [editingKey, setEditingKey] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  const turns = useMemo(() => groupIntoTurns(items), [items]);
  const others = useMemo(
    () => otherSpeakerOrder(turns, speakers),
    [turns, speakers],
  );

  const nameFor = useMemo(
    () =>
      speakerNameResolver(speakers, others, {
        me: t("meetings.speakers.me"),
        others: t("meetings.speakers.others"),
        numbered: (number) => t("meetings.speakers.numbered", { number }),
      }),
    [speakers, others, t],
  );

  const {
    containerRef,
    totalHeight,
    items: windowed,
    measure,
  } = useWindowedList({
    count: turns.length,
    estimatedItemHeight: ESTIMATED_TURN_HEIGHT,
    resetKey,
  });

  // Follow the conversation while it is happening. Only on a new turn, so a
  // measurement settling does not yank the view.
  const turnCount = turns.length;
  useEffect(() => {
    if (!stickToBottom) return;
    const element = containerRef.current;
    if (!element) return;
    element.scrollTop = element.scrollHeight;
  }, [stickToBottom, turnCount, containerRef]);

  const startRename = (speakerKey: string) => {
    if (!onRenameSpeaker) return;
    setEditingKey(speakerKey);
    setDraft(nameFor(speakerKey));
  };

  const commitRename = () => {
    const name = draft.trim();
    if (editingKey && name) onRenameSpeaker?.(editingKey, name);
    setEditingKey(null);
  };

  if (turns.length === 0) {
    return (
      <div className="px-6 py-12 text-center text-sm text-muted">
        {emptyLabel}
      </div>
    );
  }

  return (
    <div ref={containerRef} className={`overflow-y-auto ${heightClassName}`}>
      <div className="px-5 pt-5 pb-2 sm:px-6">
        {/* The spacer holds the scrollbar at full length; rows are positioned
            inside it so only the visible ones exist in the DOM. */}
        <div className="relative" style={{ height: totalHeight }}>
          {windowed.map(({ index, offset }) => {
            const turn = turns[index];
            return (
              <div
                key={turn.key}
                ref={measure(index)}
                className="absolute inset-x-0"
                style={{ top: offset }}
              >
                <TurnRow
                  turn={turn}
                  name={nameFor(turn.speakerKey)}
                  tone={speakerTone(turn.speakerKey, others)}
                  editable={Boolean(onRenameSpeaker)}
                  editing={editingKey === turn.speakerKey}
                  draft={draft}
                  onDraftChange={setDraft}
                  onStartRename={() => startRename(turn.speakerKey)}
                  onCommitRename={commitRename}
                  onCancelRename={() => setEditingKey(null)}
                />
              </div>
            );
          })}
        </div>
        {footer}
      </div>
    </div>
  );
};

interface TurnRowProps {
  turn: SpeakerTurn;
  name: string;
  tone: SettingTone;
  editable: boolean;
  editing: boolean;
  draft: string;
  onDraftChange: (value: string) => void;
  onStartRename: () => void;
  onCommitRename: () => void;
  onCancelRename: () => void;
}

/** One speaker turn: an avatar, the name and when they started, then what
 *  they said as an ordinary paragraph. */
const TurnRow: React.FC<TurnRowProps> = ({
  turn,
  name,
  tone,
  editable,
  editing,
  draft,
  onDraftChange,
  onStartRename,
  onCommitRename,
  onCancelRename,
}) => {
  const { t } = useTranslation();
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  // A low-confidence label is a guess. It keeps the speaker's colour so the
  // transcript still scans, but says so rather than asserting a name the
  // diarizer was unsure about.
  const uncertain = turn.lowConfidence;

  return (
    <div className="flex gap-3.5 pb-6">
      <span
        className={`mt-px grid h-8 w-8 shrink-0 select-none place-items-center rounded-full text-[0.8125rem] font-semibold ${AVATAR[tone]} ${
          uncertain
            ? "opacity-60 outline-1 outline-offset-2 outline-dashed"
            : ""
        }`}
        aria-hidden="true"
      >
        {speakerInitial(name)}
      </span>

      <div className="min-w-0 flex-1">
        <div className="flex min-h-8 flex-wrap items-center gap-x-2.5 gap-y-1">
          {editing ? (
            <span className="flex items-center gap-1">
              <input
                ref={inputRef}
                value={draft}
                onChange={(event) => onDraftChange(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") onCommitRename();
                  if (event.key === "Escape") onCancelRename();
                }}
                aria-label={t("meetings.transcript.renameSpeaker")}
                className="h-8 w-44 rounded-lg border border-hairline-strong bg-surface px-2.5 text-sm text-ink focus:border-ink focus:outline-none"
              />
              <button
                type="button"
                onClick={onCommitRename}
                title={t("common.save")}
                aria-label={t("common.save")}
                className="grid h-8 w-8 cursor-pointer place-items-center rounded-lg text-muted transition-colors hover:bg-ink/6 hover:text-ink"
              >
                <Check className="h-4 w-4" />
              </button>
              <button
                type="button"
                onClick={onCancelRename}
                title={t("common.cancel")}
                aria-label={t("common.cancel")}
                className="grid h-8 w-8 cursor-pointer place-items-center rounded-lg text-muted transition-colors hover:bg-ink/6 hover:text-ink"
              >
                <X className="h-4 w-4" />
              </button>
            </span>
          ) : editable ? (
            <button
              type="button"
              onClick={onStartRename}
              title={t("meetings.transcript.renameSpeaker")}
              className={`group/name -mx-1.5 inline-flex cursor-pointer items-center gap-1.5 rounded-md px-1.5 py-0.5 text-sm font-semibold transition-colors hover:bg-ink/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${
                uncertain ? "text-muted" : "text-ink"
              }`}
            >
              {name}
              <Pencil
                className="h-3 w-3 text-muted-soft opacity-0 transition-opacity group-hover/name:opacity-100 group-focus-visible/name:opacity-100"
                aria-hidden="true"
              />
            </button>
          ) : (
            <span
              className={`text-sm font-semibold ${uncertain ? "text-muted" : "text-ink"}`}
            >
              {name}
            </span>
          )}
          <span className="text-xs tabular-nums text-muted-soft">
            {formatClock(turn.startMs)}
          </span>
          {uncertain && (
            <span
              className="inline-flex items-center gap-1 rounded-full border border-dashed border-hairline-strong px-2 py-px text-[0.6875rem] font-medium text-muted"
              title={t("meetings.transcript.lowConfidenceHelp")}
            >
              <HelpCircle className="h-3 w-3" aria-hidden="true" />
              {t("meetings.transcript.lowConfidence")}
            </span>
          )}
        </div>
        <p
          className={`mt-0.5 max-w-[72ch] whitespace-pre-wrap break-words text-[0.9375rem] leading-[1.7] select-text ${
            uncertain ? "text-muted" : "text-body"
          }`}
        >
          {turn.text}
        </p>
      </div>
    </div>
  );
};
