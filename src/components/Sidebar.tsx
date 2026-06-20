import React from "react";
import { useTranslation } from "react-i18next";
import {
  SlidersHorizontal,
  Box,
  Wrench,
  FlaskConical,
  History,
  Info,
  Sparkles,
  MessageCircle,
} from "lucide-react";
import Wordmark from "./Wordmark";
import { useSettings } from "../hooks/useSettings";
import {
  GeneralSettings,
  AdvancedSettings,
  HistorySettings,
  DebugSettings,
  AboutSettings,
  PostProcessingSettings,
  ModelsSettings,
  AssistantSettings,
} from "./settings";

export type SidebarSection = keyof typeof SECTIONS_CONFIG;

interface IconProps {
  width?: number | string;
  height?: number | string;
  size?: number | string;
  className?: string;
  [key: string]: any;
}

interface SectionConfig {
  labelKey: string;
  icon: React.ComponentType<IconProps>;
  component: React.ComponentType;
  enabled: (settings: any) => boolean;
}

export const SECTIONS_CONFIG = {
  general: {
    labelKey: "sidebar.general",
    icon: SlidersHorizontal,
    component: GeneralSettings,
    enabled: () => true,
  },
  models: {
    labelKey: "sidebar.models",
    icon: Box,
    component: ModelsSettings,
    enabled: () => true,
  },
  advanced: {
    labelKey: "sidebar.advanced",
    icon: Wrench,
    component: AdvancedSettings,
    enabled: () => true,
  },
  history: {
    labelKey: "sidebar.history",
    icon: History,
    component: HistorySettings,
    enabled: () => true,
  },
  postprocessing: {
    labelKey: "sidebar.postProcessing",
    icon: Sparkles,
    component: PostProcessingSettings,
    enabled: (settings) => settings?.post_process_enabled ?? false,
  },
  assistant: {
    labelKey: "sidebar.assistant",
    icon: MessageCircle,
    component: AssistantSettings,
    enabled: () => true,
  },
  debug: {
    labelKey: "sidebar.debug",
    icon: FlaskConical,
    component: DebugSettings,
    enabled: (settings) => settings?.debug_mode ?? false,
  },
  about: {
    labelKey: "sidebar.about",
    icon: Info,
    component: AboutSettings,
    enabled: () => true,
  },
} as const satisfies Record<string, SectionConfig>;

interface SidebarProps {
  activeSection: SidebarSection;
  onSectionChange: (section: SidebarSection) => void;
}

export const Sidebar: React.FC<SidebarProps> = ({
  activeSection,
  onSectionChange,
}) => {
  const { t } = useTranslation();
  const { settings } = useSettings();

  const availableSections = Object.entries(SECTIONS_CONFIG)
    .filter(([_, config]) => config.enabled(settings))
    .map(([id, config]) => ({ id: id as SidebarSection, ...config }));

  return (
    <div className="flex flex-col w-44 h-full border-e border-hairline bg-canvas-soft px-3 py-4">
      <Wordmark className="text-2xl mx-2 mb-5 mt-1" />
      <nav className="flex flex-col w-full gap-0.5">
        {availableSections.map((section) => {
          const Icon = section.icon;
          const isActive = activeSection === section.id;

          return (
            <button
              key={section.id}
              type="button"
              aria-current={isActive ? "page" : undefined}
              className={`flex gap-2.5 items-center px-3 py-2 w-full rounded-xl cursor-pointer transition-colors text-start ${
                isActive
                  ? "bg-surface text-ink font-medium border border-hairline shadow-[0_1px_2px_rgba(12,10,9,0.04)]"
                  : "text-muted hover:text-ink hover:bg-surface-strong border border-transparent"
              }`}
              onClick={() => onSectionChange(section.id)}
            >
              <Icon width={18} height={18} className="shrink-0" />
              <span className="text-sm truncate" title={t(section.labelKey)}>
                {t(section.labelKey)}
              </span>
            </button>
          );
        })}
      </nav>
    </div>
  );
};
