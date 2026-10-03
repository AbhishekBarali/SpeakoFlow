import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Mic, SquarePen, Volume2, X } from "lucide-react";
import AudioWaveform from "@/components/shared/AudioWaveform";
import { Keycaps } from "@/components/ui/Keycaps";
import QuickAsk from "@/assistant/QuickAsk";
import { VoiceOrb } from "@/assistant/VoiceOrb";
import { FONT_SIZES } from "@/assistant/appearance";
import type { ConversationPhase } from "@/assistant/useVoiceConversation";
import type { QuickAskPhase } from "@/assistant/quickAskState";
import "@/overlay/RecordingOverlay.css";
import "@/assistant/AssistantPanel.css";

/** A finished dictation, as the overlay ends one: the fill at the end. */
const FULL = () => 1;

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
 * Each one acts out the numbered steps listed beside it, and shows
 * everything the person does, not only what the app does in reply: the keys
 * appear on the stage while they are held, a pointer selects and clicks, what is
 * said is drawn as a subtitle, and the result lands in the app with a brief
 * highlight. The first version showed only the app's half, so a card simply
 * appeared and an answer filled it, and nothing said why.
 *
 * They sit on the same faux desktop the Assistant page uses
 * (`.assistant-desktop`), so the floating surfaces read as floating over your
 * work. Everything is inert: nothing records, sends or changes a setting.
 */

export type SceneId = "dictate" | "ask" | "call";

