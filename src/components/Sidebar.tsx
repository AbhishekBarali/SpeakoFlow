import React, { useState } from "react";
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
  ChevronLeft,
  ChevronRight,
} from "lucide-react";
import Logo from "./Logo";
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
  // The sidebar always opens expanded; the toggle only affects the current
  // session (no persistence), so the full rail is the default on every launch.
  const [collapsed, setCollapsed] = useState(false);

  const availableSections = Object.entries(SECTIONS_CONFIG)
    .filter(([_, config]) => config.enabled(settings))
    .map(([id, config]) => ({ id: id as SidebarSection, ...config }));

  const toggleLabel = collapsed ? t("sidebar.expand") : t("sidebar.collapse");

  return (
    <div
      className={`flex flex-col h-full border-e border-hairline bg-canvas-soft pt-5 pb-3 overflow-hidden transition-[width] duration-200 ease-out motion-reduce:transition-none ${
        collapsed ? "w-16 items-center px-2" : "w-52 px-3"
      }`}
    >
      {/* Brand — full lockup when expanded (teal mark + wordmark), mark only
          when collapsed. Sized to lead the rail: the logo is the one piece of
          brand color on the page, so it carries the hierarchy. */}
      <div
        className={`flex mb-7 ${
          collapsed ? "justify-center" : "items-center ps-2.5"
        }`}
      >
        {collapsed ? (
          <Logo className="text-accent h-6 w-auto shrink-0" />
        ) : (
          <div className="flex items-center gap-2 min-w-0">
            <Logo className="text-accent h-[26px] w-auto shrink-0" />
            <Wordmark className="text-[17px] truncate" />
          </div>
        )}
      </div>

      <nav className="flex flex-col w-full gap-px">
        {availableSections.map((section) => {
          const Icon = section.icon;
          const isActive = activeSection === section.id;

          return (
            <button
              key={section.id}
              type="button"
              aria-current={isActive ? "page" : undefined}
              title={t(section.labelKey)}
              className={`flex gap-2.5 items-center h-[34px] w-full rounded-lg cursor-pointer transition-colors duration-150 text-start ${
                collapsed ? "justify-center px-0" : "px-2.5"
              } ${
                isActive
                  ? "bg-accent/10 text-accent font-medium"
                  : "text-body font-normal hover:text-ink hover:bg-ink/4"
              }`}
              onClick={() => onSectionChange(section.id)}
            >
              <Icon
                width={16}
                height={16}
                className={`shrink-0 ${isActive ? "opacity-100" : "opacity-70"}`}
              />
              {!collapsed && (
                <span className="text-[13px] truncate">
                  {t(section.labelKey)}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      {/* Collapse control — a plain chevron pinned to the bottom, kept clear of
          the logo and nav. Points inward (left) to collapse, outward (right) to
          expand. */}
      <button
        type="button"
        onClick={() => setCollapsed((value) => !value)}
        aria-label={toggleLabel}
        aria-expanded={!collapsed}
        title={toggleLabel}
        className={`mt-auto flex items-center h-8 w-full rounded-lg cursor-pointer text-muted-soft transition-colors hover:text-ink hover:bg-ink/4 ${
          collapsed ? "justify-center" : "justify-start px-2.5"
        }`}
      >
        {collapsed ? (
          <ChevronRight width={16} height={16} />
        ) : (
          <ChevronLeft width={16} height={16} />
        )}
      </button>
    </div>
  );
};
