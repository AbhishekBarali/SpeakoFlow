import React, { useEffect, useState, useRef, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  formatKeyCombination,
  hasMouseButton,
  isClickKey,
  isMouseButtonKey,
} from "../../lib/utils/keyboard";
import { shortcutUsingKeys } from "../../lib/utils/shortcutConflict";
import { SettingContainer } from "../ui/SettingContainer";
import { type SettingIcon, type SettingTone } from "../ui/tones";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "../../hooks/useOsType";
import { commands } from "@/bindings";
import { toast } from "sonner";
import {
  RecordingKeys,
  ShortcutEditActions,
  ShortcutKeysAtRest,
  announceTurnedOff,
  isOptionalShortcut,
  useMouseButtonsWhileRecording,
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
  /** Only the keys, without the settings row around them. */
  bare?: boolean;
  finish?: ShortcutFinish;
  size?: ShortcutSize;
}

interface HandyKeysEvent {
  modifiers: string[];
  key: string | null;
  is_key_down: boolean;
  hotkey_string: string;
}

/** Which of the shortcut's two sets of keys is being recorded. */
type Slot = "primary" | "alternate";

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
}) => {
  const { t } = useTranslation();
  const {
    getSetting,
    updateBinding,
    updateAlternateBinding,
    resetBinding,
    isUpdating,
    isLoading,
  } = useSettings();
  const [slot, setSlot] = useState<Slot | null>(null);
  const isRecording = slot !== null;
  const [currentKeys, setCurrentKeys] = useState<string>("");
  const [originalBinding, setOriginalBinding] = useState<string>("");
  const shortcutRef = useRef<HTMLDivElement | null>(null);
  const unlistenRef = useRef<(() => void) | null>(null);
  // Use a ref to track currentKeys for the event handler (avoids stale closure)
  const currentKeysRef = useRef<string>("");
  const osType = useOsType();

  const bindings = getSetting("bindings") || {};
  const mouseButtons = getSetting("mouse_button_shortcuts") ?? false;
  // Read by the key-event handler, which outlives a render. Switching from
  // the shortcut's own keys to its other keys must not restart the listener:
  // tearing it down stops the backend recorder with it.
  const latest = useRef({
    bindings,
    mouseButtons,
    slot,
    originalBinding,
    osType,
    t,
  });
  latest.current = { bindings, mouseButtons, slot, originalBinding, osType, t };

  // Back, Forward and the middle button are being recorded, not acted on.
  useMouseButtonsWhileRecording(isRecording);

  const resetEditor = useCallback(() => {
    setSlot(null);
    setCurrentKeys("");
    currentKeysRef.current = "";
    setOriginalBinding("");
  }, []);

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

    // Restore original binding. Other keys being recorded were never touched.
    if (slot === "primary" && originalBinding) {
      try {
        await updateBinding(shortcutId, originalBinding);
      } catch (error) {
        console.error("Failed to restore original binding:", error);
        toast.error(t("settings.general.shortcut.errors.restore"));
      }
    }

    resetEditor();
  }, [
    isRecording,
    slot,
    originalBinding,
    shortcutId,
    updateBinding,
    resetEditor,
    t,
  ]);

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

          const { key, hotkey_string, is_key_down } = event.payload;
          const now = latest.current;

          // A mouse button is part of a shortcut only while mouse buttons are
          // on, and a click never is: it is how the editor itself is used.
          if (isMouseButtonKey(key) && (!now.mouseButtons || isClickKey(key))) {
            return;
          }

          if (is_key_down && hotkey_string) {
            // Update both state (for display) and ref (for release handler)
            currentKeysRef.current = hotkey_string;
            setCurrentKeys(hotkey_string);
          } else if (!is_key_down && currentKeysRef.current) {
            // Key released - commit the shortcut using the ref value
            const keysToCommit = currentKeysRef.current;
            const target = now.slot ?? "primary";
            const takenBy = shortcutUsingKeys(
              now.bindings,
              keysToCommit,
              shortcutId,
              target,
            );
            if (takenBy === shortcutId) {
              toast.error(
                now.t("settings.general.shortcut.errors.sameAsOther", {
                  keys: formatKeyCombination(keysToCommit, now.osType),
                }),
              );
            } else if (takenBy) {
              // Nothing was changed, so the shortcut keeps its keys.
              toast.error(
                now.t("settings.general.shortcut.errors.inUse", {
                  name: now.t(
                    `settings.general.shortcut.bindings.${takenBy}.name`,
                    now.bindings[takenBy]?.name ?? takenBy,
                  ),
                  keys: formatKeyCombination(keysToCommit, now.osType),
                }),
              );
            } else if (target === "alternate") {
              try {
                await updateAlternateBinding(shortcutId, keysToCommit);
              } catch (error) {
                console.error("Failed to set the other keys:", error);
                toast.error(
                  now.t("settings.general.shortcut.errors.set", {
                    error: String(error),
                  }),
                );
              }
            } else {
              try {
                await updateBinding(shortcutId, keysToCommit);
              } catch (error) {
                console.error("Failed to change binding:", error);
                toast.error(
                  now.t("settings.general.shortcut.errors.set", {
                    error: String(error),
                  }),
                );

                // Reset to original binding on error
                if (now.originalBinding) {
                  try {
                    await updateBinding(shortcutId, now.originalBinding);
                  } catch (resetError) {
                    console.error("Failed to reset binding:", resetError);
                    toast.error(
                      now.t("settings.general.shortcut.errors.reset"),
                    );
                  }
                }
              }
            }

            // Stop recording
            if (unlistenRef.current) {
              unlistenRef.current();
              unlistenRef.current = null;
            }
            await commands.stopHandyKeysRecording().catch(console.error);
            setSlot(null);
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
  }, [isRecording, shortcutId, updateBinding, updateAlternateBinding]);

  // Handle click outside
  useEffect(() => {
    if (!isRecording) return;

    const handleClickOutside = (e: MouseEvent) => {
      // Judged by the path the click took when it happened: "Add another"
      // re-renders the editor during its own click, so by the time the click
      // reaches the window the button it landed on is no longer in the page.
      if (
        shortcutRef.current &&
        !e.composedPath().includes(shortcutRef.current)
      ) {
        cancelRecording();
      }
    };

    window.addEventListener("click", handleClickOutside);
    return () => window.removeEventListener("click", handleClickOutside);
  }, [isRecording, cancelRecording]);

  // Start recording new keys for one of the shortcut's two sets
  const startRecording = async (target: Slot) => {
    if (isRecording) return;

    // Store the original binding to restore if canceled
    setOriginalBinding(
      target === "primary" ? bindings[shortcutId]?.current_binding || "" : "",
    );

    // Start backend recording
    try {
      await commands.startHandyKeysRecording(shortcutId);
      setSlot(target);
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
    if (!currentKeys)
      return mouseButtons
        ? t("shortcutEditor.pressKeysOrButton")
        : t("settings.general.shortcut.pressKeys");
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

  const optional = isOptionalShortcut(shortcutId);
  const busy = isUpdating(`binding_${shortcutId}`);
  // A mouse button kept while mouse buttons are off is listened for by
  // nothing, so it is not shown as working keys either.
  const storedOther = binding.alternate_binding?.trim() ?? "";
  const otherKeys =
    storedOther && (mouseButtons || !hasMouseButton(storedOther))
      ? storedOther
      : "";

  // Leave the editor without restoring anything: the action that follows
  // decides what the shortcut becomes.
  const leaveEditor = async () => {
    if (unlistenRef.current) {
      unlistenRef.current();
      unlistenRef.current = null;
    }
    await commands.stopHandyKeysRecording().catch(console.error);
    resetEditor();
  };
  const useDefault = async () => {
    await leaveEditor();
    await resetBinding(shortcutId);
  };
  const turnOff = async () => {
    const previous = originalBinding;
    await leaveEditor();
    try {
      await updateBinding(shortcutId, "");
      announceTurnedOff(t, translatedName, () => {
        void updateBinding(shortcutId, previous).catch(console.error);
      });
    } catch (error) {
      console.error("Failed to turn off binding:", error);
      toast.error(
        t("settings.general.shortcut.errors.set", { error: String(error) }),
      );
    }
  };
  // The recorder keeps running: the next keys pressed become the other keys.
  // Nothing about the shortcut's own keys changed, so there is nothing to
  // restore if this is abandoned.
  const addAnother = () => {
    setSlot("alternate");
    setOriginalBinding("");
    setCurrentKeys("");
    currentKeysRef.current = "";
  };
  const removeOther = async () => {
    await leaveEditor();
    try {
      await updateAlternateBinding(shortcutId, "");
    } catch (error) {
      console.error("Failed to remove the other keys:", error);
      toast.error(
        t("settings.general.shortcut.errors.set", { error: String(error) }),
      );
    }
  };

  const control = (
    <div className="flex items-center gap-1">
      {isRecording ? (
        <div ref={shortcutRef} className="flex flex-col items-end gap-0.5">
          <RecordingKeys
            text={formatCurrentKeys()}
            finish={finish}
            size={size ?? "md"}
          />
          {slot === "primary" ? (
            <ShortcutEditActions
              current={originalBinding}
              defaultBinding={binding.default_binding}
              optional={optional}
              finish={finish}
              disabled={busy}
              onUseDefault={() => void useDefault()}
              onTurnOff={() => void turnOff()}
              onAddAnother={
                originalBinding.trim() && !otherKeys ? addAnother : undefined
              }
            />
          ) : (
            <ShortcutEditActions
              current={otherKeys}
              defaultBinding=""
              optional={false}
              finish={finish}
              disabled={busy}
              onRemove={otherKeys ? () => void removeOther() : undefined}
            />
          )}
        </div>
      ) : (
        <ShortcutKeysAtRest
          binding={binding.current_binding}
          alternate={otherKeys}
          name={translatedName}
          finish={finish}
          size={size}
          disabled={disabled}
          emptyLabel={optional ? t("common.off") : undefined}
          onClick={() => void startRecording("primary")}
          onClickAlternate={() => void startRecording("alternate")}
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
