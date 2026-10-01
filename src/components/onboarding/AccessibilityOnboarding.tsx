import { useEffect, useState, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { platform } from "@tauri-apps/plugin-os";
import {
  checkAccessibilityPermission,
  requestAccessibilityPermission,
  checkMicrophonePermission,
  requestMicrophonePermission,
} from "tauri-plugin-macos-permissions-api";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { useSettingsStore } from "@/stores/settingsStore";
import { Keyboard, Mic, Check, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { StepHeading } from "./OnboardingFrame";
import "./onboarding.css";

interface AccessibilityOnboardingProps {
  onComplete: () => void;
}

type PermissionStatus = "checking" | "needed" | "waiting" | "granted";
type PermissionPlatform = "macos" | "windows" | "other";

interface PermissionsState {
  accessibility: PermissionStatus;
  microphone: PermissionStatus;
}

/** How long "Waiting…" runs before the reset option appears. */
const STUCK_AFTER_MS = 6000;

const AccessibilityOnboarding: React.FC<AccessibilityOnboardingProps> = ({
  onComplete,
}) => {
  const { t } = useTranslation();
  const refreshAudioDevices = useSettingsStore(
    (state) => state.refreshAudioDevices,
  );
  const refreshOutputDevices = useSettingsStore(
    (state) => state.refreshOutputDevices,
  );
  const [permissionPlatform, setPermissionPlatform] =
    useState<PermissionPlatform | null>(null);
  const [permissions, setPermissions] = useState<PermissionsState>({
    accessibility: "checking",
    microphone: "checking",
  });
  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const errorCountRef = useRef<number>(0);
  const MAX_POLLING_ERRORS = 3;
  // An update can leave a grant behind that System Settings still shows as on
  // but no longer matches this build's signature (issue #34), so waiting would
  // never end. After a few seconds of it, offer to clear the stale entry.
  const [accessibilityStuck, setAccessibilityStuck] = useState(false);
  const [resetState, setResetState] = useState<"idle" | "resetting" | "done">(
    "idle",
  );
  // macOS asks for the microphone exactly once. After an earlier "Don't Allow"
  // (or a grant an update made stale) the request shows nothing, so "Waiting…"
  // would never end and this screen has no way past it. After a few seconds,
  // offer System Settings and a reset that makes macOS ask again.
  const [microphoneStuck, setMicrophoneStuck] = useState(false);
  const [micResetState, setMicResetState] = useState<
    "idle" | "resetting" | "done"
  >("idle");

  const isMacOS = permissionPlatform === "macos";
  const isWindows = permissionPlatform === "windows";
  const showMicrophonePermission = isMacOS || isWindows;
  const showAccessibilityPermission = isMacOS;

  const allGranted = isMacOS
    ? permissions.accessibility === "granted" &&
      permissions.microphone === "granted"
    : isWindows
      ? permissions.microphone === "granted"
      : true;

  const completeOnboarding = useCallback(async () => {
    await Promise.all([refreshAudioDevices(), refreshOutputDevices()]);
    onComplete();
  }, [onComplete, refreshAudioDevices, refreshOutputDevices]);

  const hasWindowsMicrophoneAccess = useCallback(async (): Promise<boolean> => {
    const microphoneStatus =
      await commands.getWindowsMicrophonePermissionStatus();

    if (!microphoneStatus.supported) {
      return true;
    }

    return microphoneStatus.overall_access !== "denied";
  }, []);

  // Check platform and permission status on mount
  useEffect(() => {
    const currentPlatform = platform();
    const nextPlatform: PermissionPlatform =
      currentPlatform === "macos"
        ? "macos"
        : currentPlatform === "windows"
          ? "windows"
          : "other";

    setPermissionPlatform(nextPlatform);

    // Skip immediately on unsupported platforms
    if (nextPlatform === "other") {
      onComplete();
      return;
    }

    const checkInitial = async () => {
      if (nextPlatform === "macos") {
        try {
          const [accessibilityGranted, microphoneGranted] = await Promise.all([
            checkAccessibilityPermission(),
            checkMicrophonePermission(),
          ]);

          // If accessibility is granted, initialize Enigo and shortcuts
          if (accessibilityGranted) {
            try {
              await Promise.all([
                commands.initializeEnigo(),
                commands.initializeShortcuts(),
              ]);
            } catch (e) {
              console.warn("Failed to initialize after permission grant:", e);
            }
          }

          const newState: PermissionsState = {
            accessibility: accessibilityGranted ? "granted" : "needed",
            microphone: microphoneGranted ? "granted" : "needed",
          };

          setPermissions(newState);

          if (accessibilityGranted && microphoneGranted) {
            await completeOnboarding();
          }
        } catch (error) {
          console.error("Failed to check macOS permissions:", error);
          toast.error(t("onboarding.permissions.errors.checkFailed"));
          setPermissions({
            accessibility: "needed",
            microphone: "needed",
          });
        }

        return;
      }

      try {
        const microphoneGranted = await hasWindowsMicrophoneAccess();

        setPermissions({
          accessibility: "granted",
          microphone: microphoneGranted ? "granted" : "needed",
        });

        if (microphoneGranted) {
          await completeOnboarding();
        }
      } catch (error) {
        console.warn("Failed to check Windows microphone permissions:", error);
        setPermissions({
          accessibility: "granted",
          microphone: "granted",
        });
        await completeOnboarding();
      }
    };

    checkInitial();
  }, [completeOnboarding, hasWindowsMicrophoneAccess, onComplete, t]);

  // Polling for permissions after user clicks a button
  const startPolling = useCallback(() => {
    if (pollingRef.current || permissionPlatform === null) return;

    pollingRef.current = setInterval(async () => {
      try {
        if (permissionPlatform === "windows") {
          const microphoneGranted = await hasWindowsMicrophoneAccess();

          if (microphoneGranted) {
            setPermissions((prev) => ({ ...prev, microphone: "granted" }));

            if (pollingRef.current) {
              clearInterval(pollingRef.current);
              pollingRef.current = null;
            }

            await completeOnboarding();
          }

          errorCountRef.current = 0;
          return;
        }

        const [accessibilityGranted, microphoneGranted] = await Promise.all([
          checkAccessibilityPermission(),
          checkMicrophonePermission(),
        ]);

        setPermissions((prev) => {
          const newState = { ...prev };

          if (accessibilityGranted && prev.accessibility !== "granted") {
            newState.accessibility = "granted";
            // Initialize Enigo and shortcuts when accessibility is granted
            Promise.all([
              commands.initializeEnigo(),
              commands.initializeShortcuts(),
            ]).catch((e) => {
              console.warn("Failed to initialize after permission grant:", e);
            });
          }

          if (microphoneGranted && prev.microphone !== "granted") {
            newState.microphone = "granted";
          }

          return newState;
        });

        // If both granted, stop polling, refresh audio devices, and proceed
        if (accessibilityGranted && microphoneGranted) {
          if (pollingRef.current) {
            clearInterval(pollingRef.current);
            pollingRef.current = null;
          }
          await completeOnboarding();
        }

        // Reset error count on success
        errorCountRef.current = 0;
      } catch (error) {
        console.error("Error checking permissions:", error);
        errorCountRef.current += 1;

        if (errorCountRef.current >= MAX_POLLING_ERRORS) {
          // Stop polling after too many consecutive errors
          if (pollingRef.current) {
            clearInterval(pollingRef.current);
            pollingRef.current = null;
          }
          toast.error(t("onboarding.permissions.errors.checkFailed"));
        }
      }
    }, 1000);
  }, [completeOnboarding, hasWindowsMicrophoneAccess, permissionPlatform, t]);

  // Cleanup polling and timeouts on unmount
  useEffect(() => {
    return () => {
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
      }
    };
  }, []);

  useEffect(() => {
    if (permissions.accessibility !== "waiting") return;
    const timer = setTimeout(() => setAccessibilityStuck(true), STUCK_AFTER_MS);
    return () => clearTimeout(timer);
  }, [permissions.accessibility]);

  useEffect(() => {
    if (!isMacOS || permissions.microphone !== "waiting") return;
    const timer = setTimeout(() => setMicrophoneStuck(true), STUCK_AFTER_MS);
    return () => clearTimeout(timer);
  }, [isMacOS, permissions.microphone]);

  const handleGrantAccessibility = async () => {
    try {
      await requestAccessibilityPermission();
      setPermissions((prev) => ({ ...prev, accessibility: "waiting" }));
      startPolling();
    } catch (error) {
      console.error("Failed to request accessibility permission:", error);
      toast.error(t("onboarding.permissions.errors.requestFailed"));
    }
  };

  const handleResetAccessibility = async () => {
    setResetState("resetting");
    let reset: Awaited<
      ReturnType<typeof commands.resetMacosAccessibilityPermission>
    >;
    try {
      reset = await commands.resetMacosAccessibilityPermission();
    } catch (error) {
      reset = { status: "error", error: String(error) };
    }
    if (reset.status === "error") {
      console.error("Failed to reset accessibility permission:", reset.error);
      toast.error(t("onboarding.permissions.errors.resetFailed"));
      setResetState("idle");
      return;
    }
    try {
      // With the stale entry gone, this adds a fresh one for the running build,
      // so the user only has to flip a switch rather than add the app by hand.
      await requestAccessibilityPermission();
    } catch (error) {
      console.warn("Failed to re-request accessibility permission:", error);
    }
    const opened = await commands
      .openMacosAccessibilitySettings()
      .catch((error: unknown) => ({
        status: "error" as const,
        error: String(error),
      }));
    if (opened.status === "error") {
      console.warn("Failed to open Accessibility settings:", opened.error);
    }
    setPermissions((prev) => ({ ...prev, accessibility: "waiting" }));
    // The existing poll picks up the new grant, no relaunch needed. It is
    // restarted here in case repeated errors had stopped it.
    startPolling();
    setResetState("done");
  };

  const handleGrantMicrophone = async () => {
    try {
      if (isWindows) {
        await commands.openMicrophonePrivacySettings();
      } else {
        await requestMicrophonePermission();
      }

      setPermissions((prev) => ({ ...prev, microphone: "waiting" }));
      startPolling();
    } catch (error) {
      console.error("Failed to request microphone permission:", error);
      toast.error(t("onboarding.permissions.errors.requestFailed"));
    }
  };

  const handleOpenMicrophoneSettings = async () => {
    const opened = await commands
      .openMicrophonePrivacySettings()
      .catch((error: unknown) => ({
        status: "error" as const,
        error: String(error),
      }));
    if (opened.status === "error") {
      console.warn("Failed to open Microphone settings:", opened.error);
      toast.error(t("onboarding.permissions.errors.requestFailed"));
      return;
    }
    // The poll picks up the switch being turned on; restart it in case repeated
    // errors had stopped it.
    startPolling();
  };

  const handleResetMicrophone = async () => {
    setMicResetState("resetting");
    let reset: Awaited<
      ReturnType<typeof commands.resetMacosMicrophonePermission>
    >;
    try {
      reset = await commands.resetMacosMicrophonePermission();
    } catch (error) {
      reset = { status: "error", error: String(error) };
    }
    if (reset.status === "error") {
      console.error("Failed to reset microphone permission:", reset.error);
      toast.error(t("onboarding.permissions.errors.requestFailed"));
      setMicResetState("idle");
      return;
    }
    try {
      // With the old decision gone, this shows the system prompt again.
      await requestMicrophonePermission();
    } catch (error) {
      console.warn("Failed to re-request microphone permission:", error);
    }
    setPermissions((prev) => ({ ...prev, microphone: "waiting" }));
    startPolling();
    setMicResetState("done");
  };

  const isChecking =
    permissionPlatform === null ||
    (isMacOS &&
      permissions.accessibility === "checking" &&
      permissions.microphone === "checking") ||
    (isWindows && permissions.microphone === "checking");

  // Nothing to ask for, or still finding out: an empty sheet, which reads as
  // the page turning. A spinner and then an "All set" check flashed past on
  // every machine that already had the permissions, which is almost all of
  // them, and looked like something had gone wrong.
  if (isChecking || allGranted) {
    return <PermissionsSheet>{null}</PermissionsSheet>;
  }
  const status = (value: PermissionStatus) =>
    value === "granted" ? (
      <span className="inline-flex items-center gap-1.5 text-sm font-medium text-accent">
        <Check className="h-4 w-4" aria-hidden="true" />
        {t("onboarding.permissions.granted")}
      </span>
    ) : value === "waiting" ? (
      <span className="inline-flex items-center gap-1.5 text-sm text-muted">
        <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
        {t("onboarding.permissions.waiting")}
      </span>
    ) : null;

  // Show permissions request screen
  return (
    <PermissionsSheet>
      <div className="mx-auto flex w-full max-w-3xl flex-1 flex-col justify-center py-10">
        <StepHeading
          title={t("onboarding.permissions.title")}
          body={t("onboarding.permissions.description")}
        />
        <ul className="mt-8 divide-y divide-hairline rounded-2xl border border-hairline bg-surface elev-card">
          {showMicrophonePermission && (
            <li className="px-5 py-4">
              <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
                <span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-surface-strong text-ink">
                  <Mic
                    className="h-[1.125rem] w-[1.125rem]"
                    aria-hidden="true"
                  />
                </span>
                <div className="min-w-[12rem] flex-1">
                  <h2 className="text-sm font-medium text-ink">
                    {t("onboarding.permissions.microphone.title")}
                  </h2>
                  <p className="mt-0.5 text-[0.8125rem] text-muted">
                    {t("onboarding.permissions.microphone.description")}
                  </p>
                </div>
                {status(permissions.microphone) ?? (
                  <Button size="md" onClick={handleGrantMicrophone}>
                    {isWindows
                      ? t("accessibility.openSettings")
                      : t("onboarding.permissions.grant")}
                  </Button>
                )}
              </div>
              {isMacOS &&
                permissions.microphone === "waiting" &&
                (microphoneStuck || micResetState === "done") && (
                  <div className="mt-3 flex flex-wrap items-center gap-3 ps-[3.25rem]">
                    <p
                      className="min-w-0 flex-1 text-[0.8125rem] text-muted"
                      role="status"
                    >
                      {t("errors.micPermissionDenied.macos")}
                    </p>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={handleOpenMicrophoneSettings}
                    >
                      {t("accessibility.openSettings")}
                    </Button>
                    {microphoneStuck && (
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={handleResetMicrophone}
                        disabled={micResetState === "resetting"}
                      >
                        {t("onboarding.permissions.accessibility.reset")}
                      </Button>
                    )}
                  </div>
                )}
            </li>
          )}

          {showAccessibilityPermission && (
            <li className="px-5 py-4">
              <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
                <span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-surface-strong text-ink">
                  <Keyboard
                    className="h-[1.125rem] w-[1.125rem]"
                    aria-hidden="true"
                  />
                </span>
                <div className="min-w-[12rem] flex-1">
                  <h2 className="text-sm font-medium text-ink">
                    {t("onboarding.permissions.accessibility.title")}
                  </h2>
                  <p className="mt-0.5 text-[0.8125rem] text-muted">
                    {t("onboarding.permissions.accessibility.description")}
                  </p>
                </div>
                {status(permissions.accessibility) ?? (
                  <Button size="md" onClick={handleGrantAccessibility}>
                    {t("onboarding.permissions.grant")}
                  </Button>
                )}
              </div>
              {permissions.accessibility === "waiting" &&
                (accessibilityStuck || resetState === "done") && (
                  <div className="mt-3 flex flex-wrap items-center gap-3 ps-[3.25rem]">
                    <p
                      className="min-w-0 flex-1 text-[0.8125rem] text-muted"
                      role={resetState === "done" ? "status" : undefined}
                    >
                      {resetState === "done"
                        ? t("onboarding.permissions.accessibility.resetDone")
                        : t("onboarding.permissions.accessibility.stuckHint")}
                    </p>
                    {accessibilityStuck && (
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={handleResetAccessibility}
                        disabled={resetState === "resetting"}
                      >
                        {t("onboarding.permissions.accessibility.reset")}
                      </Button>
                    )}
                  </div>
                )}
            </li>
          )}
        </ul>
      </div>
    </PermissionsSheet>
  );
};

/** The same inset sheet the rest of onboarding sits on. */
const PermissionsSheet: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => (
  <div className="flex h-full min-h-0 flex-col bg-canvas-soft">
    <main className="flex min-h-0 flex-1 flex-col overflow-y-auto rounded-t-[1.25rem] border-t border-hairline bg-canvas px-6 elev-pane sm:px-10">
      {children}
    </main>
  </div>
);

export default AccessibilityOnboarding;
