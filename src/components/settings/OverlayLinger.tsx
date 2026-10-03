import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import type { OverlayLinger as OverlayLingerValue } from "@/bindings";

interface OverlayLingerProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

const CHOICES: readonly OverlayLingerValue[] = [
  "quick",
  "standard",
  "long",
  "extended",
];

/**
 * How long the finished Live card stays on screen after a dictation, for
 * reading or copying the final text. Only the Live card lingers, so the caller
 * shows this row only when that is the overlay in use.
 */
export const OverlayLinger: React.FC<OverlayLingerProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const options = CHOICES.map((value) => ({
      value,
      label: t(`settings.advanced.overlayLinger.options.${value}`),
    }));
    const selected = (getSetting("overlay_linger") ??
      "quick") as OverlayLingerValue;

    return (
      <SettingContainer
        title={t("settings.advanced.overlayLinger.title")}
        description={t("settings.advanced.overlayLinger.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <Dropdown
          options={options}
          selectedValue={selected}
          onSelect={(value) =>
            updateSetting("overlay_linger", value as OverlayLingerValue)
          }
          disabled={isUpdating("overlay_linger")}
        />
      </SettingContainer>
    );
  },
);

OverlayLinger.displayName = "OverlayLinger";
