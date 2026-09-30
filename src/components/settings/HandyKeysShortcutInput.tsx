import React, { useEffect, useState, useRef, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { formatKeyCombination } from "../../lib/utils/keyboard";
import { ResetButton } from "../ui/ResetButton";
import { SettingContainer } from "../ui/SettingContainer";
import { type SettingIcon, type SettingTone } from "../ui/tones";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "../../hooks/useOsType";
import { commands } from "@/bindings";
import { toast } from "sonner";
import {
  RecordingKeys,
  ShortcutKeysButton,
  type ShortcutFinish,
  type ShortcutSize,
} from "./ShortcutControl";

interface HandyKeysShortcutInputProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  shortcutId: string;
  disabled?: boolean;
  icon?: SettingIcon;
  tone?: SettingTone;
  /** Only the keys and their reset, without the settings row around them. */
  bare?: boolean;
  finish?: ShortcutFinish;
  size?: ShortcutSize;
  /** When to offer "back to the default". Rows always; bare only if changed. */
  showReset?: "always" | "changed" | "never";
}

interface HandyKeysEvent {
  modifiers: string[];
  key: string | null;
  is_key_down: boolean;
  hotkey_string: string;
}

export const HandyKeysShortcutInput: React.FC<HandyKeysShortcutInputProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
  shortcutId,
  disabled = false,
  icon,
  tone = "teal",
  bare = false,
  finish = "default",
  size,
  showReset,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateBinding, resetBinding, isUpdating, isLoading } =
    useSettings();
  const [isRecording, setIsRecording] = useState(false);
  const [currentKeys, setCurrentKeys] = useState<string>("");
  const [originalBinding, setOriginalBinding] = useState<string>("");
  const shortcutRef = useRef<HTMLDivElement | null>(null);
  const unlistenRef = useRef<(() => void) | null>(null);
  // Use a ref to track currentKeys for the event handler (avoids stale closure)
  const currentKeysRef = useRef<string>("");
  const osType = useOsType();

  const bindings = getSetting("bindings") || {};

  // Handle cancellation
  const cancelRecording = useCallback(async () => {
    if (!isRecording) return;

    // Stop listening for backend events
    if (unlistenRef.current) {
      unlistenRef.current();
      unlistenRef.current = null;
    }

    // Stop backend recording
    await commands.stopHandyKeysRecording().catch(console.error);

    // Restore original binding
    if (originalBinding) {
      try {
        await updateBinding(shortcutId, originalBinding);
      } catch (error) {
        console.error("Failed to restore original binding:", error);
        toast.error(t("settings.general.shortcut.errors.restore"));
      }
    }

    setIsRecording(false);
    setCurrentKeys("");
    currentKeysRef.current = "";
    setOriginalBinding("");
  }, [isRecording, originalBinding, shortcutId, updateBinding, t]);

  // Set up event listener for handy-keys events
  useEffect(() => {
    if (!isRecording) return;

    let cleanup = false;

    const setupListener = async () => {
      // Listen for key events from backend
      const unlisten = await listen<HandyKeysEvent>(
        "handy-keys-event",
        async (event) => {
          if (cleanup) return;

          const { hotkey_string, is_key_down } = event.payload;

          if (is_key_down && hotkey_string) {
            // Update both state (for display) and ref (for release handler)
            currentKeysRef.current = hotkey_string;
            setCurrentKeys(hotkey_string);
          } else if (!is_key_down && currentKeysRef.current) {
            // Key released - commit the shortcut using the ref value
            const keysToCommit = currentKeysRef.current;
            try {
              await updateBinding(shortcutId, keysToCommit);
            } catch (error) {
              console.error("Failed to change binding:", error);
              toast.error(
                t("settings.general.shortcut.errors.set", {
                  error: String(error),
                }),
              );

              // Reset to original binding on error
              if (originalBinding) {
                try {
                  await updateBinding(shortcutId, originalBinding);
                } catch (resetError) {
                  console.error("Failed to reset binding:", resetError);
                  toast.error(t("settings.general.shortcut.errors.reset"));
                }
              }
            }

            // Stop recording
            if (unlistenRef.current) {
              unlistenRef.current();
              unlistenRef.current = null;
            }
            await commands.stopHandyKeysRecording().catch(console.error);
            setIsRecording(false);
            setCurrentKeys("");
            currentKeysRef.current = "";
            setOriginalBinding("");
          }
        },
      );

      // The effect may have been torn down while `listen()` was resolving; the
      // cleanup then found no handle to call, so remove the listener here.
      if (cleanup) {
        unlisten();
        return;
      }
      unlistenRef.current = unlisten;
    };

    setupListener();

    return () => {
      cleanup = true;
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
      // Stop backend recording on unmount to prevent orphaned recording loops
      commands.stopHandyKeysRecording().catch(console.error);
    };
  }, [
    isRecording,
    shortcutId,
    originalBinding,
    updateBinding,
    cancelRecording,
    t,
  ]);

  // Handle click outside
  useEffect(() => {
    if (!isRecording) return;

    const handleClickOutside = (e: MouseEvent) => {
      if (
        shortcutRef.current &&
        !shortcutRef.current.contains(e.target as Node)
      ) {
        cancelRecording();
      }
    };

    window.addEventListener("click", handleClickOutside);
    return () => window.removeEventListener("click", handleClickOutside);
  }, [isRecording, cancelRecording]);

  // Start recording a new shortcut
  const startRecording = async () => {
    if (isRecording) return;

    // Store the original binding to restore if canceled
    setOriginalBinding(bindings[shortcutId]?.current_binding || "");

    // Start backend recording
    try {
      await commands.startHandyKeysRecording(shortcutId);
      setIsRecording(true);
      setCurrentKeys("");
      currentKeysRef.current = "";
    } catch (error) {
      console.error("Failed to start recording:", error);
      toast.error(
        t("settings.general.shortcut.errors.set", { error: String(error) }),
      );
    }
  };

  // Format the current shortcut keys being recorded
  const formatCurrentKeys = (): string => {
    if (!currentKeys) return t("settings.general.shortcut.pressKeys");
    return formatKeyCombination(currentKeys, osType);
  };

  // If still loading, show loading state
  if (isLoading) {
    if (bare) return null;
    return (
      <SettingContainer
        title={t("settings.general.shortcut.title")}
        description={t("settings.general.shortcut.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="text-sm text-mid-gray">
          {t("settings.general.shortcut.loading")}
        </div>
      </SettingContainer>
    );
  }

  // If no bindings are loaded, show empty state
  if (Object.keys(bindings).length === 0) {
    if (bare) return null;
    return (
      <SettingContainer
        title={t("settings.general.shortcut.title")}
        description={t("settings.general.shortcut.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="text-sm text-mid-gray">
          {t("settings.general.shortcut.none")}
        </div>
      </SettingContainer>
    );
  }

  const binding = bindings[shortcutId];
  if (!binding) {
    if (bare) return null;
    return (
      <SettingContainer
        title={t("settings.general.shortcut.title")}
        description={t("settings.general.shortcut.notFound")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="text-sm text-mid-gray">
          {t("settings.general.shortcut.none")}
        </div>
      </SettingContainer>
    );
  }

  // Get translated name and description for the binding
  const translatedName = t(
    `settings.general.shortcut.bindings.${shortcutId}.name`,
    binding.name,
  );
  const translatedDescription = t(
    `settings.general.shortcut.bindings.${shortcutId}.description`,
    binding.description,
  );

  const resetMode = showReset ?? (bare ? "changed" : "always");
  const changed = binding.current_binding !== binding.default_binding;
  const control = (
    <div className="flex items-center gap-1">
      {isRecording ? (
        <RecordingKeys
          ref={shortcutRef}
          text={formatCurrentKeys()}
          finish={finish}
          size={size ?? "md"}
        />
      ) : (
        <ShortcutKeysButton
          binding={binding.current_binding}
          name={translatedName}
          finish={finish}
          size={size}
          disabled={disabled}
          onClick={() => void startRecording()}
        />
      )}
      {(resetMode === "always" || (resetMode === "changed" && changed)) && (
        <ResetButton
          onClick={() => resetBinding(shortcutId)}
          disabled={isUpdating(`binding_${shortcutId}`)}
          ariaLabel={t("shortcutEditor.reset", { name: translatedName })}
          className={
            finish === "hero"
              ? "text-hero-muted! hover:text-hero-ink! hover:bg-hero-hover! focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-hero-ink"
              : ""
          }
        />
      )}
    </div>
  );

  if (bare) return control;

  return (
    <SettingContainer
      title={translatedName}
      description={translatedDescription}
      icon={icon}
      tone={tone}
      descriptionMode={descriptionMode}
      grouped={grouped}
      disabled={disabled}
      layout="horizontal"
    >
      {control}
    </SettingContainer>
  );
};
