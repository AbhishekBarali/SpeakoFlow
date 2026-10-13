import React from "react";
import { useTranslation } from "react-i18next";
import { SettingContainer } from "../ui/SettingContainer";
import { Switch } from "../ui/Switch";
import { Keycaps } from "../ui/Keycaps";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "../../hooks/useOsType";
import { formatKeyCombination } from "@/lib/utils/keyboard";
import { handsFreeSpace } from "@/lib/handsFree";

interface HandsFreeProps {
  grouped?: boolean;
}

/**
 * The dictation keys plus Space (`hands_free.rs`): pressed while the keys are
 * held, the dictation keeps listening after they are let go. The row shows the
 * actual keys, because "Space" alone does not say it is Space *with* them.
 * Where there can be no such key, the switch is off and the reason is on the
 * row, not behind the (i): a disabled control needs its reason in sight.
 */
export const HandsFreeSpaceSetting: React.FC<HandsFreeProps> = React.memo(
  ({ grouped = false }) => {
    const { t } = useTranslation();
    const { settings, getSetting, updateSetting, isUpdating } = useSettings();
    const osType = useOsType();

    const space = handsFreeSpace(settings);
    const enabled = getSetting("dictation_hands_free_space") ?? true;
    const unavailable = space.state === "unavailable";
    const reason =
      space.state === "unavailable"
        ? t("settings.general.handsFree.space.unavailable")
        : space.state === "taken" && enabled
          ? t("settings.general.handsFree.space.taken", {
              keys: formatKeyCombination(space.keys, osType),
            })
          : undefined;

    return (
      <SettingContainer
        title={t("settings.general.handsFree.space.label")}
        description={reason}
        info={t("settings.general.handsFree.space.description")}
        descriptionMode={reason ? "caption" : "tooltip"}
        grouped={grouped}
        disabled={unavailable}
      >
        <div className="flex items-center gap-3">
          {space.state === "ready" && enabled && (
            <Keycaps binding={space.keys} size="sm" />
          )}
          <Switch
            checked={enabled && !unavailable}
            onChange={(on) =>
              void updateSetting("dictation_hands_free_space", on)
            }
            disabled={unavailable || isUpdating("dictation_hands_free_space")}
            label={t("settings.general.handsFree.space.label")}
          />
        </div>
      </SettingContainer>
    );
  },
);

HandsFreeSpaceSetting.displayName = "HandsFreeSpaceSetting";

/**
 * With Hold to talk, a quick tap keeps the dictation listening instead of
 * ending it. With Tap to toggle every dictation already does, so the switch
 * is off and says why rather than being a setting that does nothing.
 */
export const HandsFreeTapSetting: React.FC<HandsFreeProps> = React.memo(
  ({ grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const holdToTalk = getSetting("push_to_talk") ?? true;
    const enabled = getSetting("dictation_hands_free_tap") ?? false;

    return (
      <SettingContainer
        title={t("settings.general.handsFree.tap.label")}
        description={
          holdToTalk ? undefined : t("settings.general.handsFree.tap.tapMode")
        }
        info={t("settings.general.handsFree.tap.description")}
        descriptionMode={holdToTalk ? "tooltip" : "caption"}
        grouped={grouped}
        disabled={!holdToTalk}
      >
        <Switch
          checked={holdToTalk && enabled}
          onChange={(on) => void updateSetting("dictation_hands_free_tap", on)}
          disabled={!holdToTalk || isUpdating("dictation_hands_free_tap")}
          label={t("settings.general.handsFree.tap.label")}
        />
      </SettingContainer>
    );
  },
);

HandsFreeTapSetting.displayName = "HandsFreeTapSetting";
