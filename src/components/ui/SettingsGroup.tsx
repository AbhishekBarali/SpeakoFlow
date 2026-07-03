import React from "react";
import type { SettingIcon } from "./tones";

interface SettingsGroupProps {
  title?: string;
  description?: string;
  /** Optional accent icon shown before the title. When provided, the header
   * switches to the brand-accent style (icon + colored title); without it the
   * quiet uppercase label is kept so other pages stay unchanged. */
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
      {title &&
        (Icon ? (
          <div className="px-1">
            <div className="flex items-center gap-2">
              <Icon className="w-[17px] h-[17px] text-accent" strokeWidth={2} />
              <h2 className="text-[15px] font-semibold tracking-tight text-accent">
                {title}
              </h2>
            </div>
            {description && (
              <p className="text-xs text-muted mt-1 ms-[25px]">{description}</p>
            )}
          </div>
        ) : (
          <div className="px-1">
            <h2 className="text-xs font-semibold uppercase tracking-[0.08em] text-muted">
              {title}
            </h2>
            {description && (
              <p className="text-xs text-muted mt-0.5">{description}</p>
            )}
          </div>
        ))}
      <div className="bg-surface rounded-2xl overflow-visible border border-hairline elev-card">
        <div className="divide-y divide-hairline">{children}</div>
      </div>
    </div>
  );
};
