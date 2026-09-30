import { type ReactNode, useEffect, useState, useRef } from "react";
import { toast, Toaster } from "sonner";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { platform } from "@tauri-apps/plugin-os";
import {
  checkAccessibilityPermission,
  checkMicrophonePermission,
} from "tauri-plugin-macos-permissions-api";
import { ModelStateEvent, RecordingErrorEvent } from "./lib/types/events";
import "./App.css";
import {
  AccessibilityOnboarding,
  FinishStep,
  SetupStep,
  WelcomeStep,
  TourStep,
  VoicePrefetch,
  hasCompletedOnboarding,
  markOnboardingComplete,
  useOnboardingReplay,
  useSetupQueue,
} from "./components/onboarding";
import TitleBar from "./components/TitleBar";
import { MainShell } from "./components/shell/MainShell";
import {
  INITIAL_NAVIGATION,
  type NavigationState,
} from "./components/shell/navigation";
import { useSettings } from "./hooks/useSettings";
import { useSettingsStore } from "./stores/settingsStore";
import { commands } from "@/bindings";
import { insertDictation } from "@/lib/insertDictation";
import { getLanguageDirection, initializeRTL } from "@/lib/utils/rtl";
import {
  applyThemePreference,
  watchSystemTheme,
  type ThemePreference,
} from "@/lib/theme";

type OnboardingStep =
  | "welcome"
  | "accessibility"
  | "setup"
  | "tour"
  | "finish"
  | "done";

// Force the full onboarding flow on every launch so it can be tested
// repeatedly. This is intentionally gated to dev builds only
// (`import.meta.env.DEV`): during `tauri dev` the wizard shows every launch
// for easy iteration, while compiled/release builds fall back to the real
// first-run detection in `checkOnboardingStatus` (show onboarding only when
// no model is installed yet and setup has never been finished).
const FORCE_ONBOARDING = import.meta.env.DEV;

