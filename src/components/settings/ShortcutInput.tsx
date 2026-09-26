import React from "react";
import { useSettings } from "../../hooks/useSettings";
import { GlobalShortcutInput } from "./GlobalShortcutInput";
import { HandyKeysShortcutInput } from "./HandyKeysShortcutInput";
import type { ShortcutFinish, ShortcutSize } from "./ShortcutControl";
import type { SettingIcon, SettingTone } from "../ui/tones";

interface ShortcutInputProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  shortcutId: string;
  disabled?: boolean;
  icon?: SettingIcon;
  tone?: SettingTone;
  /** Render only the clickable keys (for a table cell or a hero). */
  bare?: boolean;
  finish?: ShortcutFinish;
  size?: ShortcutSize;
  showReset?: "always" | "changed" | "never";
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
 *
 * Both render the same keys, so a shortcut can be changed wherever it is shown:
 * a Settings row by default, or just the keys with `bare`.
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
