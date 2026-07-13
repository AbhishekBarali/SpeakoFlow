import React from "react";
import type { SettingIcon } from "./tones";

interface SettingsGroupProps {
  title?: string;
  description?: string;
  /** Optional accent icon shown in the header tile before the title. */
  icon?: SettingIcon;
  children: React.ReactNode;
}

export const SettingsGroup: React.FC<SettingsGroupProps> = ({
  title,
  description,
  icon: Icon,
  children,
}) => {
  return (
    <div className="space-y-2.5">
      {title && (
        <div className="flex items-start gap-2 px-1">
          {Icon && (
            <span className="mt-px flex h-5 w-5 shrink-0 items-center justify-center text-accent">
              <Icon size={15} />
            </span>
          )}
          <div>
            <h2 className="text-[13.5px] font-semibold tracking-tight text-ink">
              {title}
            </h2>
            {description && (
              <p className="text-xs text-muted mt-0.5">{description}</p>
            )}
          </div>
        </div>
      )}
      <div className="bg-surface rounded-2xl overflow-visible border border-hairline elev-card">
        <div className="divide-y divide-hairline">{children}</div>
      </div>
    </div>
  );
};
