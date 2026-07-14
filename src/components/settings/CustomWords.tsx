import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, X } from "lucide-react";
import { toast } from "sonner";
import { useSettings } from "../../hooks/useSettings";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { SettingContainer } from "../ui/SettingContainer";

interface CustomWordsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const CustomWords: React.FC<CustomWordsProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
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

    const handleKeyPress = (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleAddWord();
      }
    };

    return (
      <SettingContainer
        title={t("settings.advanced.customWords.title")}
        description={t("settings.advanced.customWords.description")}
        info={t("settings.advanced.customWords.info")}
        descriptionMode={descriptionMode}
        grouped={grouped}
        layout="stacked"
      >
        <div className="space-y-2.5">
          <div className="flex items-center gap-2">
            <Input
              type="text"
              variant="compact"
              className="min-w-0 flex-1"
              value={newWord}
              maxLength={50}
              onChange={(e) => setNewWord(e.target.value)}
              onKeyDown={handleKeyPress}
              placeholder={t("settings.advanced.customWords.placeholder")}
              disabled={updating}
              aria-label={t("settings.advanced.customWords.placeholder")}
            />
            <Button
              onClick={handleAddWord}
              disabled={
                !newWord.trim() || newWord.trim().length > 50 || updating
              }
              variant="primary-soft"
              size="sm"
              className="shrink-0"
            >
              <Plus className="h-3.5 w-3.5" aria-hidden="true" />
              {t("settings.advanced.customWords.add")}
            </Button>
          </div>

          {customWords.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {customWords.map((word) => (
                <span
                  key={word}
                  className="inline-flex h-7 items-center gap-1 rounded-md bg-surface-strong ps-2.5 pe-1.5 text-xs font-medium text-ink"
                >
                  <span>{word}</span>
                  <button
                    type="button"
                    onClick={() => handleRemoveWord(word)}
                    disabled={updating}
                    className="rounded p-1 text-muted-soft transition-colors hover:bg-error/10 hover:text-error focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-50"
                    aria-label={t("settings.advanced.customWords.remove", {
                      word,
                    })}
                  >
                    <X className="h-3 w-3" aria-hidden="true" />
                  </button>
                </span>
              ))}
            </div>
          )}
        </div>
      </SettingContainer>
    );
  },
);

CustomWords.displayName = "CustomWords";
