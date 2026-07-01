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
import LogoLockup from "./LogoLockup";
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
      className={`flex flex-col h-full border-e border-hairline bg-canvas-soft py-4 overflow-hidden transition-[width] duration-200 ease-out motion-reduce:transition-none ${
        collapsed ? "w-16 items-center px-2" : "w-44 px-3"
      }`}
    >
      {/* Brand mark — full lockup when expanded, icon-only when collapsed. */}
      <div
        className={`flex mb-6 mt-1 ${
          collapsed ? "justify-center" : "items-center ms-1"
        }`}
      >
        {collapsed ? (
          <Logo className="text-ink h-6 w-auto shrink-0" />
        ) : (
          <LogoLockup iconClassName="h-6 w-auto" />
        )}
      </div>

      <nav className="flex flex-col w-full gap-0.5">
        {availableSections.map((section) => {
          const Icon = section.icon;
          const isActive = activeSection === section.id;

          return (
            <button
              key={section.id}
              type="button"
              aria-current={isActive ? "page" : undefined}
              title={t(section.labelKey)}
              className={`flex gap-2.5 items-center py-2 w-full rounded-xl cursor-pointer transition-colors text-start ${
                collapsed ? "justify-center px-0" : "px-3"
              } ${
                isActive
                  ? "bg-surface text-ink font-semibold border border-hairline shadow-[0_1px_2px_rgba(12,10,9,0.04)]"
                  : "text-muted font-medium hover:text-ink hover:bg-surface-strong border border-transparent"
              }`}
              onClick={() => onSectionChange(section.id)}
            >
              <Icon width={18} height={18} className="shrink-0" />
              {!collapsed && (
                <span className="text-sm truncate">{t(section.labelKey)}</span>
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
        className={`mt-auto flex items-center h-8 w-full rounded-lg cursor-pointer text-muted transition-colors hover:text-ink hover:bg-surface-strong ${
          collapsed ? "justify-center" : "justify-start px-3"
        }`}
      >
        {collapsed ? (
          <ChevronRight width={18} height={18} />
        ) : (
          <ChevronLeft width={18} height={18} />
        )}
      </button>
    </div>
  );
};
