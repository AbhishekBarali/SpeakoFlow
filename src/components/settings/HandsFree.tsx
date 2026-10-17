import React from "react";
import { useTranslation } from "react-i18next";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "../../hooks/useOsType";
import { formatKeyCombination } from "@/lib/utils/keyboard";
import { HANDS_FREE_ID, handsFree } from "@/lib/handsFree";
import { ShortcutInput } from "./ShortcutInput";

interface HandsFreeShortcutProps {
  grouped?: boolean;
}

/**
 * The hands-free key (`hands_free.rs`): one key, pressed while the dictation
 * keys are held, that keeps the dictation listening after they are let go. It
 * is changed like any shortcut, and removing it is how hands-free mode is
 * turned off. With Tap to toggle every dictation already is hands-free, so
 * the row is not there. Where the key cannot work, the reason is on the row
 * rather than behind the (i): a control that does nothing needs its reason in
 * sight.
 */
export const HandsFreeShortcut: React.FC<HandsFreeShortcutProps> = React.memo(
  ({ grouped = false }) => {
    const { t } = useTranslation();
    const { settings } = useSettings();
    const osType = useOsType();

    const state = handsFree(settings);
    if (state.state === "tapped") return null;

    const dictationKeys = formatKeyCombination(
      settings?.bindings?.transcribe?.current_binding ?? "",
      osType,
    );
    const unavailable = state.state === "unavailable";
    const reason =
      state.state === "unavailable"
        ? state.reason === "engine"
          ? t("settings.general.handsFree.unavailableEngine", {
              where: `${t("settingsDialog.title")} → ${t("settingsDialog.tabs.advanced")}`,
            })
          : t("settings.general.handsFree.unavailableKeys", {
              keys: dictationKeys,
            })
        : state.state === "taken"
          ? t("settings.general.handsFree.taken", {
              name: t(`settings.general.shortcut.bindings.${state.by}.name`),
              keys: formatKeyCombination(state.keys, osType),
            })
          : undefined;

    return (
      <SettingContainer
        title={t("settings.general.shortcut.bindings.hands_free.name")}
        description={reason}
        info={t("settings.general.handsFree.description", {
          keys: dictationKeys,
        })}
        descriptionMode={reason ? "caption" : "tooltip"}
        grouped={grouped}
        disabled={unavailable}
      >
        {/* One more key on top of the dictation keys, drawn that way, so it
            does not read as a shortcut on Space alone. */}
        <span className="flex items-center gap-1.5">
          {state.state !== "off" && (
            <span aria-hidden="true" className="text-[0.9375rem] text-muted">
              +
            </span>
          )}
          <ShortcutInput
            shortcutId={HANDS_FREE_ID}
            bare
            disabled={unavailable}
          />
        </span>
      </SettingContainer>
    );
  },
);

HandsFreeShortcut.displayName = "HandsFreeShortcut";
