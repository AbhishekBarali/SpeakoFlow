import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Mic, SquarePen, Volume2, X } from "lucide-react";
import AudioWaveform from "@/components/shared/AudioWaveform";
import CompletionMark from "@/overlay/CompletionMark";
import QuickAsk from "@/assistant/QuickAsk";
import { VoiceOrb } from "@/assistant/VoiceOrb";
import { FONT_SIZES } from "@/assistant/appearance";
import type { ConversationPhase } from "@/assistant/useVoiceConversation";
import type { QuickAskPhase } from "@/assistant/quickAskState";
import "@/overlay/RecordingOverlay.css";
import "@/assistant/AssistantPanel.css";

/*
 * The tour's three previews.
 *
 * Each is drawn with the surface it is about, not a picture of one: the
 * dictation preview renders the recording overlay's own card markup from
 * `RecordingOverlay.css`, the assistant preview renders `QuickAsk` itself (as
 * the Assistant page's panel preview does), and the call preview renders the
 * call bar's classes from `CallBar.css` with the real `VoiceOrb`. The previous
 * tour drew look-alikes — a teal ring for the call, a dark card for the answer
 * — which is how it came to show a call screen the app does not have.
 *
 * They sit on the same faux desktop the Assistant page uses
 * (`.assistant-desktop`), so the floating surfaces read as floating over your
 * work. Everything is inert: nothing records, sends or changes a setting.
 */

export type SceneId = "dictate" | "ask" | "call";

export interface SceneProps {
  /** Reduced motion: hold one representative frame. */
  still: boolean;
  /** Called with whether the shortcut is "down" in this frame, so the keys in
   *  the copy column can press along with the preview. */
  onPressed: (pressed: boolean) => void;
  holdToTalk: boolean;
}

/** Step through `durations` in a loop; the index is the current beat. Under
 *  reduced motion the scene rests on `stillBeat`. */
function useLoop(
  durations: readonly number[],
  still: boolean,
  stillBeat: number,
) {
  const [beat, setBeat] = useState(still ? stillBeat : 0);
  useEffect(() => {
    if (still) {
      setBeat(stillBeat);
      return;
    }
    let index = 0;
    let timer: number;
    const next = () => {
      timer = window.setTimeout(() => {
        index = (index + 1) % durations.length;
        setBeat(index);
        next();
      }, durations[index]);
    };
    setBeat(0);
    next();
    return () => window.clearTimeout(timer);
  }, [durations, still, stillBeat]);
  return beat;
}

/** Reveal `text` a word at a time while `running`, over `duration` ms. */
function useWords(
  text: string,
  running: boolean,
  duration: number,
  full: boolean,
) {
  const words = useMemo(() => text.split(/\s+/).filter(Boolean), [text]);
  const [count, setCount] = useState(full ? words.length : 0);
  useEffect(() => {
    if (full) {
      setCount(words.length);
      return;
    }
    if (!running) {
      setCount(0);
      return;
    }
    setCount(1);
    const step = Math.max(40, duration / Math.max(1, words.length));
    const timer = window.setInterval(
      () =>
        setCount((value) => {
          if (value >= words.length) {
            window.clearInterval(timer);
            return value;
          }
          return value + 1;
        }),
      step,
    );
    return () => window.clearInterval(timer);
  }, [running, full, words, duration]);
  return words.slice(0, count).join(" ");
}

/** Plausible microphone levels while `active`, for the waveforms. */
function useLevels(active: boolean, still: boolean) {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (!active || still) return;
    const timer = window.setInterval(() => setTick((value) => value + 1), 70);
    return () => window.clearInterval(timer);
  }, [active, still]);
  return useMemo(() => {
    if (!active) return [];
    const loud = 0.35 + 0.55 * Math.abs(Math.sin(tick / 2.6));
    return Array.from({ length: 16 }, (_, i) =>
      Math.max(0, Math.min(1, loud * (0.7 + 0.3 * Math.sin(i * 1.9 + tick)))),
    );
  }, [active, tick]);
}

