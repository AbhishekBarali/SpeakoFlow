import React, { useEffect, useState, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  getKeyName,
  formatKeyCombination,
  hasMouseButton,
  normalizeKey,
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

interface GlobalShortcutInputProps {
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

export const GlobalShortcutInput: React.FC<GlobalShortcutInputProps> = ({
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
  const [keyPressed, setKeyPressed] = useState<string[]>([]);
  const [recordedKeys, setRecordedKeys] = useState<string[]>([]);
  const [editingShortcutId, setEditingShortcutId] = useState<string | null>(
    null,
  );
  // Which of the shortcut's two sets of keys is being recorded.
  const [editingSlot, setEditingSlot] = useState<"primary" | "alternate">(
    "primary",
  );
  const [originalBinding, setOriginalBinding] = useState<string>("");
  const shortcutRefs = useRef<Map<string, HTMLDivElement | null>>(new Map());
  const osType = useOsType();

  const bindings = getSetting("bindings") || {};
  const mouseButtons = getSetting("mouse_button_shortcuts") ?? false;

  // This engine cannot see a mouse button at all. Someone who turned mouse
  // buttons on and presses one here is told where they work, instead of
  // watching the editor ignore it.
  useMouseButtonsWhileRecording(editingShortcutId !== null, () => {
    if (mouseButtons) toast.info(t("shortcutEditor.mouseNeedsKeysEngine"));
  });

  useEffect(() => {
    // Only add event listeners when we're in editing mode
    if (editingShortcutId === null) return;

    let cleanup = false;

    // Keyboard event listeners
    const handleKeyDown = async (e: KeyboardEvent) => {
      if (cleanup) return;
      if (e.repeat) return; // ignore auto-repeat
      e.preventDefault();

      // Get the key with OS-specific naming and normalize it
      const rawKey = getKeyName(e, osType);
      const key = normalizeKey(rawKey);

      if (!keyPressed.includes(key)) {
        setKeyPressed((prev) => [...prev, key]);
        // Also add to recorded keys if not already there
        if (!recordedKeys.includes(key)) {
          setRecordedKeys((prev) => [...prev, key]);
        }
      }
    };

    const handleKeyUp = async (e: KeyboardEvent) => {
      if (cleanup) return;
      e.preventDefault();

      // Get the key with OS-specific naming and normalize it
      const rawKey = getKeyName(e, osType);
      const key = normalizeKey(rawKey);

      // Remove from currently pressed keys
      setKeyPressed((prev) => prev.filter((k) => k !== key));

      // If no keys are pressed anymore, commit the shortcut
      const updatedKeyPressed = keyPressed.filter((k) => k !== key);
      if (updatedKeyPressed.length === 0 && recordedKeys.length > 0) {
        // Create the shortcut string from all recorded keys
        // Sort keys so modifiers come first, then the main key
        const modifiers = [
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
        ];
        const sortedKeys = recordedKeys.sort((a, b) => {
          const aIsModifier = modifiers.includes(a.toLowerCase());
          const bIsModifier = modifiers.includes(b.toLowerCase());
          if (aIsModifier && !bIsModifier) return -1;
          if (!aIsModifier && bIsModifier) return 1;
          return 0;
        });
        const newShortcut = sortedKeys.join("+");
        const takenBy = editingShortcutId
          ? shortcutUsingKeys(
              bindings,
              newShortcut,
              editingShortcutId,
              editingSlot,
            )
          : null;

        if (editingShortcutId && takenBy) {
          // Nothing was changed: put the suspended shortcut back as it was.
          toast.error(
            takenBy === editingShortcutId
              ? t("settings.general.shortcut.errors.sameAsOther", {
                  keys: formatKeyCombination(newShortcut, osType),
                })
              : t("settings.general.shortcut.errors.inUse", {
                  name: t(
                    `settings.general.shortcut.bindings.${takenBy}.name`,
                    bindings[takenBy]?.name ?? takenBy,
                  ),
                  keys: formatKeyCombination(newShortcut, osType),
                }),
          );
          commands.resumeBinding(editingShortcutId).catch(console.error);
          setEditingShortcutId(null);
          setKeyPressed([]);
          setRecordedKeys([]);
          setOriginalBinding("");
        } else if (
          editingShortcutId &&
          bindings[editingShortcutId] &&
          editingSlot === "alternate"
        ) {
          try {
            // Registers both sets of keys again, which also ends the suspend.
            await updateAlternateBinding(editingShortcutId, newShortcut);
          } catch (error) {
            console.error("Failed to set the other keys:", error);
            toast.error(
              t("settings.general.shortcut.errors.set", {
                error: String(error),
              }),
            );
            commands.resumeBinding(editingShortcutId).catch(console.error);
          }
          setEditingShortcutId(null);
          setKeyPressed([]);
          setRecordedKeys([]);
          setOriginalBinding("");
        } else if (editingShortcutId && bindings[editingShortcutId]) {
          try {
            await updateBinding(editingShortcutId, newShortcut);
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
                await updateBinding(editingShortcutId, originalBinding);
              } catch (resetError) {
                console.error("Failed to reset binding:", resetError);
                toast.error(t("settings.general.shortcut.errors.reset"));
              }
            }
          }

          // Exit editing mode and reset states
          setEditingShortcutId(null);
          setKeyPressed([]);
          setRecordedKeys([]);
          setOriginalBinding("");
        }
      }
    };

    // Add click outside handler
    const handleClickOutside = async (e: MouseEvent) => {
      if (cleanup) return;
      const activeElement = shortcutRefs.current.get(editingShortcutId);
      // Judged by the path the click took when it happened: "Add another"
      // re-renders the editor during its own click, so by the time the click
      // reaches the window the button it landed on is no longer in the page.
      if (activeElement && !e.composedPath().includes(activeElement)) {
        // Cancel shortcut recording and restore original binding. Other keys
        // being recorded changed nothing, so the suspended shortcut only
        // needs registering again.
        if (editingShortcutId && editingSlot === "primary" && originalBinding) {
          try {
            await updateBinding(editingShortcutId, originalBinding);
          } catch (error) {
            console.error("Failed to restore original binding:", error);
            toast.error(t("settings.general.shortcut.errors.restore"));
          }
        } else if (editingShortcutId) {
          commands.resumeBinding(editingShortcutId).catch(console.error);
        }
        setEditingShortcutId(null);
        setKeyPressed([]);
        setRecordedKeys([]);
        setOriginalBinding("");
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
  }, [
    keyPressed,
    recordedKeys,
    editingShortcutId,
    editingSlot,
    bindings,
    originalBinding,
    updateBinding,
    updateAlternateBinding,
    osType,
  ]);

  // Start recording new keys for one of the shortcut's two sets
  const startRecording = async (
    id: string,
    slot: "primary" | "alternate" = "primary",
  ) => {
    if (editingShortcutId === id) return; // Already editing this shortcut

    // Suspend current binding to avoid firing while recording
    await commands.suspendBinding(id).catch(console.error);

    // Store the original binding to restore if canceled
    setOriginalBinding(
      slot === "primary" ? bindings[id]?.current_binding || "" : "",
    );
    setEditingSlot(slot);
    setEditingShortcutId(id);
    setKeyPressed([]);
    setRecordedKeys([]);
  };

  // Format the current shortcut keys being recorded
  const formatCurrentKeys = (): string => {
    if (recordedKeys.length === 0)
      return t("settings.general.shortcut.pressKeys");

    // Use the same formatting as the display to ensure consistency
    return formatKeyCombination(recordedKeys.join("+"), osType);
  };

  // Store references to shortcut elements
  const setShortcutRef = (id: string, ref: HTMLDivElement | null) => {
    shortcutRefs.current.set(id, ref);
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
  // This engine never listens for a mouse button, so one stored from the
  // other engine is not shown as working keys.
  const storedOther = binding.alternate_binding?.trim() ?? "";
  const otherKeys = hasMouseButton(storedOther) ? "" : storedOther;

  // Leave the editor without restoring anything: the action that follows
  // decides what the shortcut becomes (and registers it).
  const leaveEditor = () => {
    setEditingShortcutId(null);
    setKeyPressed([]);
    setRecordedKeys([]);
    setOriginalBinding("");
  };
  // The shortcut stays suspended and the next keys become its other keys.
  // Its own keys were not changed, so abandoning this only resumes it.
  const addAnother = () => {
    setEditingSlot("alternate");
    setOriginalBinding("");
    setKeyPressed([]);
    setRecordedKeys([]);
  };
  const removeOther = async () => {
    leaveEditor();
    try {
      await updateAlternateBinding(shortcutId, "");
    } catch (error) {
      console.error("Failed to remove the other keys:", error);
      toast.error(
        t("settings.general.shortcut.errors.set", { error: String(error) }),
      );
      commands.resumeBinding(shortcutId).catch(console.error);
    }
  };
  const useDefault = async () => {
    leaveEditor();
    await resetBinding(shortcutId);
  };
  const turnOff = async () => {
    const previous = originalBinding;
    leaveEditor();
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

  const control = (
    <div className="flex items-center gap-1">
      {editingShortcutId === shortcutId ? (
        <div
          ref={(ref) => setShortcutRef(shortcutId, ref)}
          className="flex flex-col items-end gap-0.5"
        >
          <RecordingKeys
            text={formatCurrentKeys()}
            finish={finish}
            size={size ?? "md"}
          />
          {editingSlot === "primary" ? (
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
          onClick={() => void startRecording(shortcutId)}
          onClickAlternate={() => void startRecording(shortcutId, "alternate")}
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
