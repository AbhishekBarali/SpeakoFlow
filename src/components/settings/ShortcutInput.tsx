import React from "react";
import { useSettings } from "../../hooks/useSettings";
import { GlobalShortcutInput } from "./GlobalShortcutInput";
import { HandyKeysShortcutInput } from "./HandyKeysShortcutInput";
import type { SettingIcon, SettingTone } from "../ui/tones";

interface ShortcutInputProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  shortcutId: string;
  disabled?: boolean;
  icon?: SettingIcon;
  tone?: SettingTone;
}

/**
 * Wrapper that picks the shortcut-capture UI to match the active
 * `keyboard_implementation` engine.
 *
 * - "handy_keys": HandyKeysShortcutInput (backend key events; supports
 *   modifier-only shortcuts like "Ctrl+Super"). This is the default engine on
 *   Windows and macOS.
 * - "tauri": GlobalShortcutInput (JS keyboard events; requires a main key).
 *   Default engine on Linux, and the fallback used here if the setting is unset.
 */
export const ShortcutInput: React.FC<ShortcutInputProps> = (props) => {
  const { getSetting } = useSettings();
  const keyboardImplementation = getSetting("keyboard_implementation");

  // Fall back to the Tauri capture UI only when the setting is unset.
  if (keyboardImplementation === "handy_keys") {
    return <HandyKeysShortcutInput {...props} />;
  }

  return <GlobalShortcutInput {...props} />;
};
