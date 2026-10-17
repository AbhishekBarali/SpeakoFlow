import React from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "../../hooks/useOsType";
import { shortKeyCombination } from "../ui/Keycaps";
import { SettingContainer } from "../ui/SettingContainer";
import { HANDS_FREE_ID, handsFree, type HandsFree } from "@/lib/handsFree";
import type { OSType } from "@/lib/utils/keyboard";
import { KeysPrefix, ShortcutRow } from "./ShortcutRow";

type T = (key: string, options?: Record<string, unknown>) => string;

/** What the hands-free row's (i) says, the same on Home and in Settings. */
export const handsFreeInfo = (
  t: T,
  state: HandsFree,
  dictationBinding: string | undefined,
  os: OSType,
): string => {
  const keys = shortKeyCombination(dictationBinding, os);
  switch (state.state) {
    case "off":
      return t("settings.general.handsFree.descriptionOff");
    case "taken":
      return t("settings.general.handsFree.taken", {
        name: t(`settings.general.shortcut.bindings.${state.by}.name`),
        keys: shortKeyCombination(state.keys, os),
      });
    case "unavailable":
      return os === "linux"
        ? t("settings.general.handsFree.linux")
        : state.reason === "engine"
          ? t("settings.general.handsFree.unavailableEngine", {
              where: `${t("settingsDialog.title")} → ${t("settingsDialog.tabs.advanced")}`,
            })
          : t("settings.general.handsFree.unavailableKeys", { keys });
    default:
      return t("home.shortcuts.handsFree.what", {
        keys,
        key: state.state === "ready" ? shortKeyCombination(state.key, os) : "",
      });
  }
};

/**
 * Hands-free (`hands_free.rs`): one key that, pressed while dictating, keeps
 * the dictation listening after the dictation keys are let go. Shown as what
 * it is: "While dictating, press [Space]". The whole combination
 * (Ctrl + Win + Space) read as three keys to press at once, and "+ Space" as
 * half a shortcut.
 *
 * With Hold to talk off every dictation already is hands-free, so the row is
 * not there. Where there can be no hands-free key (Linux's Ctrl + Space, the
 * other keyboard engine), the row says to turn Hold to talk off instead, which
 * is the switch right above it.
 */
export const HandsFreeShortcut: React.FC = React.memo(() => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const os = useOsType();

  const state = handsFree(settings);
  if (state.state === "tapped") return null;

  const title = t("settings.general.shortcut.bindings.hands_free.name");
  const info = handsFreeInfo(
    t,
    state,
    settings?.bindings?.transcribe?.current_binding,
    os,
  );

  if (state.state === "unavailable") {
    // Nothing to press here: the fix is the Hold to talk switch above.
    return (
      <SettingContainer
        title={title}
        description={info}
        descriptionMode="caption"
        grouped={true}
      >
        {null}
      </SettingContainer>
    );
  }

  return (
    <ShortcutRow
      shortcutId={HANDS_FREE_ID}
      title={title}
      info={state.state === "taken" ? undefined : info}
      caption={state.state === "taken" ? info : undefined}
      prefix={
        state.state === "off" ? null : (
          <KeysPrefix>
            {t("settings.general.handsFree.whileDictating")}
          </KeysPrefix>
        )
      }
    />
  );
});

HandsFreeShortcut.displayName = "HandsFreeShortcut";
