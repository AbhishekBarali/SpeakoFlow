import React from "react";
import { useTranslation } from "react-i18next";
import { CircleArrowUp, LoaderCircle, RotateCw } from "lucide-react";
import { useNavigation } from "../shell/navigation";
import { PILL_PHASES, useUpdateStore } from "./updateStore";

/**
 * The one place an available update is announced: a quiet row at the bottom
 * of the sidebar that stays until the update is installed. No pop-up, no
 * badge on every page — but also never hidden, which is what the old
 * `className="hidden"` checker got wrong. Clicking it opens Settings → About,
 * where the release notes and the install button are.
 */
export const UpdatePill: React.FC<{ collapsed: boolean }> = ({ collapsed }) => {
  const { t } = useTranslation();
  const { openSettings } = useNavigation();
  const phase = useUpdateStore((s) => s.phase);
  const available = useUpdateStore((s) => s.available);
  const progress = useUpdateStore((s) => s.progress);
  const restart = useUpdateStore((s) => s.restart);

  if (!PILL_PHASES.includes(phase)) return null;

  const busy =
    phase === "downloading" ||
    phase === "installing" ||
    phase === "savingInstaller";

  const title =
    phase === "restartRequired"
      ? t("updates.pill.restart")
      : busy
        ? progress !== null && phase !== "installing"
          ? t("updates.pill.progress", { progress })
          : t("updates.pill.working")
        : phase === "installerSaved"
          ? t("updates.pill.installerReady")
          : t("updates.pill.available");
  const subtitle = available
    ? t("updates.pill.version", { version: available.version })
    : null;
  const label = subtitle ? `${title}. ${subtitle}` : title;

  const Icon = busy
    ? LoaderCircle
    : phase === "restartRequired"
      ? RotateCw
      : CircleArrowUp;

  return (
    <button
      type="button"
      onClick={() =>
        phase === "restartRequired" ? void restart() : openSettings("about")
      }
      title={collapsed ? label : undefined}
      aria-label={label}
      className={`relative flex w-full cursor-pointer items-center gap-2.5 overflow-hidden rounded-lg border border-accent/25 bg-accent/[0.07] text-start transition-colors hover:bg-accent/[0.12] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${
        collapsed ? "h-9 justify-center" : "px-2.5 py-2"
      }`}
    >
      <Icon
        className={`h-4 w-4 shrink-0 text-accent ${busy ? "animate-spin motion-reduce:animate-none" : ""}`}
        aria-hidden="true"
      />
      {!collapsed && (
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[0.8125rem] font-medium text-ink">
            {title}
          </span>
          {subtitle && (
            <span className="block truncate text-xs text-muted">
              {subtitle}
            </span>
          )}
        </span>
      )}
      {busy && progress !== null && (
        <span
          aria-hidden="true"
          className="absolute inset-x-0 bottom-0 h-0.5 bg-accent/15"
        >
          <span
            className="block h-full bg-accent transition-[width] duration-300"
            style={{ width: `${progress}%` }}
          />
        </span>
      )}
    </button>
  );
};
