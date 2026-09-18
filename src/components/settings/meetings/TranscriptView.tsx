import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, HelpCircle, X } from "lucide-react";
import { TONE_PILL, type SettingTone } from "@/components/ui/tones";
import type { MeetingSpeaker } from "./api";
import {
  formatClock,
  groupIntoTurns,
  otherSpeakerOrder,
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
const ESTIMATED_TURN_HEIGHT = 92;

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
      <div className="px-4 py-10 text-center text-[13px] text-muted">
        {emptyLabel}
      </div>
    );
  }

  return (
    <div ref={containerRef} className={`overflow-y-auto ${heightClassName}`}>
      <div className="px-4 py-3">
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

/** One speaker turn: a coloured name above the words they said. */
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

  return (
    <div className="pb-4">
      <div className="mb-1 flex items-center gap-2">
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
              className="w-40 rounded-md border border-hairline-strong bg-surface px-2 py-1 text-[12px] text-ink focus:border-ink focus:outline-none"
            />
            <button
              type="button"
              onClick={onCommitRename}
              title={t("common.save")}
              aria-label={t("common.save")}
              className="cursor-pointer rounded-md p-1 text-muted transition-colors hover:bg-ink/6 hover:text-ink"
            >
              <Check size={13} />
            </button>
            <button
              type="button"
              onClick={onCancelRename}
              title={t("common.cancel")}
              aria-label={t("common.cancel")}
              className="cursor-pointer rounded-md p-1 text-muted transition-colors hover:bg-ink/6 hover:text-ink"
            >
              <X size={13} />
            </button>
          </span>
        ) : (
          <button
            type="button"
            onClick={onStartRename}
            disabled={!editable}
            title={
              editable ? t("meetings.transcript.renameSpeaker") : undefined
            }
            className={`rounded-md border px-2 py-0.5 text-[10.5px] font-semibold uppercase tracking-wide transition-colors ${
              // A low-confidence label is a guess. It keeps the speaker's colour
              // so the transcript still scans, but says so rather than asserting
              // a name the diarizer was unsure about.
              turn.lowConfidence
                ? "border-dashed border-hairline-strong bg-surface-strong/60 text-muted"
                : TONE_PILL[tone]
            } ${editable ? "cursor-pointer hover:opacity-80" : "cursor-default"}`}
          >
            {name}
          </button>
        )}
        {turn.lowConfidence && (
          <span
            className="flex items-center gap-1 text-[10.5px] text-muted-soft"
            title={t("meetings.transcript.lowConfidenceHelp")}
          >
            <HelpCircle size={11} />
            {t("meetings.transcript.lowConfidence")}
          </span>
        )}
        <span className="text-[10.5px] tabular-nums text-muted-soft">
          {formatClock(turn.startMs)}
        </span>
      </div>
      <p
        className={`whitespace-pre-wrap break-words text-[13px] leading-relaxed ${
          turn.lowConfidence ? "text-muted" : "text-body"
        }`}
      >
        {turn.text}
      </p>
    </div>
  );
};
