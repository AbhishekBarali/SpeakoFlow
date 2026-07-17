import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { type } from "@tauri-apps/plugin-os";
import {
  checkScreenRecordingPermission,
  requestScreenRecordingPermission,
} from "tauri-plugin-macos-permissions-api";

/**
 * macOS-only prompt to grant Screen Recording permission for the assistant's
 * screen-vision feature.
 *
 * On macOS, screen capture (xcap) silently returns an empty/black frame — or
 * fails with a generic capture error — unless the app holds the Screen
 * Recording TCC grant, and that grant only takes effect after the app is
 * relaunched. So the copy asks the user to grant it in System Settings and
 * restart. This is a no-op on Windows/Linux (screen capture needs no such
 * permission there): the component renders nothing off macOS and also nothing
 * once the permission is already granted.
 *
 * Intended to be rendered inside the Screen vision settings group, only when
 * screen access is enabled (see AssistantSettings).
 */
const ScreenRecordingPermission: React.FC = () => {
  const { t } = useTranslation();
  // Start "granted" so nothing flashes on non-macOS or before the first check.
  const [granted, setGranted] = useState<boolean>(true);

  const isMacOS = type() === "macos";

  useEffect(() => {
    if (!isMacOS) return;

    let cancelled = false;
    const check = async (): Promise<void> => {
      try {
        const ok = await checkScreenRecordingPermission();
        if (!cancelled) setGranted(ok);
      } catch (error) {
        console.error("Error checking screen recording permission:", error);
      }
    };

    check();
    // Re-check when the user returns to the window (e.g. after toggling the
    // permission in System Settings) so the card can clear itself without a
    // manual refresh in the cases where the OS reports the grant live.
    const onFocus = () => check();
    window.addEventListener("focus", onFocus);
    return () => {
      cancelled = true;
      window.removeEventListener("focus", onFocus);
    };
  }, [isMacOS]);

  // Nothing to do off macOS or once the permission is held.
  if (!isMacOS || granted) return null;

  const handleClick = async (): Promise<void> => {
    try {
      await requestScreenRecordingPermission();
    } catch (error) {
      console.error("Error requesting screen recording permission:", error);
    }
  };

  return (
    <div className="p-4 w-full max-w-2xl rounded-xl border border-hairline bg-surface">
      <div className="flex justify-between items-center gap-3">
        <p className="text-sm font-medium">
          {t("settings.assistant.vision.screenRecording.description")}
        </p>
        <button
          onClick={handleClick}
          className="min-h-10 px-3 py-1.5 text-[13px] font-medium bg-accent text-on-primary hover:bg-accent-strong rounded-lg cursor-pointer transition-colors whitespace-nowrap"
        >
          {t("settings.assistant.vision.screenRecording.grant")}
        </button>
      </div>
    </div>
  );
};

export default ScreenRecordingPermission;
