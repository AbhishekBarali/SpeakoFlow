import React, { useEffect, useRef, useState } from "react";
import { SettingContainer } from "./SettingContainer";

interface SliderProps {
  value: number;
  /** Persists a value. Called when a drag or key press is committed, not on
   *  every tick of the drag (see `liveCommitMs`). A returned promise is awaited
   *  before the next commit is sent, so commits never overlap. */
  onChange: (value: number) => unknown;
  min: number;
  max: number;
  step?: number;
  disabled?: boolean;
  label: string;
  description?: string;
  /** Optional deep-dive help shown behind a small (i) icon, matching the
   *  dropdown rows (e.g. Panel size) so slider rows can carry the same hint. */
  info?: string;
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  showValue?: boolean;
  formatValue?: (value: number) => string;
  /** Tailwind width class for the control column. Defaults to full width. The
   *  assistant opacity row passes a fixed width so the slider lines up with the
   *  dropdown rows above it instead of running wider and starting further left. */
  controlClassName?: string;
  /** Tailwind classes for the value label. Defaults to a fixed `w-12`, which
   *  fits numbers and percentages; a slider that shows words (the audio-tag
   *  intensity) widens it so a long label, or a longer translation, does not
   *  spill past the row. */
  valueClassName?: string;
  /** For a setting whose effect is visible or audible elsewhere while it is
   *  dragged (the assistant's voice volume during a call): also commit during
   *  the drag, at most once per this many milliseconds. Omit to commit only
   *  when the drag ends. */
  liveCommitMs?: number;
}

/**
 * A settings slider.
 *
 * The thumb follows the pointer from a local draft, and the value is persisted
 * on release (pointer up, key up, the native `change`, or blur), the way the
 * Assistant page's opacity control already worked. Persisting every `input`
 * tick meant ~60 settings writes a second during a drag, each followed in the
 * callers by a full settings refetch in this window and, for assistant
 * settings, in the assistant panel as well.
 */
export const Slider: React.FC<SliderProps> = ({
  value,
  onChange,
  min,
  max,
  step = 0.01,
  disabled = false,
  label,
  description,
  info,
  descriptionMode = "tooltip",
  grouped = false,
  showValue = true,
  formatValue = (v) => v.toFixed(2),
  controlClassName = "w-full",
  valueClassName = "w-12",
  liveCommitMs,
}) => {
  // What the thumb shows while a drag is uncommitted (or its commit is still
  // on its way back as `value`); null means "show `value`".
  const [draft, setDraft] = useState<number | null>(null);
  const draftRef = useRef<number | null>(null);
  const valueRef = useRef(value);
  valueRef.current = value;
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const sending = useRef(false);
  const queued = useRef<number | null>(null);
  const lastSent = useRef<number | null>(null);
  const liveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const setDraftValue = (next: number | null) => {
    draftRef.current = next;
    setDraft(next);
  };

  /** Send one value; a value arriving while one is in flight waits for it,
   *  and only the newest waiting value is sent. */
  const send = (next: number) => {
    if (sending.current) {
      queued.current = next;
      return;
    }
    sending.current = true;
    lastSent.current = next;
    let result: unknown;
    try {
      result = onChangeRef.current(next);
    } catch {
      result = undefined;
    }
    void Promise.resolve(result)
      .catch(() => {})
      .finally(() => {
        sending.current = false;
        const waiting = queued.current;
        queued.current = null;
        if (waiting !== null && waiting !== next) {
          send(waiting);
          return;
        }
        // The committed value is now `value` (or was refused, and `value`
        // says what it really is); hand the thumb back unless a newer drag
        // has started since.
        if (draftRef.current === next) setDraftValue(null);
      });
  };

  const clearLiveTimer = () => {
    if (liveTimer.current !== null) {
      clearTimeout(liveTimer.current);
      liveTimer.current = null;
    }
  };

  const commit = () => {
    clearLiveTimer();
    const next = draftRef.current;
    if (next === null) return;
    if (sending.current) {
      queued.current = next;
      return;
    }
    // Nothing to persist: back where it started, and the backend already has
    // it (a drag that came back to its value, or a repeat of the last commit).
    if (
      next === valueRef.current &&
      (lastSent.current === null || lastSent.current === next)
    ) {
      setDraftValue(null);
      return;
    }
    send(next);
  };
  const commitRef = useRef(commit);
  commitRef.current = commit;

  // The native `change` fires once when a drag ends (and per keyboard step),
  // including a release outside the element, which `pointerup` can miss.
  useEffect(() => {
    const node = inputRef.current;
    if (!node) return;
    const onNativeChange = () => commitRef.current();
    node.addEventListener("change", onNativeChange);
    return () => node.removeEventListener("change", onNativeChange);
  }, []);

  // Never lose a drag the component did not live to commit.
  useEffect(
    () => () => {
      clearLiveTimer();
      const pending = draftRef.current;
      if (pending !== null && pending !== lastSent.current) {
        try {
          void Promise.resolve(onChangeRef.current(pending)).catch(() => {});
        } catch {
          // Nothing to recover on unmount.
        }
      }
    },
    [],
  );

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setDraftValue(parseFloat(e.target.value));
    if (liveCommitMs !== undefined && liveTimer.current === null) {
      liveTimer.current = setTimeout(() => {
        liveTimer.current = null;
        const next = draftRef.current;
        if (next !== null && next !== lastSent.current) send(next);
      }, liveCommitMs);
    }
  };

  const shown = draft ?? value;

  return (
    <SettingContainer
      title={label}
      description={description}
      info={info}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="horizontal"
      disabled={disabled}
    >
      <div className={controlClassName}>
        <div className="flex items-center space-x-1 h-6">
          <input
            ref={inputRef}
            type="range"
            min={min}
            max={max}
            step={step}
            value={shown}
            onChange={handleChange}
            onPointerUp={commit}
            onKeyUp={commit}
            onBlur={commit}
            disabled={disabled}
            className="flex-grow h-2 rounded-full appearance-none cursor-pointer focus:outline-none focus:ring-2 focus:ring-ink/20 disabled:opacity-50 disabled:cursor-not-allowed"
            style={{
              background: `linear-gradient(to right, var(--color-background-ui) ${
                ((shown - min) / (max - min)) * 100
              }%, var(--color-hairline) ${
                ((shown - min) / (max - min)) * 100
              }%)`,
            }}
          />
          {showValue && (
            <span
              className={`text-sm font-medium text-ink text-end ${valueClassName}`}
            >
              {formatValue(shown)}
            </span>
          )}
        </div>
      </div>
    </SettingContainer>
  );
};
