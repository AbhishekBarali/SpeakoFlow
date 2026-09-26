import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  BookA,
  ChartColumn,
  ChevronsLeft,
  ChevronsRight,
  Cloud,
  Cpu,
  History,
  House,
  MessageCircle,
  Settings,
  Users,
  Wand2,
} from "lucide-react";
import DownloadIndicator from "./model-selector/DownloadIndicator";
import { useNavigation, type PageId } from "./shell/navigation";
import { useModelSlots } from "./shell/useModelSlots";
import { useSttLoadState } from "./shell/useSttStatus";

type NavIcon = React.ComponentType<{
  className?: string;
  size?: number | string;
  strokeWidth?: number | string;
}>;

interface NavItem {
  id: PageId;
  labelKey: string;
  icon: NavIcon;
}

/**
 * Primary navigation. Pages are the things you *do* (and look back at); the
 * one page that configures models sits below a rule, and everything else —
 * the long tail of preferences — lives behind Settings at the bottom, the way
 * the reference apps arrange it. The old rail was five settings pages, which
 * put a hundred options one click from the top and the product nowhere.
 *
 * Insights closes the list: it is where you look back, not where you work, so
 * the pages used every day come first.
 */
export const NAV_ITEMS: NavItem[] = [
  { id: "home", labelKey: "nav.home", icon: House },
  { id: "history", labelKey: "nav.history", icon: History },
  { id: "assistant", labelKey: "nav.assistant", icon: MessageCircle },
  { id: "meetings", labelKey: "nav.meetings", icon: Users },
  { id: "cleanup", labelKey: "nav.cleanup", icon: Wand2 },
  { id: "dictionary", labelKey: "nav.dictionary", icon: BookA },
  { id: "insights", labelKey: "nav.insights", icon: ChartColumn },
];

const MODELS_ITEM: NavItem = {
  id: "models",
  labelKey: "nav.models",
  icon: Cpu,
};

const NavButton: React.FC<{
  item: NavItem;
  active: boolean;
  collapsed: boolean;
  onClick: () => void;
}> = ({ item, active, collapsed, onClick }) => {
  const { t } = useTranslation();
  const Icon = item.icon;
  const label = t(item.labelKey);
  return (
    <button
      type="button"
      aria-current={active ? "page" : undefined}
      title={collapsed ? label : undefined}
      onClick={onClick}
      className={`group flex h-9 w-full cursor-pointer items-center gap-3 rounded-lg text-start transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${
        collapsed ? "justify-center px-0" : "px-2.5"
      } ${
        active
          ? "bg-accent/10 font-medium text-accent-strong"
          : "text-body hover:bg-ink/[0.045] hover:text-ink"
      }`}
    >
      <Icon
        size={17}
        strokeWidth={active ? 2 : 1.75}
        className={`shrink-0 transition-colors ${
          active ? "text-accent" : "text-muted group-hover:text-ink"
        }`}
      />
      {!collapsed && <span className="truncate text-[0.9375rem]">{label}</span>}
    </button>
  );
};

/** Speech-model status line: which model is listening, and whether it is up. */
const ListeningStatus: React.FC<{ collapsed: boolean }> = ({ collapsed }) => {
  const { t } = useTranslation();
  const { openModelSlot } = useNavigation();
  const slots = useModelSlots();
  const stt = slots.stt;
  const { state } = useSttLoadState();

  const isCloud = stt.where === "cloud";
  const tone = !stt.ready
    ? "bg-amber-500"
    : isCloud || state === "ready"
      ? "bg-emerald-500"
      : state === "loading"
        ? "bg-sky-500 animate-pulse"
        : state === "error"
          ? "bg-error"
          : "bg-muted-soft";

  const statusText = !stt.ready
    ? t("nav.status.setup")
    : isCloud
      ? t("nav.status.cloud", { provider: stt.providerLabel ?? "" })
      : state === "loading"
        ? t("nav.status.loading")
        : state === "error"
          ? t("nav.status.error")
          : state === "ready"
            ? t("nav.status.ready")
            : t("nav.status.idle");

  const name = stt.modelLabel ?? t("nav.status.noModel");
  const label = t("nav.status.label", { model: name, status: statusText });

  return (
    <button
      type="button"
      onClick={() => openModelSlot("stt")}
      title={label}
      aria-label={label}
      className={`flex w-full cursor-pointer items-center gap-2.5 rounded-lg text-start transition-colors hover:bg-ink/[0.045] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${
        collapsed ? "h-9 justify-center" : "px-2.5 py-2"
      }`}
    >
      <span className="relative grid h-4 w-4 shrink-0 place-items-center">
        {isCloud && !collapsed ? (
          <Cloud className="h-3.5 w-3.5 text-muted" aria-hidden="true" />
        ) : (
          <span className={`h-2 w-2 rounded-full ${tone}`} />
        )}
      </span>
      {!collapsed && (
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[0.8125rem] font-medium text-ink">
            {name}
          </span>
          <span className="block truncate text-xs text-muted">
            {statusText}
          </span>
        </span>
      )}
    </button>
  );
};

