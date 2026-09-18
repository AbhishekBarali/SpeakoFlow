import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Check, GraduationCap, X } from "lucide-react";
import { ToggleSwitch } from "../ui/ToggleSwitch";

/** Mirrors `AutoLearnStatus`. */
interface AutoLearnStatus {
  supported: boolean;
  enabled: boolean;
  learned: string[];
  max_learned: number;
}

/** A word was just learned. */
const LEARNED_EVENT = "autolearn-words-changed";

/*
 * Commands go through `invoke` rather than the generated `commands.*` wrappers, and
 * the status type is hand-mirrored, for the reason the meetings and reminder surfaces
 * do the same: `src/bindings.ts` is only regenerated while the app runs in dev, so a
 * fresh checkout would fail to type-check anything that depended on it existing.
 */
const getStatus = (): Promise<AutoLearnStatus> =>
  invoke<AutoLearnStatus>("get_auto_learn_status");

const setEnabled = (enabled: boolean): Promise<null> =>
  invoke<null>("set_auto_learn_corrections", { enabled });

const setWords = (words: string[]): Promise<null> =>
  invoke<null>("set_learned_words", { words });

const keepWord = (word: string): Promise<null> =>
  invoke<null>("keep_learned_word", { word });

interface AutoLearnProps {
  grouped?: boolean;
}

/**
 * "Learn words I correct."
 *
 * The switch is off by default and stays that way: turning it on means the app reads
 * the contents of the text field just dictated into, which is unlike anything else in
 * Settings and is not something to enable on someone's behalf. The description says
 * so rather than describing only the benefit.
 *
 * Learned words are listed here because a learned word is a **guess**. Each one can be
 * removed, or kept — which promotes it into the user's own dictionary, where it stops
 * being reviewable and stops counting against the cap. That review affordance is what
 * makes the underlying limitation acceptable: nothing can distinguish "the recogniser
 * misheard me and I fixed it" from "I typed a typo", so the user has to be able to see
 * what was inferred.
 */
export const AutoLearnCorrections: React.FC<AutoLearnProps> = ({
  grouped = false,
}) => {
  const { t } = useTranslation();
  const [status, setStatus] = useState<AutoLearnStatus | null>(null);
  const [saving, setSaving] = useState(false);

  const refresh = useCallback(() => {
    void getStatus()
      .then(setStatus)
      .catch(() => setStatus(null));
  }, []);

  useEffect(refresh, [refresh]);

  // A word can be learned at any moment, from a dictation into another app entirely,
  // so the list has to arrive rather than be polled.
  useEffect(() => {
    const unlisten = listen(LEARNED_EVENT, () => refresh());
    return () => {
      void unlisten.then((off) => off());
    };
  }, [refresh]);

  if (!status) return null;

  // Hidden rather than disabled where the field cannot be read: a switch that does
  // nothing is worse than no switch, and the note says what is missing.
  if (!status.supported) {
    return (
      <p className="px-1 text-xs text-muted-soft">
        {t("settings.advanced.autoLearn.unsupported")}
      </p>
    );
  }

  const remove = (word: string) => {
    const next = status.learned.filter((existing) => existing !== word);
    setStatus({ ...status, learned: next });
    void setWords(next).catch(refresh);
  };

  const keep = (word: string) => {
    setStatus({
      ...status,
      learned: status.learned.filter((existing) => existing !== word),
    });
    void keepWord(word).catch(refresh);
  };

  const clearAll = () => {
    setStatus({ ...status, learned: [] });
    void setWords([]).catch(refresh);
  };

  return (
    <div className="space-y-2">
      <ToggleSwitch
        checked={status.enabled}
        onChange={(next) => {
          setStatus({ ...status, enabled: next });
          setSaving(true);
          void setEnabled(next)
            .catch(() => setStatus({ ...status, enabled: !next }))
            .finally(() => setSaving(false));
        }}
        isUpdating={saving}
        label={t("settings.advanced.autoLearn.label")}
        description={t("settings.advanced.autoLearn.description")}
        info={t("settings.advanced.autoLearn.info")}
        descriptionMode="inline"
        icon={GraduationCap}
        tone="amber"
        grouped={grouped}
      />

      {status.enabled && (
        <div className="rounded-xl border border-hairline bg-surface-strong/50 px-3.5 py-3">
          <div className="mb-2 flex items-center justify-between gap-2">
            <p className="text-[12px] font-medium text-ink">
              {t("settings.advanced.autoLearn.learnedTitle")}
            </p>
            {status.learned.length > 0 && (
              <button
                type="button"
                onClick={clearAll}
                className="shrink-0 cursor-pointer text-[11.5px] text-muted underline hover:text-ink"
              >
                {t("settings.advanced.autoLearn.clearAll")}
              </button>
            )}
          </div>

          {status.learned.length === 0 ? (
            <p className="text-[11.5px] text-muted-soft">
              {t("settings.advanced.autoLearn.learnedEmpty")}
            </p>
          ) : (
            <>
              <div className="flex flex-wrap gap-1.5">
                {status.learned.map((word) => (
                  <span
                    key={word}
                    className="inline-flex items-center gap-1 rounded-md border border-hairline bg-surface px-2 py-1 text-[12px] text-ink"
                  >
                    {word}
                    <button
                      type="button"
                      onClick={() => keep(word)}
                      title={t("settings.advanced.autoLearn.keep")}
                      aria-label={t("settings.advanced.autoLearn.keep", {
                        word,
                      })}
                      className="cursor-pointer rounded p-0.5 text-muted-soft transition-colors hover:bg-ink/6 hover:text-teal-600"
                    >
                      <Check size={11} />
                    </button>
                    <button
                      type="button"
                      onClick={() => remove(word)}
                      title={t("settings.advanced.autoLearn.remove")}
                      aria-label={t("settings.advanced.autoLearn.remove", {
                        word,
                      })}
                      className="cursor-pointer rounded p-0.5 text-muted-soft transition-colors hover:bg-ink/6 hover:text-error"
                    >
                      <X size={11} />
                    </button>
                  </span>
                ))}
              </div>
              <p className="mt-2 text-[11px] text-muted-soft">
                {t("settings.advanced.autoLearn.reviewHint", {
                  max: status.max_learned,
                })}
              </p>
            </>
          )}
        </div>
      )}
    </div>
  );
};
