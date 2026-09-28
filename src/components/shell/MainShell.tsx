import React, { useState } from "react";
import AccessibilityPermissions from "@/components/AccessibilityPermissions";
import { Sidebar } from "@/components/Sidebar";
import UpdateChecker from "@/components/update-checker";
import { FeedbackHost } from "@/components/feedback/FeedbackDialog";
import { SettingsDialog } from "@/components/settings-dialog/SettingsDialog";
import {
  INITIAL_NAVIGATION,
  NavigationProvider,
  type NavigationState,
} from "./navigation";
import { PageStack } from "./PageStack";
import { useSlotDataSync } from "./slotData";
import { useAutoSelectDownloadedModel } from "./useSttStatus";

/** Shell-wide effects that must run exactly once. */
const ShellEffects: React.FC = () => {
  useSlotDataSync();
  // Downloading a speech model switches to it once the file is on disk.
  useAutoSelectDownloadedModel(true);
  return null;
};

/**
 * The main window after onboarding: sidebar, the page stack, and the Settings
 * dialog. Split out of `App` so the browser preview (`src/preview`) renders the
 * exact same tree against a mocked backend.
 */
export const MainShell: React.FC<{ initialNavigation?: NavigationState }> = ({
  initialNavigation = INITIAL_NAVIGATION,
}) => {
  const [navigation, setNavigation] =
    useState<NavigationState>(initialNavigation);

  return (
    <NavigationProvider state={navigation} setState={setNavigation}>
      <ShellEffects />
      {/* The row carries the chrome colour; the content column is a sheet
          inset into it, so title bar + sidebar read as one frame and the page
          floats on top of it. */}
      <div className="flex flex-1 overflow-hidden bg-canvas-soft">
        <Sidebar />
        <main className="relative flex flex-1 flex-col overflow-hidden rounded-ss-[1.25rem] border-s border-t border-hairline bg-canvas elev-pane">
          <div className="mx-auto w-full max-w-5xl px-6 pt-5 empty:hidden sm:px-10">
            <AccessibilityPermissions />
          </div>
          <PageStack />
        </main>
      </div>
      <SettingsDialog />
      <FeedbackHost />
      {/* Background update checks and the tray's "Check for updates…". What
          they find is shown by the sidebar's UpdatePill and Settings → About. */}
      <UpdateChecker />
    </NavigationProvider>
  );
};
