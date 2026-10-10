import React from "react";
import { useTranslation } from "react-i18next";
import { SettingContainer } from "../ui/SettingContainer";
import { useRecordingMode } from "../../hooks/useRecordingMode";

interface PushToTalkProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const PushToTalk: React.FC<PushToTalkProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { mode, setMode, updating } = useRecordingMode();

    const options = [
      { value: "hold" as const, label: t("settings.general.pushToTalk.hold") },
      { value: "tap" as const, label: t("settings.general.pushToTalk.tap") },
      { value: "auto" as const, label: t("settings.general.pushToTalk.auto") },
    ];

    return (
      <SettingContainer
        title={t("settings.general.pushToTalk.label")}
        description={t("settings.general.pushToTalk.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div
          role="radiogroup"
          aria-label={t("settings.general.pushToTalk.label")}
          className="inline-grid w-max shrink-0 grid-flow-col auto-cols-auto rounded-lg border border-hairline-strong bg-surface-strong p-0.5"
        >
          {options.map((option) => {
            const selected = mode === option.value;
            return (
              <button
                key={option.label}
                type="button"
                role="radio"
                aria-checked={selected}
                disabled={updating}
                onClick={() => void setMode(option.value)}
                className={`h-8 whitespace-nowrap rounded-md px-3.5 text-[13px] font-medium leading-none transition-[background-color,color,box-shadow,transform] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 active:scale-[0.98] disabled:cursor-not-allowed disabled:opacity-50 ${
                  selected
                    ? "bg-surface text-ink shadow-[0_1px_3px_rgba(0,0,0,0.14)]"
                    : "text-muted hover:text-ink"
                }`}
              >
                {option.label}
              </button>
            );
          })}
        </div>
      </SettingContainer>
    );
  },
);

PushToTalk.displayName = "PushToTalk";
