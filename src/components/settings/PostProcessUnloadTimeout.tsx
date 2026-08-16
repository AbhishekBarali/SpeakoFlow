import type { ModelUnloadTimeout } from "@/bindings";
import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";

interface PostProcessUnloadTimeoutProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

/**
 * How long the on-device AI-cleanup model stays in memory after its last use.
 *
 * Deliberately separate from the assistant's own setting: cleanup runs on every
 * dictation with a small model, so holding it through a writing session removes
 * the reload from the dictation path, while the assistant's larger model can
 * still be released quickly. An idle model costs memory, not CPU.
 */
export const PostProcessUnloadTimeout: React.FC<
  PostProcessUnloadTimeoutProps
> = ({ descriptionMode = "tooltip", grouped = false }) => {
  const { t } = useTranslation();
  const { settings, getSetting, updateSetting, isUpdating } = useSettings();

  // NOTE: the values are the serde snake_case forms the backend accepts
  // ("min2", not the "min_2" specta emits for the TS type) — matching
  // ModelUnloadTimeout.tsx and AssistantSettings, hence the casts. Sending the
  // specta form would fail to deserialize and silently not save.
  const options = useMemo(() => {
    const base: { value: ModelUnloadTimeout; label: string }[] = [
      {
        value: "never" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.never"),
      },
      {
        value: "immediately" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.immediately"),
      },
      {
        value: "min5" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min5"),
      },
      {
        value: "min15" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.min15"),
      },
      {
        value: "hour1" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.hour1"),
      },
    ];
    if (settings?.debug_mode) {
      base.push({
        value: "sec15" as ModelUnloadTimeout,
        label: t("settings.advanced.modelUnload.options.sec15"),
      });
    }
    return base;
  }, [settings?.debug_mode, t]);

  const current =
    getSetting("post_process_unload_timeout") ??
    ("min15" as ModelUnloadTimeout);

  return (
    <SettingContainer
      title={t("settings.dictation.aiCleanup.unloadTimeout.title")}
      description={t("settings.dictation.aiCleanup.unloadTimeout.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
    >
      <Dropdown
        options={options}
        selectedValue={current}
        onSelect={(value) =>
          updateSetting(
            "post_process_unload_timeout",
            value as ModelUnloadTimeout,
          )
        }
        disabled={isUpdating("post_process_unload_timeout")}
        className="min-w-[200px]"
      />
    </SettingContainer>
  );
};
