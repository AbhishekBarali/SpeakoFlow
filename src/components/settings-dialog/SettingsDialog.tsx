import React, { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  FlaskConical,
  Headphones,
  Info,
  Keyboard,
  ShieldCheck,
  SlidersHorizontal,
  TextCursorInput,
  Wrench,
} from "lucide-react";
import { Dialog, DialogCloseButton } from "@/components/ui/Dialog";
import { InfoTip } from "@/components/ui/InfoTip";
import { useSettings } from "@/hooks/useSettings";
import { useNavigation, type SettingsTab } from "@/components/shell/navigation";
import {
  AboutTab,
  AdvancedTab,
  AudioTab,
  DebugTab,
  DictationTab,
  GeneralTab,
  PrivacyTab,
  ShortcutsTab,
} from "./tabs";

type TabIcon = React.ComponentType<{
  className?: string;
  size?: number | string;
  strokeWidth?: number | string;
}>;

interface TabDef {
  id: SettingsTab;
  icon: TabIcon;
  component: React.ComponentType;
  /** One sentence of context, behind the (i) next to the tab's title. */
  intro?: string;
}

const TABS: TabDef[] = [
  { id: "general", icon: SlidersHorizontal, component: GeneralTab },
  {
    id: "shortcuts",
    icon: Keyboard,
    component: ShortcutsTab,
    intro: "settingsDialog.shortcuts.intro",
  },
  { id: "audio", icon: Headphones, component: AudioTab },
  {
    id: "dictation",
    icon: TextCursorInput,
    component: DictationTab,
    intro: "settingsDialog.dictation.intro",
  },
  {
    id: "privacy",
    icon: ShieldCheck,
    component: PrivacyTab,
    intro: "settingsDialog.privacy.intro",
  },
  { id: "advanced", icon: Wrench, component: AdvancedTab },
  { id: "about", icon: Info, component: AboutTab },
  {
    id: "debug",
    icon: FlaskConical,
    component: DebugTab,
    intro: "sectionSubtitles.debug",
  },
];

/**
 * Settings, as a window over the app rather than a page in it.
 *
 * Everything that configures the *app* — shortcuts, microphone, how text is
 * typed, storage, updates — lives here, grouped into a short list of tabs, so
 * the sidebar can be about the things you actually do. Opened from the gear at
 * the bottom of the sidebar, or deep-linked to a tab from anywhere
 * (`openSettings("shortcuts")`).
 */
export const SettingsDialog: React.FC = () => {
  const { t } = useTranslation();
  const { settingsTab, openSettings, closeSettings } = useNavigation();
  const { settings } = useSettings();
  const bodyRef = useRef<HTMLDivElement>(null);
  const debugMode = settings?.debug_mode ?? false;

  const tabs = TABS.filter((tab) => tab.id !== "debug" || debugMode);
  const active =
    tabs.find((tab) => tab.id === settingsTab) ?? tabs[0] ?? TABS[0];
  const ActiveTab = active.component;

  useEffect(() => {
    bodyRef.current?.scrollTo({ top: 0 });
  }, [active.id]);

  // Leaving debug mode while its tab is open falls back to General.
  useEffect(() => {
    if (settingsTab === "debug" && !debugMode) openSettings("general");
  }, [settingsTab, debugMode, openSettings]);

  const titleId = "settings-dialog-title";

  return (
    <Dialog
      open={settingsTab !== null}
      onClose={closeSettings}
      size="settings"
      bare
      labelledBy={titleId}
      bodyClassName=""
    >
      <nav
        aria-label={t("settingsDialog.title")}
        className="flex w-[13.5rem] shrink-0 flex-col gap-0.5 overflow-y-auto border-e border-hairline bg-canvas-soft px-3 pt-6 pb-3"
      >
        <h2
          id={titleId}
          className="mb-4 px-2.5 font-display text-[1.25rem] text-ink"
        >
          {t("settingsDialog.title")}
        </h2>
        {tabs.map((tab) => {
          const Icon = tab.icon;
          const selected = tab.id === active.id;
          return (
            <button
              key={tab.id}
              type="button"
              aria-current={selected ? "page" : undefined}
              data-autofocus={selected ? "" : undefined}
              onClick={() => openSettings(tab.id)}
              className={`flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-lg px-2.5 text-start text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${
                selected
                  ? "bg-ink/[0.07] font-medium text-ink"
                  : "text-body hover:bg-ink/[0.045] hover:text-ink"
              }`}
            >
              <Icon
                size={16}
                strokeWidth={selected ? 2 : 1.75}
                className={selected ? "text-ink" : "text-muted"}
              />
              <span className="truncate">
                {t(`settingsDialog.tabs.${tab.id}`)}
              </span>
            </button>
          );
        })}
      </nav>

      <section className="flex min-w-0 flex-1 flex-col">
        <header className="flex shrink-0 items-center justify-between gap-4 px-8 pt-6 pb-5">
          <div className="flex min-w-0 items-center gap-1.5">
            <h3 className="font-display text-[1.375rem] text-ink">
              {t(`settingsDialog.tabs.${active.id}`)}
            </h3>
            {active.intro && (
              <InfoTip text={t(active.intro)} size="md" className="mt-1" />
            )}
          </div>
          <DialogCloseButton
            onClick={closeSettings}
            label={t("common.close")}
          />
        </header>
        <div
          ref={bodyRef}
          className="min-h-0 flex-1 space-y-7 overflow-y-auto px-8 pb-10"
        >
          <ActiveTab key={active.id} />
        </div>
      </section>
    </Dialog>
  );
};