export interface SceneProps {
  /** Reduced motion: hold one representative frame. */
  still: boolean;
  holdToTalk: boolean;
  /** The shortcut being shown, drawn on the stage while it is pressed. */
  binding: string | null | undefined;
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

/** Plausible voice levels while `active`, for the waveforms and the orb. */
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

/** How long a tap reads as pressed: a real tap is shorter, but a viewer has to
 *  see it happen. */
const TAP_MS = 420;

/**
 * Whether the shortcut is down in this frame: held for as long as `down` in
 * hold mode, and in tap mode a short press each time `tap` names a new press.
 * Only the stage shows it: the copy column beside the preview stays still, so
 * the eye has one moving thing to follow.
 */
function usePressed(
  down: boolean,
  tap: string | null,
  hold: boolean,
  still: boolean,
) {
  const [pressed, setPressed] = useState(false);
  useEffect(() => {
    if (still) {
      setPressed(false);
      return;
    }
    if (hold) {
      setPressed(down);
      return;
    }
    if (tap === null) {
      setPressed(false);
      return;
    }
    setPressed(true);
    const timer = window.setTimeout(() => setPressed(false), TAP_MS);
    return () => window.clearTimeout(timer);
  }, [down, tap, hold, still]);
  return pressed;
}

/** The shortcut, drawn on the stage while it is down, the way a screencast
 *  shows keystrokes. The keys beside the preview press too, but the eye is on
 *  the preview, and this is what ties the press to what it does. */
const StageKeys: React.FC<{
  binding: string | null | undefined;
  pressed: boolean;
  /** Where on the stage is clear of the scene's own surfaces: a bottom or top
   *  corner, or centred just above the recording overlay (`overlay`). */
  corner?: "bottom" | "top" | "overlay";
}> = ({ binding, pressed, corner = "bottom" }) =>
  binding ? (
    <div
      className="ob-stage-keys"
      data-visible={pressed}
      data-corner={corner}
      aria-hidden="true"
    >
      <Keycaps binding={binding} size="sm" variant="hero" />
    </div>
  ) : null;

/* ------------------------------------------------------------------------ */
/* The pointer: the person's hand in the assistant preview.                  */
/* ------------------------------------------------------------------------ */

interface Pointer {
  x: number;
  y: number;
  /** Milliseconds the move to (x, y) takes; 0 jumps. */
  move: number;
  shape: "arrow" | "text";
  down: boolean;
  shown: boolean;
}

const HIDDEN_POINTER: Pointer = {
  x: 0,
  y: 0,
  move: 0,
  shape: "arrow",
  down: false,
  shown: false,
};

const FakePointer: React.FC<{ pointer: Pointer }> = ({ pointer }) => (
  <span
    className="ob-pointer"
    data-shape={pointer.shape}
    data-down={pointer.down}
    data-shown={pointer.shown}
    style={
      {
        transform: `translate(${pointer.x}px, ${pointer.y}px)`,
        "--ob-move": `${pointer.move}ms`,
      } as React.CSSProperties
    }
    aria-hidden="true"
  >
    <span className="ob-pointer-ripple" />
    {pointer.shape === "text" ? (
      <svg
        className="ob-pointer-text"
        width="16"
        height="22"
        viewBox="0 0 16 22"
      >
        <path
          d="M5 2.5h6M8 2.5v17M5 19.5h6"
          fill="none"
          stroke="#0d181c"
          strokeWidth="3.6"
          strokeLinecap="round"
        />
        <path
          d="M5 2.5h6M8 2.5v17M5 19.5h6"
          fill="none"
          stroke="#ffffff"
          strokeWidth="1.6"
          strokeLinecap="round"
        />
      </svg>
    ) : (
      <svg
        className="ob-pointer-arrow"
        width="20"
        height="24"
        viewBox="0 0 20 24"
      >
        <path
          d="M3 2.2v17.3l4.6-4.3 3 6.6 3-1.3-3-6.5h6.2z"
          fill="#ffffff"
          stroke="#0d181c"
          strokeWidth="1.4"
          strokeLinejoin="round"
        />
      </svg>
    )}
  </span>
);

type Point = { x: number; y: number };

/**
 * Where `target` is, in the scene's own coordinates. The scene is scaled down
 * to fit a small stage (`.ob-scene` in onboarding.css), and the pointer lives
 * inside it, so a screen position is divided back out by that scale.
 */
function pointIn(
  scene: HTMLElement | null,
  target: Element | null | undefined,
  where: "center" | "start" | "end" = "center",
): Point | null {
  if (!scene || !target) return null;
  const box = scene.getBoundingClientRect();
  const scale = scene.offsetWidth > 0 ? box.width / scene.offsetWidth : 1;
  const rects = target.getClientRects();
  const whole = target.getBoundingClientRect();
  const first = rects[0] ?? whole;
  const last = rects[rects.length - 1] ?? whole;
  const rect = where === "start" ? first : where === "end" ? last : whole;
  const x =
    where === "start"
      ? rect.left
      : where === "end"
        ? rect.right
        : rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  return { x: (x - box.left) / scale, y: (y - box.top) / scale };
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
/* Dictation: hold the keys, speak, let go, and it is typed.                 */
/* ------------------------------------------------------------------------ */

/** Waiting, the keys go down, you speak, you let go and it lands, a rest. */
const DICTATE_BEATS = [1000, 700, 2600, 900, 2400] as const;

export const DictateScene: React.FC<
  SceneProps & { compactOverlay: boolean }
> = ({ still, holdToTalk, binding, compactOverlay }) => {
  const { t } = useTranslation();
  const beat = useLoop(DICTATE_BEATS, still, 3);
  const recording = beat === 1 || beat === 2;
  const done = beat === 3;
  const landed = beat >= 3;
  const said = t("onboarding.tour.dictate.said");
  const live = useWords(said, beat === 2, 2300, still);
  const levels = useLevels(recording, still);
  // Hold: down for the whole recording, up when it lands. Tap: once to start,
  // once to stop.
  const pressed = usePressed(
    recording,
    beat === 1 ? "start" : beat === 3 ? "stop" : null,
    holdToTalk,
    still,
  );
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
          {landed ? (
            <span className={still ? undefined : "ob-ink-in"}>{said}</span>
          ) : (
            <span className="ob-caret" aria-hidden="true" />
          )}
        </p>
      </AppWindow>

      <div
        className="ob-overlay"
        data-compact={compactOverlay}
        data-visible={beat >= 1 && beat <= 3}
      >
        <div className="overlay-root" dir="ltr">
          {compactOverlay ? (
            <div className="overlay-pill">
              <div className="pill-wave">
                {done ? (
                  <span className="overlay-wave">
                    <AudioWaveform
                      barCount={14}
                      levels={levels}
                      size="sm"
                      active
                      mode="working"
                      progress={FULL}
                    />
                  </span>
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
                      <span className="overlay-wave">
                        <AudioWaveform
                          barCount={5}
                          pitch={5}
                          barWidth={3}
                          levels={levels}
                          size="sm"
                          active
                          mode="working"
                          progress={FULL}
                        />
                      </span>
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

      <div
        className="ob-dictate-keys"
        data-compact={compactOverlay}
        aria-hidden="true"
      >
        <StageKeys binding={binding} pressed={pressed} corner="overlay" />
      </div>
    </div>
  );
};

/* ------------------------------------------------------------------------ */
/* Assistant: select, ask out loud, and put the answer where it belongs.     */
/* ------------------------------------------------------------------------ */

/** What happens to the answer once it is there. */
type AskAction = "replace" | "insert" | "read";
type Example = {
  id: "translate" | "decline" | "explain";
  /** The ask is about selected text (otherwise the caret is in a reply). */
  selected: boolean;
  action: AskAction;
};
/** The asks the assistant preview cycles through, in order. */
export const ASK_EXAMPLE_IDS = ["translate", "decline", "explain"] as const;
const EXAMPLES: Example[] = [
  { id: "translate", selected: true, action: "replace" },
  { id: "decline", selected: false, action: "insert" },
  { id: "explain", selected: true, action: "read" },
];

type AskBeat =
  | "select"
  | "listen"
  | "think"
  | "answer"
  | "read"
  | "click"
  | "applied";
const ASK_BEAT_MS: Record<AskBeat, number> = {
  select: 1300,
  listen: 1800,
  think: 700,
  answer: 1300,
  read: 900,
  click: 700,
  applied: 1800,
};
/** An answer that is only read gets the time the others spend being used. */
const READ_ONLY_MS = 3000;

/** Every beat of every example, flattened so one loop drives the scene. */
const ASK_SEQUENCE = EXAMPLES.flatMap((example, index) => {
  const beats: AskBeat[] =
    example.action === "read"
      ? ["select", "listen", "think", "answer", "read"]
      : ["select", "listen", "think", "answer", "read", "click", "applied"];
  return beats.map((beat) => ({
    index,
    beat,
    ms:
      beat === "read" && example.action === "read"
        ? READ_ONLY_MS
        : ASK_BEAT_MS[beat],
  }));
});
const ASK_DURATIONS = ASK_SEQUENCE.map((entry) => entry.ms);
const ASK_STILL = ASK_SEQUENCE.findIndex(
  (entry) => entry.index === 0 && entry.beat === "read",
);
const PREVIEW_FONT = FONT_SIZES.medium;
const noop = () => {};

export const AskScene: React.FC<SceneProps> = ({
  still,
  holdToTalk,
  binding,
}) => {
  const { t } = useTranslation();
  const position = useLoop(ASK_DURATIONS, still, ASK_STILL);
  const { index, beat } = ASK_SEQUENCE[position];
  const example = EXAMPLES[index];
  const key = `onboarding.tour.ask.examples.${example.id}`;
  const question = t(`${key}.question`);
  const answer = t(`${key}.answer`);
  const doc = t(`${key}.doc`);
  const listening = beat === "listen";
  const answered = beat === "read" || beat === "click" || beat === "applied";
  const heard = useWords(question, listening, 1300, still || beat === "think");
  const streamed = useWords(answer, beat === "answer", 1100, still || answered);
  const levels = useLevels(listening, still);
  const pressed = usePressed(
    listening,
    listening ? `ask-${index}` : beat === "think" ? `sent-${index}` : null,
    holdToTalk,
    still,
  );

  const scene = useRef<HTMLDivElement>(null);
  const docText = useRef<HTMLSpanElement>(null);
  const reply = useRef<HTMLDivElement>(null);
  const [pointer, setPointer] = useState<Pointer>(HIDDEN_POINTER);
  // The selection grows under the pointer as it drags; the reply box gets its
  // caret when the pointer clicks into it; the primary button presses in.
  const [sweeping, setSweeping] = useState(false);
  const [focused, setFocused] = useState(false);
  const [pressing, setPressing] = useState(false);

  // The pointer's script for each beat. Positions are measured from the frame
  // that was just drawn, so they follow the real layout at any stage size.
  useEffect(() => {
    if (still) {
      setPointer(HIDDEN_POINTER);
      return;
    }
    const timers: number[] = [];
    const at = (ms: number, run: () => void) =>
      timers.push(window.setTimeout(run, ms));
    const update = (patch: Partial<Pointer>) =>
      setPointer((current) => ({ ...current, ...patch }));
    const moveTo = (point: Point | null, patch: Partial<Pointer>) => {
      if (point) update({ ...patch, x: point.x, y: point.y });
    };

    if (beat === "select") {
      setSweeping(false);
      setFocused(false);
      setPressing(false);
      if (example.selected) {
        // Drag across the text: press at its start, release at its end.
        const start = pointIn(scene.current, docText.current, "start");
        const end = pointIn(scene.current, docText.current, "end");
        moveTo(start, { move: 0, shape: "text", shown: false, down: false });
        at(80, () => update({ shown: true }));
        at(320, () => {
          moveTo(end, { move: 760, down: true });
          setSweeping(true);
        });
        at(1120, () => update({ down: false }));
      } else {
        // Click into the reply, arriving from above and to the right.
        const target = pointIn(scene.current, reply.current, "center");
        if (target) {
          moveTo(
            { x: target.x + 110, y: target.y - 80 },
            { move: 0, shape: "arrow", shown: false, down: false },
          );
        }
        at(80, () => update({ shown: true }));
        at(160, () => moveTo(target, { move: 620 }));
        at(820, () => {
          update({ down: true });
          setFocused(true);
        });
        at(1000, () => update({ down: false }));
      }
    } else if (beat === "listen") {
      // Off the text it just selected, so the selection stays readable.
      setPointer((current) => ({
        ...current,
        x: current.x + 14,
        y: current.y + 18,
        move: 360,
        shape: "arrow",
        down: false,
      }));
    } else if (beat === "click") {
      const button = scene.current?.querySelector(".qa-btn.primary");
      moveTo(pointIn(scene.current, button, "center"), {
        move: 440,
        shape: "arrow",
        shown: true,
      });
      at(480, () => {
        update({ down: true });
        setPressing(true);
      });
    } else if (beat === "applied") {
      setPressing(false);
      update({ down: false });
      at(700, () => update({ shown: false }));
    } else if (beat === "read" && example.action === "read") {
      at(2000, () => update({ shown: false }));
    }
    return () => timers.forEach((timer) => window.clearTimeout(timer));
  }, [beat, index, still, example.selected, example.action]);

  const phase: QuickAskPhase | null = listening
    ? "listening"
    : beat === "think"
      ? "working"
      : beat === "answer"
        ? "answering"
        : beat === "read" || beat === "click"
          ? "done"
          : null;
  const applied = beat === "applied";
  const selectionShown =
    example.selected && (still || beat !== "select" || sweeping);

  return (
    <div className="ob-scene" ref={scene}>
      <AppWindow title={t(`${key}.window`)} className="ob-scene-window is-low">
        <p className="text-[0.9375rem] leading-relaxed text-ink">
          {applied && example.action === "replace" ? (
            <span key={`replaced-${index}`} className="ob-ink-in">
              {answer}
            </span>
          ) : (
            <span
              ref={docText}
              className={example.selected ? "ob-sweep" : undefined}
              data-on={selectionShown}
              data-instant={still}
            >
              {doc}
            </span>
          )}
        </p>
        {!example.selected && (
          <div ref={reply} className="ob-reply">
            {applied ? (
              <span className="ob-ink-in">{answer}</span>
            ) : focused || beat !== "select" ? (
              <span className="ob-caret" aria-hidden="true" />
            ) : null}
          </div>
        )}
      </AppWindow>

      {(listening || beat === "think") && heard && (
        <p
          className={`ob-caption ob-ask-caption${beat === "think" ? " is-sent" : ""}`}
        >
          <span className="ob-caption-who">
            {t("onboarding.tour.call.you")}
          </span>
          <span>{heard}</span>
        </p>
      )}

      <div
        className="assistant-scope ob-ask"
        data-press={pressing}
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
            question={question}
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

      <StageKeys binding={binding} pressed={pressed} corner="top" />
      <FakePointer pointer={pointer} />
    </div>
  );
};

/* ------------------------------------------------------------------------ */
/* Call: press once, talk, it talks back, press again to hang up.            */
/* ------------------------------------------------------------------------ */

/** The keys start it, you speak, it thinks, it answers out loud, the keys hang
 *  up, a moment of empty desktop. */
const CALL_BEATS = [1300, 2600, 1000, 3400, 900, 600] as const;
const CALL_PHASE: ConversationPhase[] = [
  "listening",
  "hearing",
  "responding",
  "speaking",
  "listening",
  "listening",
];
export const CallScene: React.FC<SceneProps> = ({ still, binding }) => {
  const { t } = useTranslation();
  const beat = useLoop(CALL_BEATS, still, 3);
  const phase = CALL_PHASE[beat];
  const ending = beat === 4;
  const gone = beat === 5;
  // The orb follows the voice either way: yours while you speak, its while it
  // answers.
  const levels = useLevels(phase === "hearing" || phase === "speaking", still);
  const level = levels.length
    ? Math.max(...levels)
    : still && phase === "speaking"
      ? 0.55
      : 0;
  // A call is one tap to start and one to hang up, whatever the dictation
  // setting is.
  const pressed = usePressed(
    false,
    beat === 0 ? "start" : beat === 4 ? "end" : null,
    false,
    still,
  );
  const bubble =
    beat === 0
      ? { kind: "hint", text: t("assistant.conversation.startHint") }
      : ending
        ? null
        : {
            kind: "status",
            text: t(`assistant.conversation.phase.${phase}`),
          };
  const heard = useWords(
    t("onboarding.tour.call.said"),
    phase === "hearing",
    2000,
    still || (beat > 1 && beat < 5),
  );
  const reply = useWords(
    t("onboarding.tour.call.reply"),
    phase === "speaking" && !ending,
    2600,
    still || ending,
  );

  return (
    <div className="ob-scene ob-scene-call">
      {!gone && (
        <>
          <div className="ob-captions" data-leaving={ending} aria-hidden="true">
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
          <div className="assistant-scope ob-call" data-leaving={ending}>
            <div className="call-stack">
              {bubble ? (
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
              ) : (
                <div className="ob-call-bubble-space" />
              )}
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
        </>
      )}
      <StageKeys binding={binding} pressed={pressed} />
    </div>
  );
};

/** The faux desktop every preview sits on. Inert and hidden from assistive
 *  technology: the steps beside it say the same thing in words. */
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
