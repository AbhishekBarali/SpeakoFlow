import React from "react";

interface SettingsGroupProps {
  title?: string;
  description?: string;
  children: React.ReactNode;
}

export const SettingsGroup: React.FC<SettingsGroupProps> = ({
  title,
  description,
  children,
}) => {
  return (
    <div className="space-y-2.5">
      {title && (
        <div className="px-1">
          <h2 className="text-[11px] font-semibold text-muted uppercase tracking-[0.1em]">
            {title}
          </h2>
          {description && (
            <p className="text-xs text-muted mt-1">{description}</p>
          )}
        </div>
      )}
      <div className="bg-surface border border-hairline rounded-2xl overflow-visible shadow-[0_1px_2px_rgba(12,10,9,0.04)]">
        <div className="divide-y divide-hairline">{children}</div>
      </div>
    </div>
  );
};
