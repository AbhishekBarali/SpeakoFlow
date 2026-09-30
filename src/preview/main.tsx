// Must be first: installs the fake Tauri backend before any module touches it.
import { previewParams } from "./mockBackend";

import React, { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import { Toaster } from "sonner";
import "@fontsource-variable/inter";
import "@fontsource/instrument-serif/400.css";
import "@fontsource/instrument-serif/400-italic.css";
import "../App.css";
import "../i18n";
import TitleBar from "@/components/TitleBar";
import { MainShell } from "@/components/shell/MainShell";
import {
  INITIAL_NAVIGATION,
  type ModelSlot,
  type NavigationState,
  type PageId,
  type SettingsTab,
} from "@/components/shell/navigation";
import {
  AccessibilityOnboarding,
  FinishStep,
  SetupStep,
  WelcomeStep,
  TourStep,
  VoicePrefetch,
  useOnboardingReplay,
  useSetupQueue,
} from "@/components/onboarding";
import { applyThemePreference, type ThemePreference } from "@/lib/theme";
import { watchScreenScale } from "@/lib/screenScale";
import { useModelStore } from "@/stores/modelStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { suppressCaretBrowsing } from "@/lib/caretBrowsing";
import { useUpdateStore } from "@/components/update-checker/updateStore";
import { useFeedbackDialog } from "@/components/feedback/feedbackStore";

suppressCaretBrowsing();

// Screenshots should not wait out the real 8-second first check.
if (previewParams.get("update") === "1") {
  void useUpdateStore.getState().check();
}
if (previewParams.get("feedback") === "1") {
  useFeedbackDialog.getState().show();
}

document.documentElement.dataset.platform = "windows";
applyThemePreference(
  (previewParams.get("theme") ?? "light") as ThemePreference,
);
watchScreenScale();
useModelStore.getState().initialize();
void useSettingsStore.getState().initialize();

const page = (previewParams.get("page") ?? "home") as PageId;
const initialNavigation: NavigationState = {
  ...INITIAL_NAVIGATION,
  page,
  modelSlot: (previewParams.get("tab") as ModelSlot | null) ?? "stt",
  settingsTab: (previewParams.get("settings") as SettingsTab | null) ?? null,
  // `?from=home` shows the back link a deep link would leave.
  history: previewParams.get("from")
    ? [{ page: previewParams.get("from") as PageId, modelSlot: null }]
    : [],
};

type FlowStep =
  | "welcome"
  | "accessibility"
  | "setup"
  | "tour"
  | "finish"
  | "done";

/** `?onboarding=welcome|setup|tour|finish` walks first-run setup from that step,
 *  with `&scene=0…2` opening the tour on one scene. Without `?onboarding`, the
 *  shell's Settings → General → Show onboarding again starts the flow too. */
const OnboardingPreview: React.FC<{ from: FlowStep }> = ({ from }) => {
  const [step, setStep] = useState<FlowStep>(from);
  const [chooseModels, setChooseModels] = useState(false);
  // Settings → General → Show onboarding again, as App handles it.
  const replays = useOnboardingReplay((state) => state.requests);
  useEffect(() => {
    if (replays === 0) return;
    useSetupQueue.getState().clearSettled();
    setChooseModels(false);
    setStep("welcome");
  }, [replays]);
  if (step === "welcome") {
    return <WelcomeStep onContinue={() => setStep("setup")} />;
  }
  if (step === "accessibility") {
    return <AccessibilityOnboarding onComplete={() => setStep("setup")} />;
  }
  if (step === "setup") {
    return <SetupStep onContinue={() => setStep("tour")} />;
  }
  if (step === "tour") {
    return (
      <TourStep
        initialIndex={Number(previewParams.get("scene") ?? 0)}
        onDone={() => setStep("finish")}
      />
    );
  }
  if (step === "finish") {
    return (
      <FinishStep
        onDone={(choose) => {
          setChooseModels(!!choose);
          setStep("done");
        }}
      />
    );
  }
  return (
    <MainShell
      initialNavigation={
        chooseModels
          ? { ...initialNavigation, page: "models", modelSlot: "stt" }
          : initialNavigation
      }
    />
  );
};

const onboarding = previewParams.get("onboarding") as FlowStep | null;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <div className="flex h-screen cursor-default flex-col overflow-hidden select-none">
      <Toaster theme="system" />
      <TitleBar />
      <div className="flex min-h-0 flex-1 flex-col">
        <OnboardingPreview from={onboarding ?? "done"} />
      </div>
      <VoicePrefetch />
    </div>
  </React.StrictMode>,
);