function App() {
  const { t, i18n } = useTranslation();
  const [onboardingStep, setOnboardingStep] = useState<OnboardingStep | null>(
    null,
  );
  // Track if this is a returning user who just needs to grant permissions
  // (vs a new user who needs full onboarding including model selection)
  const [isReturningUser, setIsReturningUser] = useState(false);
  // Setup was left with "Set up models later": the app opens on Models.
  const [skippedModels, setSkippedModels] = useState(false);
  const [shellNavigation, setShellNavigation] =
    useState<NavigationState>(INITIAL_NAVIGATION);
  const { settings, updateSetting } = useSettings();
  const direction = getLanguageDirection(i18n.language);
  const refreshAudioDevices = useSettingsStore(
    (state) => state.refreshAudioDevices,
  );
  const refreshOutputDevices = useSettingsStore(
    (state) => state.refreshOutputDevices,
  );
  const hasCompletedPostOnboardingInit = useRef(false);
  const replayRequests = useOnboardingReplay((state) => state.requests);

  useEffect(() => {
    checkOnboardingStatus();
  }, []);

  // Settings → General → "Show onboarding again": the whole first-run flow,
  // from the welcome screen, exactly as a new install sees it. Nothing is
  // reset underneath it: models on disk stay installed (setup shows them as
  // installed and offers Continue), downloads in flight keep going, and the
  // completion flag is only rewritten when the replay reaches the end.
  useEffect(() => {
    if (replayRequests === 0) return;
    useSetupQueue.getState().clearSettled();
    setSkippedModels(false);
    setIsReturningUser(false);
    setShellNavigation(INITIAL_NAVIGATION);
    setOnboardingStep("welcome");
  }, [replayRequests]);

  // Initialize RTL direction when language changes
  useEffect(() => {
    initializeRTL(i18n.language);
  }, [i18n.language]);

  // Apply the appearance preference (light / dark / system) to <html>. The
  // CSS reacts to the resolved data-theme attribute. While the preference is
  // "system", watchSystemTheme keeps every main-window surface — including
  // onboarding and its loading states — aligned with live OS theme changes.
  const themePreference = (settings?.theme ?? "light") as ThemePreference;
  useEffect(() => {
    applyThemePreference(themePreference);
  }, [themePreference]);
  useEffect(() => watchSystemTheme(() => themePreference), [themePreference]);

  // Initialize Enigo, shortcuts, and refresh audio devices when main app loads
  useEffect(() => {
    if (onboardingStep === "done" && !hasCompletedPostOnboardingInit.current) {
      hasCompletedPostOnboardingInit.current = true;
      Promise.all([
        commands.initializeEnigo(),
        commands.initializeShortcuts(),
      ]).catch((e) => {
        console.warn("Failed to initialize:", e);
      });
      refreshAudioDevices();
      refreshOutputDevices();
    }
  }, [onboardingStep, refreshAudioDevices, refreshOutputDevices]);

  // Handle keyboard shortcuts for debug mode toggle
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      // Check for Ctrl+Shift+D (Windows/Linux) or Cmd+Shift+D (macOS)
      const isDebugShortcut =
        event.shiftKey &&
        event.key.toLowerCase() === "d" &&
        (event.ctrlKey || event.metaKey);

      if (isDebugShortcut) {
        event.preventDefault();
        const currentDebugMode = settings?.debug_mode ?? false;
        updateSetting("debug_mode", !currentDebugMode);
      }
    };

    // Add event listener when component mounts
    document.addEventListener("keydown", handleKeyDown);

    // Cleanup event listener when component unmounts
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [settings?.debug_mode, updateSetting]);

  // Listen for recording errors from the backend and show a toast
  useEffect(() => {
    const unlisten = listen<RecordingErrorEvent>("recording-error", (event) => {
      const { error_type, detail } = event.payload;

      if (error_type === "microphone_permission_denied") {
        const currentPlatform = platform();
        const platformKey = `errors.micPermissionDenied.${currentPlatform}`;
        const description = t(platformKey, {
          defaultValue: t("errors.micPermissionDenied.generic"),
        });
        toast.error(t("errors.micPermissionDeniedTitle"), { description });
      } else if (error_type === "no_input_device") {
        toast.error(t("errors.noInputDeviceTitle"), {
          description: t("errors.noInputDevice"),
        });
      } else {
        toast.error(
          t("errors.recordingFailed", { error: detail ?? "Unknown error" }),
        );
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  // A dictation made while this window is in front comes here instead of as a
  // paste (see `insertDictation`), and lands at the caret.
  useEffect(() => {
    const unlisten = listen<string>("dictation-into-focus", (event) => {
      if (!insertDictation(event.payload ?? "")) {
        console.info("Dictation finished with no text field focused");
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Listen for paste failures and show a toast.
  // The technical error detail is logged to speakoflow.log on the Rust side
  // (see actions.rs `error!("Failed to paste transcription: ...")`),
  // so we show a localized, user-friendly message here instead of the raw error.
  useEffect(() => {
    const unlisten = listen("paste-error", () => {
      toast.error(t("errors.pasteFailedTitle"), {
        description: t("errors.pasteFailed"),
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  // Listen for model loading failures and show a toast
  useEffect(() => {
    const unlisten = listen<ModelStateEvent>("model-state-changed", (event) => {
      if (event.payload.event_type === "loading_failed") {
        toast.error(
          t("errors.modelLoadFailed", {
            model:
              event.payload.model_name || t("errors.modelLoadFailedUnknown"),
          }),
          {
            description: event.payload.error,
          },
        );
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [t]);

  const revealMainWindowForPermissions = async () => {
    try {
      await commands.showMainWindowCommand();
    } catch (e) {
      console.warn("Failed to show main window for permission onboarding:", e);
    }
  };

  const checkOnboardingStatus = async () => {
    try {
      if (FORCE_ONBOARDING) {
        setIsReturningUser(false);
        setOnboardingStep("welcome");
        return;
      }
      // Anyone who can already dictate is a returning user: a local speech
      // model on disk, speech set to a cloud service, or setup finished
      // before (models chosen for later). Only a machine with none of these
      // is new, which also keeps an upgrade from sending a cloud user through
      // setup and switching their providers to on-device models.
      const result = await commands.hasAnyModelsAvailable();
      const hasModels = result.status === "ok" && result.data;
      const settingsResult = hasModels
        ? null
        : await commands.getAppSettings().catch(() => null);
      const dictatesInCloud =
        settingsResult?.status === "ok" &&
        settingsResult.data.stt_engine_mode === "cloud";
      const currentPlatform = platform();

      if (hasModels || dictatesInCloud || hasCompletedOnboarding()) {
        // Returning user - check if they need to grant permissions first
        setIsReturningUser(true);

        if (currentPlatform === "macos") {
          try {
            const [hasAccessibility, hasMicrophone] = await Promise.all([
              checkAccessibilityPermission(),
              checkMicrophonePermission(),
            ]);
            if (!hasAccessibility || !hasMicrophone) {
              await revealMainWindowForPermissions();
              setOnboardingStep("accessibility");
              return;
            }
          } catch (e) {
            console.warn("Failed to check macOS permissions:", e);
            // If we can't check, proceed to main app and let them fix it there
          }
        }

        if (currentPlatform === "windows") {
          try {
            const microphoneStatus =
              await commands.getWindowsMicrophonePermissionStatus();
            if (
              microphoneStatus.supported &&
              microphoneStatus.overall_access === "denied"
            ) {
              await revealMainWindowForPermissions();
              setOnboardingStep("accessibility");
              return;
            }
          } catch (e) {
            console.warn("Failed to check Windows microphone permissions:", e);
            // If we can't check, proceed to main app and let them fix it there
          }
        }

        setOnboardingStep("done");
      } else {
        // New user - start full onboarding
        setIsReturningUser(false);
        setOnboardingStep("welcome");
      }
    } catch (error) {
      console.error("Failed to check onboarding status:", error);
      setOnboardingStep("accessibility");
    }
  };

  const handleAccessibilityComplete = () => {
    // Returning users already have models, skip to main app
    // New users go through setup
    setOnboardingStep(isReturningUser ? "done" : "setup");
  };

  const handleSetupComplete = ({
    skippedModels: skipped,
  }: {
    skippedModels: boolean;
  }) => {
    setSkippedModels(skipped);
    setOnboardingStep("tour");
  };

  const handleFinish = (chooseModels = skippedModels) => {
    markOnboardingComplete();
    setShellNavigation(
      chooseModels
        ? { ...INITIAL_NAVIGATION, page: "models", modelSlot: "stt" }
        : INITIAL_NAVIGATION,
    );
    setOnboardingStep("done");
  };

  // The window has no native chrome (see lib.rs), so the TitleBar renders on
  // every screen and the body swaps underneath it. This keeps the window
  // draggable/closable during onboarding too.
  let body: ReactNode = null;
  if (onboardingStep === "welcome") {
    body = (
      <div className="flex-1 min-h-0">
        <WelcomeStep onContinue={() => setOnboardingStep("accessibility")} />
      </div>
    );
  } else if (onboardingStep === "accessibility") {
    body = (
      <div className="flex-1 min-h-0">
        <AccessibilityOnboarding onComplete={handleAccessibilityComplete} />
      </div>
    );
  } else if (onboardingStep === "setup") {
    body = (
      <div className="flex-1 min-h-0">
        <SetupStep onContinue={handleSetupComplete} />
      </div>
    );
  } else if (onboardingStep === "tour") {
    body = (
      <div className="flex-1 min-h-0">
        <TourStep onDone={() => setOnboardingStep("finish")} />
      </div>
    );
  } else if (onboardingStep === "finish") {
    body = (
      <div className="flex-1 min-h-0">
        <FinishStep onDone={handleFinish} />
      </div>
    );
  } else if (onboardingStep === "done") {
    body = <MainShell initialNavigation={shellNavigation} />;
  }

  return (
    <div
      dir={direction}
      className="h-screen flex flex-col select-none cursor-default overflow-hidden"
    >
      <Toaster
        theme="system"
        toastOptions={{
          unstyled: true,
          classNames: {
            toast:
              "bg-surface border border-hairline rounded-xl shadow-lg px-4 py-3 flex items-center gap-3 text-sm",
            title: "font-medium",
            description: "text-muted",
          },
        }}
      />
      <TitleBar />
      {body}
      {/* Outlives the setup screens: the voice keeps loading after them. */}
      <VoicePrefetch />
    </div>
  );
}

export default App;
