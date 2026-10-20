import React, { useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Globe } from "lucide-react";
import { Tooltip } from "@/components/ui/Tooltip";
import { uniqueLanguages } from "@/lib/languageCodes";
import { languageNames } from "@/components/onboarding/recommend";

/**
 * Which languages a speech model hears, on its row in the catalog.
 *
 * One language reads "English only". More than one reads as a count ("14
 * languages") that opens the full list on click. The row used to say
 * "Multi-language" for all of them, which put a 4-language Canary and a
 * 99-language Whisper side by side under the same word, and the list itself
 * was nowhere in the app.
 *
 * The list opens on a click rather than on hover: Whisper's 99 names make a
 * popover far too big to appear as the pointer passes over a row, and a click
 * also works from the keyboard. Escape or a click elsewhere closes it.
 *
 * Counting goes through `uniqueLanguages`, the canonical rule the language
 * picker, the Models page filter and onboarding all use, so `zh-Hans` and
 * `zh-Hant` are one language everywhere, and so are `nb` and `no`.
 */
export const ModelLanguages: React.FC<{
  codes: ReadonlyArray<string>;
  /** The model's display name, for the list's accessible name. */
  modelName: string;
}> = ({ codes, modelName }) => {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const listId = useId();

  const languages = useMemo(() => uniqueLanguages(codes), [codes]);
  // Alphabetical in the UI language: in a list of 99, alphabetical order is
  // what lets someone find their own language quickly.
  const names = useMemo(() => {
    const collator = new Intl.Collator(i18n.language);
    return languageNames(languages, i18n.language).sort(collator.compare);
  }, [languages, i18n.language]);

  useEffect(() => {
    if (!open) return;
    const onPointer = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        buttonRef.current?.contains(target) ||
        listRef.current?.contains(target)
      ) {
        return;
      }
      setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setOpen(false);
        buttonRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  if (languages.length === 0) return null;

  if (languages.length === 1) {
    return (
      <span className="inline-flex items-center gap-1">
        <Globe className="h-3.5 w-3.5" aria-hidden="true" />
        {t("modelSelector.capabilities.languageOnly", {
          language: names[0],
        })}
      </span>
    );
  }

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        aria-expanded={open}
        aria-controls={open ? listId : undefined}
        onClick={(event) => {
          event.stopPropagation();
          setOpen((value) => !value);
        }}
        className={`-mx-1 inline-flex cursor-pointer items-center gap-1 rounded px-1 underline decoration-dotted decoration-1 underline-offset-[3px] transition-colors hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${
          open ? "text-ink" : "decoration-ink/30"
        }`}
      >
        <Globe className="h-3.5 w-3.5" aria-hidden="true" />
        {t("onboarding.speech.languages", { count: languages.length })}
      </button>
      {open && (
        <Tooltip targetRef={buttonRef} position="bottom">
          <div
            ref={listRef}
            id={listId}
            role="dialog"
            aria-label={t("modelSelector.capabilities.languageList", {
              model: modelName,
            })}
          >
            <p className="mb-1.5 text-xs font-medium text-muted">
              {t("onboarding.speech.languages", { count: languages.length })}
            </p>
            {/* A grid, not CSS columns: columns in a box of fixed height add
                more columns off to the side instead of scrolling down, which
                left Whisper's list at "Danish" with no way to reach the rest. */}
            <ul className="grid max-h-56 grid-cols-2 gap-x-4 overflow-y-auto overscroll-contain pe-1 text-[0.8125rem] leading-relaxed text-body">
              {names.map((name) => (
                <li key={name}>{name}</li>
              ))}
            </ul>
          </div>
        </Tooltip>
      )}
    </>
  );
};
