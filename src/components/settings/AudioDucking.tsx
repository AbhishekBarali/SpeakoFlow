import React from "react";
import { useTranslation } from "react-i18next";
import { commands, type AudioDucking } from "@/bindings";
import { Segmented } from "../ui/Segmented";
import { SettingContainer } from "../ui/SettingContainer";
import { Slider } from "../ui/Slider";
import { useSettings } from "../../hooks/useSettings";

/** The slider's range and default, as `AUDIO_DUCKING_AMOUNT_MIN`/`_MAX` and
 *  `default_audio_ducking_amount` in settings.rs keep them. */
const MIN_AMOUNT = 10;
const MAX_AMOUNT = 90;
const DEFAULT_AMOUNT = 70;
/** How often a drag updates the preview: often enough to follow the thumb,
 *  rarely enough that the volume tools keep up. */
const PREVIEW_EVERY_MS = 150;

const MODES: AudioDucking[] = ["off", "lower", "mute"];

interface AudioDuckingProps {
  grouped?: boolean;
}

/**
 * What a recording does to the computer's other audio: leave it, lower it,
 * or mute it. Lower brings in a second row for how much, and dragging that
 * slider dips whatever is playing to the new level for a moment, so the
 * amount can be heard rather than guessed.
 */
export const AudioDuckingSetting: React.FC<AudioDuckingProps> = React.memo(
  ({ grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting } = useSettings();
    const mode = getSetting("audio_ducking") ?? "off";
    const amount = getSetting("audio_ducking_amount") ?? DEFAULT_AMOUNT;

    return (
      <>
        <SettingContainer
          title={t("settings.sound.ducking.label")}
          description={t("settings.sound.ducking.description")}
          grouped={grouped}
        >
          <Segmented
            label={t("settings.sound.ducking.ariaLabel")}
            value={mode}
            onChange={(next) => void updateSetting("audio_ducking", next)}
            options={MODES.map((value) => ({
              value,
              label: t(`settings.sound.ducking.options.${value}`),
            }))}
          />
        </SettingContainer>
        {mode === "lower" && (
          <Slider
            value={amount}
            onChange={(next) => {
              void commands.previewAudioDucking(next);
              return updateSetting("audio_ducking_amount", next);
            }}
            min={MIN_AMOUNT}
            max={MAX_AMOUNT}
            step={5}
            label={t("settings.sound.ducking.amount.label")}
            description={t("settings.sound.ducking.amount.description")}
            grouped={grouped}
            formatValue={(value) => `${Math.round(value)}%`}
            liveCommitMs={PREVIEW_EVERY_MS}
          />
        )}
      </>
    );
  },
);

AudioDuckingSetting.displayName = "AudioDuckingSetting";
