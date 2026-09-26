import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { DropdownOption } from "../../ui/Dropdown";
import { ProviderGrid } from "../../ui/ProviderGrid";
import { useSettings } from "../../../hooks/useSettings";
import { prettyModelName } from "../../../lib/utils/prettyModelName";

/** Providers that authenticate without an API key. */
const KEYLESS = new Set(["builtin", "local", "custom", "apple_intelligence"]);

interface ProviderSelectProps {
  options: DropdownOption[];
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  /** Whose model to show under each provider. */
  modelsFor?: "cleanup" | "assistant";
}

/**
 * Cloud AI provider picker: every provider as a logo tile, with the model it
 * is set to and whether its key is saved. Shared by AI cleanup and the
 * assistant, which use the same providers and keys.
 */
export const ProviderSelect: React.FC<ProviderSelectProps> = React.memo(
  ({ options, value, onChange, disabled, modelsFor = "cleanup" }) => {
    const { t } = useTranslation();
    const { settings } = useSettings();
    const keys = settings?.post_process_api_keys;
    const models =
      modelsFor === "assistant"
        ? settings?.assistant_models
        : settings?.post_process_models;

    const tiles = useMemo(
      () =>
        options.map((option) => ({
          value: option.value,
          label: option.label,
          disabled: option.disabled,
          ready: KEYLESS.has(option.value) || !!keys?.[option.value]?.trim(),
          hint: prettyModelName(models?.[option.value]) || undefined,
        })),
      [options, keys, models],
    );

    return (
      <ProviderGrid
        kind="llm"
        label={t("settings.postProcessing.api.provider.title")}
        options={tiles}
        value={value}
        onChange={onChange}
        disabled={disabled}
      />
    );
  },
);

ProviderSelect.displayName = "ProviderSelect";
