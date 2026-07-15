import { memo, type FC } from "react";
import { useTranslation } from "react-i18next";
import { SmilePlus } from "lucide-react";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
import { useSettings } from "@/hooks/useSettings";

interface SpokenEmojiToggleProps {
  grouped?: boolean;
}

/** Opt-in, local conversion of commands such as “happy emoji” into 😊. */
export const SpokenEmojiToggle: FC<SpokenEmojiToggleProps> = memo(
  ({ grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const enabled = getSetting("spoken_emojis_enabled") ?? false;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(value) => updateSetting("spoken_emojis_enabled", value)}
        isUpdating={isUpdating("spoken_emojis_enabled")}
        label={t("settings.dictation.spokenEmoji.title")}
        description={t("settings.dictation.spokenEmoji.description")}
        descriptionMode="tooltip"
        grouped={grouped}
        icon={SmilePlus}
        tone="violet"
      />
    );
  },
);
