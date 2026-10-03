import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, X } from "lucide-react";
import { toast } from "sonner";
import { useSettings } from "../../hooks/useSettings";
import { Button } from "../ui/Button";

interface CustomWordsProps {
  /** Id for the input, so a page-level "Add word" button can focus it. */
  inputId?: string;
  /** @deprecated Layout is fixed now; kept for call sites. */
  descriptionMode?: "inline" | "tooltip";
  /** @deprecated See `descriptionMode`. */
  grouped?: boolean;
}

/**
 * The user's own words: an input to add one, and every word as a removable
 * chip. Sanitised the same way as before (no angle brackets or quotes, 50
 * characters), and duplicates are refused case-insensitively.
 */
export const CustomWords: React.FC<CustomWordsProps> = React.memo(
  ({ inputId = "dictionary-new-word" }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const [newWord, setNewWord] = useState("");
    const customWords = getSetting("custom_words") || [];
    const updating = isUpdating("custom_words");

    const handleAddWord = () => {
      const sanitizedWord = newWord.trim().replace(/[<>"']/g, "");
      if (!sanitizedWord || sanitizedWord.length > 50) return;

      const duplicate = customWords.some(
        (word) =>
          word.toLocaleLowerCase() === sanitizedWord.toLocaleLowerCase(),
      );
      if (duplicate) {
        toast.error(
          t("settings.advanced.customWords.duplicate", {
            word: sanitizedWord,
          }),
        );
        return;
      }

      void updateSetting("custom_words", [...customWords, sanitizedWord]);
      setNewWord("");
    };

    const handleRemoveWord = (wordToRemove: string) => {
      void updateSetting(
        "custom_words",
        customWords.filter((word) => word !== wordToRemove),
      );
    };

    return (
      <div className="overflow-hidden rounded-2xl border border-hairline bg-surface elev-card">
        <form
          className="flex items-center gap-2 border-b border-hairline px-3 py-3"
          onSubmit={(event) => {
            event.preventDefault();
            handleAddWord();
          }}
        >
          <input
            id={inputId}
            type="text"
            className="h-10 min-w-0 flex-1 rounded-lg bg-transparent px-2 text-[0.9375rem] text-ink placeholder:text-muted focus:outline-none"
            value={newWord}
            maxLength={50}
            onChange={(event) => setNewWord(event.target.value)}
            placeholder={t("dictionary.words.placeholder")}
            disabled={updating}
            aria-label={t("settings.advanced.customWords.placeholder")}
          />
          <Button
            type="submit"
            disabled={!newWord.trim() || newWord.trim().length > 50 || updating}
            size="sm"
          >
            <Plus className="h-3.5 w-3.5" aria-hidden="true" />
            {t("settings.advanced.customWords.add")}
          </Button>
        </form>

        {customWords.length === 0 ? (
          <p className="px-5 py-8 text-center text-sm text-muted">
            {t("dictionary.words.empty")}
          </p>
        ) : (
          <ul className="flex flex-wrap gap-2 p-4">
            {customWords.map((word) => (
              <li
                key={word}
                className="group inline-flex h-8 items-center gap-0.5 rounded-lg border border-hairline bg-surface-muted ps-3 pe-1 text-sm text-ink"
              >
                <span className="select-text">{word}</span>
                <button
                  type="button"
                  onClick={() => handleRemoveWord(word)}
                  disabled={updating}
                  className="grid h-6 w-6 cursor-pointer place-items-center rounded-md text-muted-soft transition-colors hover:bg-error/10 hover:text-error focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-50"
                  aria-label={t("settings.advanced.customWords.remove", {
                    word,
                  })}
                >
                  <X className="h-3.5 w-3.5" aria-hidden="true" />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    );
  },
);

CustomWords.displayName = "CustomWords";