export const Sidebar: React.FC = () => {
  const { t } = useTranslation();
  const { page, navigateRoot, openSettings, settingsTab } = useNavigation();
  // Opens expanded every launch; collapsing is a per-session choice.
  const [collapsed, setCollapsed] = useState(false);
  const toggleLabel = collapsed ? t("sidebar.expand") : t("sidebar.collapse");

  return (
    <aside
      className={`relative z-20 flex h-full shrink-0 flex-col bg-canvas-soft pt-3 pb-3 transition-[width] duration-200 ease-out motion-reduce:transition-none ${
        collapsed ? "w-16 px-2" : "w-[14.5rem] px-3"
      }`}
    >
      <nav className="flex flex-col gap-0.5" aria-label={t("nav.label")}>
        {NAV_ITEMS.map((item) => (
          <NavButton
            key={item.id}
            item={item}
            active={page === item.id}
            collapsed={collapsed}
            onClick={() => navigateRoot(item.id)}
          />
        ))}
        <div className="mx-2.5 my-2 h-px bg-hairline-strong/70" />
        <NavButton
          item={MODELS_ITEM}
          active={page === "models"}
          collapsed={collapsed}
          onClick={() => navigateRoot("models")}
        />
      </nav>

      <div className="mt-auto flex flex-col gap-1">
        <div className={collapsed ? "flex justify-center pb-1" : "px-1 pb-1"}>
          <DownloadIndicator placement="start" compact={collapsed} />
        </div>
        <ListeningStatus collapsed={collapsed} />
        <div className="mx-2.5 my-1 h-px bg-hairline-strong/70" />
        <div
          className={`flex gap-1 ${collapsed ? "flex-col" : "items-center"}`}
        >
          <button
            type="button"
            onClick={() => openSettings()}
            aria-haspopup="dialog"
            aria-expanded={settingsTab !== null}
            title={collapsed ? t("nav.settings") : undefined}
            className={`group flex h-9 cursor-pointer items-center gap-3 rounded-lg text-body transition-colors hover:bg-ink/[0.045] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${
              collapsed ? "w-full justify-center" : "min-w-0 flex-1 px-2.5"
            }`}
          >
            <Settings
              size={17}
              strokeWidth={1.8}
              className="shrink-0 text-muted transition-transform duration-300 group-hover:rotate-45 group-hover:text-ink motion-reduce:transition-none"
            />
            {!collapsed && (
              <span className="truncate text-[0.9375rem]">
                {t("nav.settings")}
              </span>
            )}
          </button>
          <button
            type="button"
            onClick={() => setCollapsed((value) => !value)}
            aria-label={toggleLabel}
            aria-expanded={!collapsed}
            title={toggleLabel}
            className={`grid h-9 shrink-0 cursor-pointer place-items-center rounded-lg text-muted-soft transition-colors hover:bg-ink/[0.045] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${
              collapsed ? "w-full" : "w-9"
            }`}
          >
            {collapsed ? (
              <ChevronsRight className="h-4 w-4 rtl:rotate-180" />
            ) : (
              <ChevronsLeft className="h-4 w-4 rtl:rotate-180" />
            )}
          </button>
        </div>
      </div>
    </aside>
  );
};
