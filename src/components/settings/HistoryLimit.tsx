import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useSettings } from "../../hooks/useSettings";
import { Input } from "../ui/Input";
import { SettingContainer } from "../ui/SettingContainer";

interface HistoryLimitProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

export const HistoryLimit: React.FC<HistoryLimitProps> = ({
  descriptionMode = "inline",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();

  const persistedLimit = getSetting("history_limit") ?? 20;
  const [inputValue, setInputValue] = useState(String(persistedLimit));

  useEffect(() => {
    setInputValue(String(persistedLimit));
  }, [persistedLimit]);

  const handleBlur = async () => {
    const parsed = Number.parseInt(inputValue, 10);
    if (!Number.isFinite(parsed)) {
      setInputValue(String(persistedLimit));
      return;
    }

    const next = Math.max(0, Math.min(1000, parsed));
    setInputValue(String(next));
    if (next === persistedLimit) return;

    await updateSetting("history_limit", next);
    toast.success(t("settings.debug.recordingRetention.appliedToast"));
  };

  return (
    <SettingContainer
      title={t("settings.debug.historyLimit.title")}
      description={t("settings.debug.historyLimit.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="horizontal"
    >
      <div className="flex items-center space-x-2">
        <Input
          type="number"
          min="0"
          max="1000"
          value={inputValue}
          onChange={(event) => setInputValue(event.target.value)}
          onBlur={handleBlur}
          disabled={isUpdating("history_limit")}
          className="w-20"
        />
        <span className="text-sm text-text">
          {t("settings.debug.historyLimit.entries")}
        </span>
      </div>
    </SettingContainer>
  );
};
