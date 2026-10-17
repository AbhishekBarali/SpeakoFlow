import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";

interface SilenceStopProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/** The waits on offer, in seconds. 0 is Off. */
const CHOICES = [0, 3, 5, 10, 20, 30];

/** "5 seconds" in the interface language, plural forms included, without a
 *  translation string per choice: Polish says "3 sekundy" but "5 sekund",
 *  which one "{{seconds}} seconds" string cannot. Same approach as a
 *  meeting's length (`formatDuration`). */
const secondsLabel = (seconds: number, locale: string) => {
  try {
    return new Intl.NumberFormat(locale, {
      style: "unit",
      unit: "second",
      unitDisplay: "long",
    }).format(seconds);
  } catch {
    return `${seconds} s`;
  }
};

/**
 * Ends a hands-free dictation once it has heard no speech for a while, as if
 * its shortcut had been pressed again. With Tap to toggle that is every
 * dictation; with Hold, one turned hands-free with the hands-free key, so the
 * dial applies in both. A dictation still held ends when the key is let go,
 * and is never stopped under the user's finger.
 */
export const SilenceStop: React.FC<SilenceStopProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t, i18n } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const current = getSetting("dictation_silence_stop_secs") ?? 0;
    // A value from outside the list (the settings file edited by hand) still
    // shows as itself instead of an empty dropdown.
    const choices = CHOICES.includes(current)
      ? CHOICES
      : [...CHOICES, current].sort((a, b) => a - b);
    const options = choices.map((seconds) => ({
      value: String(seconds),
      label:
        seconds === 0 ? t("common.off") : secondsLabel(seconds, i18n.language),
    }));

    return (
      <SettingContainer
        title={t("settings.general.silenceStop.label")}
        description={t("settings.general.silenceStop.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <Dropdown
          size="sm"
          options={options}
          selectedValue={String(current)}
          onSelect={(value) =>
            void updateSetting("dictation_silence_stop_secs", Number(value))
          }
          disabled={isUpdating("dictation_silence_stop_secs")}
        />
      </SettingContainer>
    );
  },
);

SilenceStop.displayName = "SilenceStop";
