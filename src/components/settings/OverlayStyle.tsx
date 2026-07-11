import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import { useModelStore } from "../../stores/modelStore";
import type { OverlayStyle as OverlayStyleValue } from "@/bindings";

interface OverlayStyleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Dictation-overlay style selector: None / Minimal / Live (Handy-style).
 * The stored value can also be "auto" (the default), which follows the model —
 * Live when the selected model supports live streaming, otherwise Minimal — so
 * we resolve it to a concrete option for display. Picking any option writes a
 * concrete value, overriding auto.
 */
export const OverlayStyle: React.FC<OverlayStyleProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const models = useModelStore((s) => s.models);
    const currentModel = useModelStore((s) => s.currentModel);

    const supportsLive =
      models.find((m) => m.id === currentModel)?.supports_streaming ?? false;

    const options = [
      {
        value: "none",
        label: t("settings.advanced.overlayStyle.options.none"),
      },
      {
        value: "minimal",
        label: t("settings.advanced.overlayStyle.options.minimal"),
      },
      {
        value: "live",
        label: t("settings.advanced.overlayStyle.options.live"),
      },
    ];

    const stored = (getSetting("overlay_style") ?? "auto") as OverlayStyleValue;
    const selected =
      stored === "auto" ? (supportsLive ? "live" : "minimal") : stored;

    return (
      <SettingContainer
        title={t("settings.advanced.overlayStyle.title")}
        description={t("settings.advanced.overlayStyle.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <Dropdown
          options={options}
          selectedValue={selected}
          onSelect={(value) =>
            updateSetting("overlay_style", value as OverlayStyleValue)
          }
          disabled={isUpdating("overlay_style")}
        />
      </SettingContainer>
    );
  },
);
