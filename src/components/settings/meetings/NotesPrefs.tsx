import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "@/components/ui/Dropdown";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
import {
  AUTO_NOTES_LANGUAGE,
  getMeetingNotesPrefs,
  NOTES_LANGUAGES,
  setMeetingAutoNotes,
  setMeetingNotesLanguage,
  type MeetingNotesPrefs,
} from "./api";

/**
 * "Write notes when a call ends" and "Notes language".
 *
 * Read and written through `invoke` rather than `useSettings`, like the other
 * meeting switches (see `CallDetectionToggle`): `updateSetting` is an allowlist
 * keyed on the generated settings type, and a key added here would type-check on
 * a machine that regenerated `bindings.ts` and break a fresh checkout.
 *
 * `prefs` is null until the first read answers. Callers treat that as "notes
 * on", the default, so a page does not flash its notes-off layout on open.
 */
export const useMeetingNotesPrefs = () => {
  const [prefs, setPrefs] = useState<MeetingNotesPrefs | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void getMeetingNotesPrefs()
      .then((value) => {
        if (!cancelled) setPrefs(value);
      })
      // An older backend without the command behaves as before: notes on, auto.
      .catch(() => {
        if (!cancelled)
          setPrefs({ auto_notes: true, language: AUTO_NOTES_LANGUAGE });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  /** Optimistic, then reverted on failure, like every switch on this page. */
  const update = useCallback(
    (next: Partial<MeetingNotesPrefs>, write: () => Promise<unknown>) => {
      if (!prefs) return;
      const previous = prefs;
      setPrefs({ ...prefs, ...next });
      setSaving(true);
      void write()
        .catch(() => setPrefs(previous))
        .finally(() => setSaving(false));
    },
    [prefs],
  );

  const setAutoNotes = useCallback(
    (enabled: boolean) =>
      update({ auto_notes: enabled }, () => setMeetingAutoNotes(enabled)),
    [update],
  );
  const setLanguage = useCallback(
    (language: string) =>
      update({ language }, () => setMeetingNotesLanguage(language)),
    [update],
  );

  return { prefs, saving, setAutoNotes, setLanguage };
};

export interface NotesLanguageOption {
  value: string;
  label: string;
}

/**
 * The notes languages, named in the app's own language ("Deutsch" in a German
 * UI, "German" in an English one) by `Intl.DisplayNames`, so no language name
 * needs a translation of its own.
 *
 * Order: the meeting's own language first, because it is the default and the
 * right answer for most people; English next, because it is the most common
 * override; then everything else alphabetically as the reader would sort it.
 */
export const useNotesLanguageOptions = (): NotesLanguageOption[] => {
  const { t, i18n } = useTranslation();
  return useMemo(() => {
    let names: Intl.DisplayNames | null = null;
    try {
      names = new Intl.DisplayNames([i18n.language, "en"], {
        type: "language",
      });
    } catch {
      names = null;
    }
    const nameOf = (code: string) => {
      try {
        return names?.of(code) ?? code;
      } catch {
        return code;
      }
    };
    const [english, ...rest] = NOTES_LANGUAGES;
    const others = rest
      .map((code) => ({ value: code, label: nameOf(code) }))
      .sort((a, b) => a.label.localeCompare(b.label, i18n.language));
    return [
      { value: AUTO_NOTES_LANGUAGE, label: t("meetings.notesPrefs.auto") },
      { value: english, label: nameOf(english) },
      ...others,
    ];
  }, [t, i18n.language]);
};

/** "Write notes when a call ends", as a row in a settings group. */
export const AutoNotesToggle: React.FC<{
  model: ReturnType<typeof useMeetingNotesPrefs>;
}> = ({ model }) => {
  const { t } = useTranslation();
  if (!model.prefs) return null;
  return (
    <ToggleSwitch
      checked={model.prefs.auto_notes}
      onChange={model.setAutoNotes}
      isUpdating={model.saving}
      label={t("meetings.notesPrefs.autoTitle")}
      description={t("meetings.notesPrefs.autoDescription")}
      grouped={true}
    />
  );
};

/** "Notes language", as a row in a settings group. */
export const NotesLanguageSetting: React.FC<{
  model: ReturnType<typeof useMeetingNotesPrefs>;
}> = ({ model }) => {
  const { t } = useTranslation();
  const options = useNotesLanguageOptions();
  if (!model.prefs) return null;
  return (
    <SettingContainer
      title={t("meetings.notesPrefs.languageTitle")}
      description={t("meetings.notesPrefs.languageDescription")}
      layout="horizontal"
      grouped={true}
    >
      <Dropdown
        options={options}
        selectedValue={model.prefs.language}
        onSelect={model.setLanguage}
        className="min-w-[200px]"
      />
    </SettingContainer>
  );
};
