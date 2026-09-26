import React from "react";
import { Info, TriangleAlert } from "lucide-react";
import type { SettingIcon } from "./tones";

type CalloutTone = "info" | "warning" | "neutral";

const TONES: Record<CalloutTone, { box: string; icon: string }> = {
  info: {
    box: "border-sky-500/25 bg-sky-500/8",
    icon: "text-sky-600 dark:text-sky-400",
  },
  warning: {
    box: "border-amber-500/30 bg-amber-500/8",
    icon: "text-amber-600 dark:text-amber-400",
  },
  neutral: {
    box: "border-hairline bg-surface-strong/50",
    icon: "text-muted",
  },
};

/**
 * A one-line explanation that sits in the flow of a page — "AI cleanup is off",
 * "this uses the assistant's model" — with an optional action at the end. For
 * the things a user needs to know to make sense of the controls around it.
 */
export const Callout: React.FC<{
  tone?: CalloutTone;
  icon?: SettingIcon;
  children: React.ReactNode;
  action?: React.ReactNode;
  className?: string;
}> = ({ tone = "info", icon, children, action, className = "" }) => {
  const Icon = icon ?? (tone === "warning" ? TriangleAlert : Info);
  const styles = TONES[tone];
  return (
    <div
      className={`flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border px-4 py-3 ${styles.box} ${className}`}
    >
      <Icon
        className={`h-4 w-4 shrink-0 ${styles.icon}`}
        size={16}
        aria-hidden="true"
      />
      <div className="min-w-[12rem] flex-1 text-sm leading-relaxed text-body">
        {children}
      </div>
      {action && <div className="shrink-0">{action}</div>}
    </div>
  );
};
