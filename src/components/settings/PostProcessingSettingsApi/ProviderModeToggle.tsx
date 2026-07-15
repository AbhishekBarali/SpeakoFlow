import React from "react";
import { useTranslation } from "react-i18next";

export type ProviderMode = "device" | "cloud";

type ProviderModeToggleProps = {
  mode: ProviderMode;
  onChange: (mode: ProviderMode) => void;
  disabled?: boolean;
};

/** Shared location picker for LLM-backed features. Keeping one component makes
 * Assistant and AI cleanup communicate local-vs-cloud execution identically. */
export const ProviderModeToggle: React.FC<ProviderModeToggleProps> = ({
  mode,
  onChange,
  disabled = false,
}) => {
  const { t } = useTranslation();
  const options: { value: ProviderMode; label: string }[] = [
    { value: "device", label: t("settings.assistant.brain.onDevice") },
    { value: "cloud", label: t("settings.assistant.brain.cloud") },
  ];

  return (
    <div
      className="inline-flex rounded-lg border border-hairline bg-surface-strong p-0.5"
      role="group"
      aria-label={t("settings.assistant.brain.whereLabel")}
    >
      {options.map((option) => {
        const active = option.value === mode;
        return (
          <button
            key={option.value}
            type="button"
            aria-pressed={active}
            disabled={disabled}
            onClick={() => onChange(option.value)}
            className={`cursor-pointer rounded-md px-3.5 py-1.5 text-[13px] font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
              active
                ? "bg-surface text-ink shadow-sm"
                : "text-muted hover:text-ink"
            }`}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
};
