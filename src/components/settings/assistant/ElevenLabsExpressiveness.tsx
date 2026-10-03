import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { RotateCcw } from "lucide-react";
import { SettingContainer } from "@/components/ui/SettingContainer";

/** The slider moves in 5% steps; finer than that is not audible. */
const STEP = 0.05;

/** Where the thumb rests while the voice's own saved setting applies. Most
 *  ElevenLabs voices ship with stability 0.5, so the middle is the honest
 *  picture of "nothing overridden". */
const UNSET_POSITION = 0.5;

const round2 = (v: number) => Math.round(v * 100) / 100;

/** Expressiveness is stability turned around: more expressive = less stable. */
const toExpressiveness = (stability: number) => round2(1 - stability);
const toStability = (expressiveness: number) => round2(1 - expressiveness);

type LevelKey =
  | "verySteady"
  | "steady"
  | "balanced"
  | "lively"
  | "veryExpressive";

function levelFor(expressiveness: number): LevelKey {
  if (expressiveness < 0.2) return "verySteady";
  if (expressiveness < 0.4) return "steady";
  if (expressiveness <= 0.6) return "balanced";
  if (expressiveness <= 0.8) return "lively";
  return "veryExpressive";
}

interface ElevenLabsExpressivenessProps {
  /** Persisted ElevenLabs stability, or null/undefined for the voice default. */
  stability: number | null | undefined;
  /** Persist a stability (0–1), or null to return to the voice default. */
  onCommit: (stability: number | null) => Promise<unknown>;
}

/**
 * ElevenLabs' Stability, presented as Expressiveness.
 *
 * Unset means the request carries no stability at all, so the voice keeps the
 * settings saved with it on ElevenLabs; the row says "Voice default" rather
 * than inventing a number. Moving the thumb sets an override, and the reset
 * button clears it again. Like the other sliders, a drag is persisted once on
 * release, not on every tick.
 */
export const ElevenLabsExpressiveness: React.FC<
  ElevenLabsExpressivenessProps
> = ({ stability, onCommit }) => {
  const { t } = useTranslation();
  const isSet = typeof stability === "number" && Number.isFinite(stability);
  const persisted = isSet ? toExpressiveness(stability) : null;

  // The thumb's position during an uncommitted drag; null shows `persisted`.
  const [draft, setDraft] = useState<number | null>(null);
  const draftRef = useRef<number | null>(null);
  const persistedRef = useRef(persisted);
  persistedRef.current = persisted;
  const onCommitRef = useRef(onCommit);
  onCommitRef.current = onCommit;
  const inputRef = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  // Commits never overlap: one arriving mid-flight waits, and only the newest
  // waiting value is sent. `undefined` means nothing is waiting.
  const sending = useRef(false);
  const queued = useRef<number | null | undefined>(undefined);

  const setDraftValue = (next: number | null) => {
    draftRef.current = next;
    setDraft(next);
  };

  const send = async (next: number | null) => {
    if (sending.current) {
      queued.current = next;
      return;
    }
    sending.current = true;
    setBusy(true);
    try {
      await onCommitRef.current(next === null ? null : toStability(next));
    } catch {
      // The caller reports failures; the refreshed setting says what stuck.
    } finally {
      sending.current = false;
      setBusy(false);
      const waiting = queued.current;
      queued.current = undefined;
      if (waiting !== undefined && waiting !== next) {
        void send(waiting);
      } else if (draftRef.current === next || next === null) {
        // Hand the thumb back to the persisted value unless a newer drag
        // started while this one was saving.
        setDraftValue(null);
      }
    }
  };

  const commit = () => {
    const next = draftRef.current;
    if (next === null) return;
    if (!sending.current && next === persistedRef.current) {
      setDraftValue(null);
      return;
    }
    void send(next);
  };
  const commitRef = useRef(commit);
  commitRef.current = commit;

  // The native `change` fires once when a drag ends, including a release
  // outside the element, which `pointerup` can miss.
  useEffect(() => {
    const node = inputRef.current;
    if (!node) return;
    const onNativeChange = () => commitRef.current();
    node.addEventListener("change", onNativeChange);
    return () => node.removeEventListener("change", onNativeChange);
  }, []);

  const shown = draft ?? persisted ?? UNSET_POSITION;
  const showsDefault = draft === null && !isSet;
  const levelLabel = showsDefault
    ? t("settings.assistant.tts.expressivenessDefault")
    : t(`settings.assistant.tts.expressivenessLevels.${levelFor(shown)}`);
  const fill = shown * 100;

  return (
    <SettingContainer
      title={t("settings.assistant.tts.expressivenessLabel")}
      description={t("settings.assistant.tts.expressivenessDescription")}
      info={t("settings.assistant.tts.expressivenessInfo")}
      layout="horizontal"
      grouped={true}
    >
      <div className="flex items-center gap-3">
        <div className="w-[200px]">
          <input
            ref={inputRef}
            type="range"
            min={0}
            max={1}
            step={STEP}
            value={shown}
            onChange={(e) => setDraftValue(parseFloat(e.target.value))}
            onPointerUp={commit}
            onKeyUp={commit}
            onBlur={commit}
            aria-label={t("settings.assistant.tts.expressivenessLabel")}
            aria-valuetext={levelLabel}
            className="block h-2 w-full rounded-full appearance-none cursor-pointer focus:outline-none focus:ring-2 focus:ring-ink/20"
            style={{
              // An unset row draws an empty track: nothing is overridden yet,
              // so nothing is "filled in".
              background: showsDefault
                ? "var(--color-hairline)"
                : `linear-gradient(to right, var(--color-background-ui) ${fill}%, var(--color-hairline) ${fill}%)`,
            }}
          />
          <div
            aria-hidden="true"
            className="mt-1.5 flex justify-between text-[11px] leading-none text-muted-soft"
          >
            <span>{t("settings.assistant.tts.expressivenessSteady")}</span>
            <span>{t("settings.assistant.tts.expressivenessExpressive")}</span>
          </div>
        </div>
        <div className="flex w-[8.5rem] items-center justify-end gap-1 self-start pt-px">
          <span
            className={`truncate text-sm font-medium ${showsDefault ? "text-muted" : "text-ink"}`}
            aria-live="polite"
          >
            {levelLabel}
          </span>
          {isSet && (
            <button
              type="button"
              onClick={() => void send(null)}
              disabled={busy}
              title={t("settings.assistant.tts.expressivenessReset")}
              aria-label={t("settings.assistant.tts.expressivenessReset")}
              className="grid h-6 w-6 shrink-0 place-items-center rounded-md text-muted transition-colors hover:bg-surface-strong hover:text-ink cursor-pointer disabled:cursor-wait disabled:opacity-50"
            >
              <RotateCcw size={13} />
            </button>
          )}
        </div>
      </div>
    </SettingContainer>
  );
};
