// Must be first: installs the fake Tauri backend before any module touches it.
import { previewParams } from "./mockBackend";

import React from "react";
import ReactDOM from "react-dom/client";
import { Toaster } from "sonner";
import "@fontsource-variable/inter";
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

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <div className="flex h-screen cursor-default flex-col overflow-hidden select-none">
      <Toaster theme="system" />
      <TitleBar />
      <MainShell initialNavigation={initialNavigation} />
    </div>
  </React.StrictMode>,
);
