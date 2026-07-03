import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import {
  getKeyName,
  formatKeyCombination,
  normalizeKey,
} from "../../lib/utils/keyboard";
import { SettingContainer } from "../ui/SettingContainer";
import { TONE_PILL, type SettingIcon, type SettingTone } from "../ui/tones";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "../../hooks/useOsType";

interface TapToLockProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  icon?: SettingIcon;
  tone?: SettingTone;
  /** Which setting this control edits. Defaults to the dictation lock key, so
   *  existing call sites keep working; the assistant passes its own key. */
  settingKey?: "tap_to_lock_key" | "assistant_tap_to_lock_key";
  /** Display fallback before settings load / when the value is unset. */
  fallback?: string;
  /** i18n keys, so one control serves both the dictation and assistant rows. */
  labelKey?: string;
  infoKey?: string;
  offKey?: string;
  clearKey?: string;
}

const MODIFIERS = [
  "ctrl",
  "control",
  "shift",
  "alt",
  "option",
  "meta",
  "command",
  "cmd",
  "super",
  "win",
  "windows",
  "fn",
];

/**
 * "Tap to Lock": while holding your record shortcut, tap this shortcut to lock
 * a recording hands-free. Captured exactly like the other shortcuts — press any
 * key/combo, or clear it to turn the gesture off. The what/why lives behind the
 * (i) hint so the row stays quiet.
 */
export const TapToLock: React.FC<TapToLockProps> = React.memo(
  ({
    grouped = false,
    icon,
    tone = "teal",
    settingKey = "tap_to_lock_key",
    fallback = "shift",
    labelKey = "settings.general.tapToLock.label",
    infoKey = "settings.general.tapToLock.info",
    offKey = "settings.general.tapToLock.off",
    clearKey = "settings.general.tapToLock.clear",
  }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting } = useSettings();
    const osType = useOsType();

    const lockKey = getSetting(settingKey) ?? fallback;

    const [recording, setRecording] = useState(false);
    const [pressed, setPressed] = useState<string[]>([]);
    const [recorded, setRecorded] = useState<string[]>([]);
    const boxRef = useRef<HTMLDivElement | null>(null);

    // Capture key presses while recording. Mirrors GlobalShortcutInput: modifiers
    // sort first, the shortcut commits once every key is released.
    useEffect(() => {
      if (!recording) return;
      let cleanup = false;

      const handleKeyDown = (e: KeyboardEvent) => {
        if (cleanup || e.repeat) return;
        e.preventDefault();
        const key = normalizeKey(getKeyName(e, osType));
        if (!pressed.includes(key)) {
          setPressed((prev) => [...prev, key]);
          if (!recorded.includes(key)) setRecorded((prev) => [...prev, key]);
        }
      };

      const handleKeyUp = (e: KeyboardEvent) => {
        if (cleanup) return;
        e.preventDefault();
        const key = normalizeKey(getKeyName(e, osType));
        setPressed((prev) => prev.filter((k) => k !== key));

        const remaining = pressed.filter((k) => k !== key);
        if (remaining.length === 0 && recorded.length > 0) {
          const sorted = [...recorded].sort((a, b) => {
            const am = MODIFIERS.includes(a.toLowerCase());
            const bm = MODIFIERS.includes(b.toLowerCase());
            if (am && !bm) return -1;
            if (!am && bm) return 1;
            return 0;
          });
          updateSetting(settingKey, sorted.join("+"));
          setRecording(false);
          setPressed([]);
          setRecorded([]);
        }
      };

      const handleClickOutside = (e: MouseEvent) => {
        if (cleanup) return;
        if (boxRef.current && !boxRef.current.contains(e.target as Node)) {
          setRecording(false);
          setPressed([]);
          setRecorded([]);
        }
      };

      window.addEventListener("keydown", handleKeyDown);
      window.addEventListener("keyup", handleKeyUp);
      window.addEventListener("click", handleClickOutside);
      return () => {
        cleanup = true;
        window.removeEventListener("keydown", handleKeyDown);
        window.removeEventListener("keyup", handleKeyUp);
        window.removeEventListener("click", handleClickOutside);
      };
    }, [recording, pressed, recorded, osType, updateSetting, settingKey]);

    const start = () => {
      if (recording) return;
      setPressed([]);
      setRecorded([]);
      setRecording(true);
    };

    const clear = useCallback(
      (e: React.MouseEvent) => {
        e.stopPropagation();
        updateSetting(settingKey, "");
      },
      [updateSetting, settingKey],
    );

    const display = recording
      ? recorded.length > 0
        ? formatKeyCombination(recorded.join("+"), osType)
        : t("settings.general.shortcut.pressKeys")
      : lockKey
        ? formatKeyCombination(lockKey, osType)
        : t(offKey);

    return (
      <SettingContainer
        title={t(labelKey)}
        info={t(infoKey)}
        icon={icon}
        tone={tone}
        grouped={grouped}
        layout="horizontal"
      >
        <div className="flex items-center space-x-1">
          <div
            ref={recording ? boxRef : undefined}
            onClick={recording ? undefined : start}
            className={
              recording
                ? "px-2.5 py-1 text-[13px] font-medium border border-accent bg-accent/10 text-accent rounded-md"
                : `px-2.5 py-1 text-[13px] font-medium border rounded-md cursor-pointer transition-all ${TONE_PILL[tone]} elev-chip hover:brightness-[1.06]`
            }
          >
            {display}
          </div>
          {!recording && lockKey && (
            <button
              type="button"
              onClick={clear}
              title={t(clearKey)}
              aria-label={t(clearKey)}
              className="p-1 text-muted hover:text-ink transition-colors cursor-pointer"
            >
              <X size={15} />
            </button>
          )}
        </div>
      </SettingContainer>
    );
  },
);

TapToLock.displayName = "TapToLock";
