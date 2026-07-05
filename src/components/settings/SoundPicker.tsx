import React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../ui/Button";
import { Dropdown, DropdownOption } from "../ui/Dropdown";
import { PlayIcon } from "lucide-react";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettingsStore } from "../../stores/settingsStore";
import { useSettings } from "../../hooks/useSettings";

interface SoundPickerProps {
  label: string;
  description: string;
}

export const SoundPicker: React.FC<SoundPickerProps> = ({
  label,
  description,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  const playTestSound = useSettingsStore((state) => state.playTestSound);
  const customSounds = useSettingsStore((state) => state.customSounds);

  const selectedTheme = getSetting("sound_theme") ?? "dictation";

  const options: DropdownOption[] = [
    { value: "dictation", label: t("settings.sound.soundTheme.default") },
    { value: "marimba", label: "Marimba" },
    { value: "pop", label: "Pop" },
    { value: "click", label: "Click" },
  ];

  // Only add Custom option if both custom sound files exist
  if (customSounds.start && customSounds.stop) {
    options.push({ value: "custom", label: "Custom" });
  }

  const handlePlayBothSounds = async () => {
    await playTestSound("start");
    await playTestSound("stop");
  };

  return (
    <SettingContainer
      title={label}
      description={description}
      grouped
      layout="horizontal"
    >
      <div className="flex items-center gap-2">
        <Dropdown
          selectedValue={selectedTheme}
          onSelect={(value) =>
            updateSetting(
              "sound_theme",
              value as "dictation" | "marimba" | "pop" | "click" | "custom",
            )
          }
          options={options}
        />
        <Button
          variant="ghost"
          size="sm"
          onClick={handlePlayBothSounds}
          title={t("settings.sound.soundTheme.preview")}
        >
          <PlayIcon className="h-4 w-4" />
        </Button>
      </div>
    </SettingContainer>
  );
};