/** Tell the copy column when the shortcut is held (hold mode) or tapped. */
function usePress(
  down: boolean,
  tapAt: boolean,
  hold: boolean,
  still: boolean,
  onPressed: SceneProps["onPressed"],
) {
  useEffect(() => {
    if (still) {
      onPressed(false);
      return;
    }
    if (hold) {
      onPressed(down);
      return;
    }
    if (!tapAt) {
      onPressed(false);
      return;
    }
    onPressed(true);
    const timer = window.setTimeout(() => onPressed(false), 200);
    return () => window.clearTimeout(timer);
  }, [down, tapAt, hold, still, onPressed]);
}

/** A window on the faux desktop: a title bar and a page. */
const AppWindow: React.FC<{
  title: string;
  children: React.ReactNode;
  className?: string;
}> = ({ title, children, className = "" }) => (
  <div
    className={`ob-window overflow-hidden rounded-xl border border-hairline bg-surface text-ink ${className}`}
  >
    <div className="flex items-center gap-3 border-b border-hairline bg-surface-muted px-4 py-2.5">
      <span className="flex gap-1.5" aria-hidden="true">
        <i className="h-2.5 w-2.5 rounded-full bg-hairline-strong" />
        <i className="h-2.5 w-2.5 rounded-full bg-hairline-strong" />
        <i className="h-2.5 w-2.5 rounded-full bg-hairline-strong" />
      </span>
      <span className="truncate text-xs font-medium text-muted">{title}</span>
    </div>
    <div className="px-5 py-4">{children}</div>
  </div>
);

/* ------------------------------------------------------------------------ */
/* Dictation: the overlay's live card, then the words in the field.          */
/* ------------------------------------------------------------------------ */

const DICTATE_BEATS = [900, 900, 2600, 2800] as const;

export const DictateScene: React.FC<
  SceneProps & { compactOverlay: boolean }
