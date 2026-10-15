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
 * Ends a tap-started dictation once it has heard no speech for a while, as if
 * its shortcut had been pressed again. A held recording already ends when the
 * key is let go, so with Hold the dial is disabled and says why, rather than
 * being a setting that does nothing.
 */
export const SilenceStop: React.FC<SilenceStopProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t, i18n } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const holdToTalk = getSetting("push_to_talk") ?? true;
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
        description={
          holdToTalk
            ? t("settings.general.silenceStop.tapOnly")
            : t("settings.general.silenceStop.description")
        }
        // A disabled control needs its reason in sight, not behind the (i).
        descriptionMode={holdToTalk ? "caption" : descriptionMode}
        grouped={grouped}
      >
        <Dropdown
          size="sm"
          options={options}
          selectedValue={holdToTalk ? "0" : String(current)}
          onSelect={(value) =>
            void updateSetting("dictation_silence_stop_secs", Number(value))
          }
          disabled={holdToTalk || isUpdating("dictation_silence_stop_secs")}
        />
      </SettingContainer>
    );
  },
);

SilenceStop.displayName = "SilenceStop";
