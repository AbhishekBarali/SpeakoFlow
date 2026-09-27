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
 * The overlay style as the user sees it: "auto" (the default) resolved to Live
 * when the active speech engine streams, otherwise Minimal. Shared with the
 * settings that only mean something for one of the styles.
 *
 * Cloud transcription has no local model to ask; there the "Transcribe as I
 * speak" switch is the closest thing the UI has to the backend's answer
 * (`selected_model_supports_live`), which also checks the provider.
 */
export const useResolvedOverlayStyle = (): Exclude<
  OverlayStyleValue,
  "auto"
> => {
  const { getSetting } = useSettings();
  const models = useModelStore((s) => s.models);
  const currentModel = useModelStore((s) => s.currentModel);
  const supportsLive =
    getSetting("stt_engine_mode") === "cloud"
      ? !!getSetting("cloud_stt_streaming")
      : (models.find((m) => m.id === currentModel)?.supports_streaming ??
        false);
  const stored = (getSetting("overlay_style") ?? "auto") as OverlayStyleValue;
  return stored === "auto" ? (supportsLive ? "live" : "minimal") : stored;
};

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
    const { updateSetting, isUpdating } = useSettings();
    const selected = useResolvedOverlayStyle();

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