> = ({ still, onPressed, holdToTalk, compactOverlay }) => {
  const { t } = useTranslation();
  const beat = useLoop(DICTATE_BEATS, still, 3);
  const recording = beat === 1 || beat === 2;
  const done = beat === 3;
  const said = t("onboarding.tour.dictate.said");
  const live = useWords(said, beat === 2, 2300, still);
  const levels = useLevels(recording, still);
  usePress(recording, beat === 1 || beat === 3, holdToTalk, still, onPressed);
  const words = live.split(" ");
  // The last word or two are still the engine's guess, as they are live.
  const committed = words.slice(0, Math.max(0, words.length - 2)).join(" ");
  const tentative = words.slice(Math.max(0, words.length - 2)).join(" ");

  return (
    <div className="ob-scene">
      <AppWindow
        title={t("onboarding.tour.mail.window")}
        className="ob-scene-window"
      >
        <p className="border-b border-hairline pb-2.5 text-[0.8125rem] text-muted">
          {t("onboarding.tour.mail.to")}{" "}
          <span className="text-ink">{t("onboarding.tour.mail.toValue")}</span>
        </p>
        <p className="min-h-[4.5rem] pt-3 text-[0.9375rem] leading-relaxed text-ink">
          {done ? (
            <span className="ob-landed">{said}</span>
          ) : (
            <span className="ob-caret" aria-hidden="true" />
          )}
        </p>
      </AppWindow>

      <div
        className="ob-overlay"
        data-compact={compactOverlay}
        data-visible={beat !== 0}
      >
        <div className="overlay-root" dir="ltr">
          {compactOverlay ? (
            <div className="overlay-pill">
              <div className="pill-wave">
                {done ? (
                  <CompletionMark label={t("overlay.done")} />
                ) : (
                  <span className="overlay-wave">
                    <AudioWaveform
                      barCount={14}
                      levels={levels}
                      size="sm"
                      active
                      mode="reactive"
                    />
                  </span>
                )}
              </div>
            </div>
          ) : (
            <div
              className={`overlay-card recording${done ? " completed" : ""}`}
            >
              <div className="card-header">
                <div className="card-status">
                  <div className="card-wave">
                    {done ? (
                      <CompletionMark label={t("overlay.done")} />
                    ) : (
                      <span className="overlay-wave">
                        <AudioWaveform
                          barCount={5}
                          pitch={5}
                          barWidth={3}
                          levels={levels}
                          size="sm"
                          active
                          mode="reactive"
                        />
                      </span>
                    )}
                  </div>
                  {!done && (
                    <span className="card-label">{t("overlay.recording")}</span>
                  )}
                </div>
                <div className="card-actions" />
              </div>
              <div className="card-body">
                {done || live ? (
                  <p className="card-text">
                    <span className="transcript-line">
                      <span>{done ? said : committed}</span>
                      {!done && tentative && (
                        <span className="transcript-tentative">
                          {committed ? " " : ""}
                          {tentative}
                        </span>
                      )}
                    </span>
                  </p>
                ) : (
                  <p className="card-text card-hint">
                    {t("overlay.listening")}
                  </p>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

/* ------------------------------------------------------------------------ */
/* Assistant: the quick ask itself, answering a few different kinds of ask.  */
/* ------------------------------------------------------------------------ */

type Example = { id: "translate" | "decline" | "explain"; selected: boolean };
/** The asks the assistant preview cycles through, in order. */
export const ASK_EXAMPLE_IDS = ["translate", "decline", "explain"] as const;
const EXAMPLES: Example[] = [
  { id: "translate", selected: true },
  { id: "decline", selected: false },
  { id: "explain", selected: true },
];
/** Per example: select, listen, think, answer, rest on the answer. */
const ASK_BEAT = [1000, 1500, 900, 1500, 3000] as const;
const ASK_BEATS = EXAMPLES.flatMap(() => ASK_BEAT);
const PREVIEW_FONT = FONT_SIZES.medium;
const noop = () => {};

export const AskScene: React.FC<
  SceneProps & { onExample?: (index: number) => void }
> = ({ still, onPressed, holdToTalk, onExample }) => {
  const { t } = useTranslation();
  const step = useLoop(ASK_BEATS, still, ASK_BEAT.length - 1);
  const exampleIndex = Math.floor(step / ASK_BEAT.length);
  const example = EXAMPLES[exampleIndex];
  useEffect(() => onExample?.(exampleIndex), [exampleIndex, onExample]);
  const beat = step % ASK_BEAT.length;
  const key = `onboarding.tour.ask.examples.${example.id}`;
  const answer = t(`${key}.answer`);
  const streamed = useWords(answer, beat === 3, 1300, still || beat === 4);
  const levels = useLevels(beat === 1, still);
  usePress(beat === 1, beat === 1 || beat === 2, holdToTalk, still, onPressed);
  const phase: QuickAskPhase | null =
    beat === 1
      ? "listening"
      : beat === 2
        ? "working"
        : beat === 3
          ? "answering"
          : beat === 4
            ? "done"
            : null;
  const doc = t(`${key}.doc`);

  return (
    <div className="ob-scene">
      <AppWindow title={t(`${key}.window`)} className="ob-scene-window is-low">
        <p className="text-[0.9375rem] leading-relaxed text-ink">
          {example.selected ? <mark className="ob-selection">{doc}</mark> : doc}
        </p>
      </AppWindow>
      <div
        className="assistant-scope ob-ask"
        style={
          {
            "--as-msg-font": PREVIEW_FONT,
            "--as-alpha": "1",
          } as React.CSSProperties
        }
      >
        {phase && (
          <QuickAsk
            phase={phase}
            status={
              phase === "listening"
                ? t("assistant.status.listening")
                : t("assistant.status.thinking")
            }
            levels={levels}
            question={t(`${key}.question`)}
            answer={phase === "done" ? answer : streamed}
            selectionChars={example.selected ? doc.length : 0}
            screen={false}
            error={null}
            notice={null}
            canRetry={phase === "done"}
            input=""
            onInputChange={noop}
            onSubmit={noop}
            onClose={noop}
            onCancel={noop}
            onStop={noop}
            onRetry={noop}
            onInsert={async () => false}
          />
        )}
      </div>
    </div>
  );
};

/* ------------------------------------------------------------------------ */
/* Call: the call's status bubble and bar, with what is being said.          */
/* ------------------------------------------------------------------------ */

/** Waiting, you speak, it thinks, it answers out loud. */
const CALL_BEATS = [1300, 2600, 1100, 3400] as const;
const CALL_PHASE: ConversationPhase[] = [
  "listening",
  "hearing",
  "responding",
  "speaking",
];

export const CallScene: React.FC<SceneProps> = ({ still, onPressed }) => {
  const { t } = useTranslation();
  const beat = useLoop(CALL_BEATS, still, 3);
  const phase = CALL_PHASE[beat];
  const levels = useLevels(phase === "hearing", still);
  const level = levels.length ? Math.max(...levels) : 0;
  // A call is one tap to start and one to hang up, whatever the dictation
  // setting is, so the keys tap once as the scene opens.
  usePress(false, beat === 0, false, still, onPressed);
  const bubble =
    phase === "listening"
      ? { kind: "hint", text: t("assistant.conversation.startHint") }
      : {
          kind: "status",
          text: t(`assistant.conversation.phase.${phase}`),
        };
  const heard = useWords(
    t("onboarding.tour.call.said"),
    phase === "hearing",
    2000,
    still || beat > 1,
  );
  const reply = useWords(
    t("onboarding.tour.call.reply"),
    phase === "speaking",
    2600,
    still,
  );

  return (
    <div className="ob-scene ob-scene-call">
      <div className="ob-captions" aria-hidden="true">
        {heard && (
          <p className="ob-caption">
            <span className="ob-caption-who">
              {t("onboarding.tour.call.you")}
            </span>
            <span>{heard}</span>
          </p>
        )}
        {reply && (
          <p className="ob-caption is-reply">
            <span className="ob-caption-who">
              <Volume2 className="h-3.5 w-3.5" />
              {t("onboarding.tour.call.assistant")}
            </span>
            <span>{reply}</span>
          </p>
        )}
      </div>
      <div className="assistant-scope ob-call">
        <div className="call-stack">
          <div className={`call-bubble ${bubble.kind}`}>
            <span className="call-bubble-open">
              {phase === "responding" && (
                <Loader2 size={13} className="call-spin" />
              )}
              {phase === "hearing" && (
                <span
                  className="call-level"
                  style={{ "--level": level } as React.CSSProperties}
                >
                  <span />
                  <span />
                  <span />
                </span>
              )}
              <span>{bubble.text}</span>
            </span>
          </div>
          <div className="call-bar-slot">
            <div className="call-bar">
              <span className="call-icon">
                <SquarePen />
              </span>
              <span className="call-icon">
                <Mic />
              </span>
              <span className="call-divider" />
              <span className="call-orb">
                <VoiceOrb phase={phase} level={level} />
                <span className="call-orb-end">
                  <X size={15} strokeWidth={2.6} />
                </span>
              </span>
              <span className="call-divider" />
              <span className="call-icon">
                <Volume2 />
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

/** The faux desktop every preview sits on. Inert and hidden from assistive
 *  technology: the copy beside it says the same thing in words. */
export const Stage: React.FC<{ scene: SceneId; children: React.ReactNode }> = ({
  scene,
  children,
}) => {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.setAttribute("inert", "");
  }, []);
  return (
    <div
      ref={ref}
      className="ob-stage assistant-desktop"
      data-scene={scene}
      aria-hidden="true"
    >
      {children}
    </div>
  );
};
