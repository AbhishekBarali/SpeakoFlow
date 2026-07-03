import React, { useRef, useState } from "react";
import { Tooltip } from "./Tooltip";
import { TONE_TILE, type SettingIcon, type SettingTone } from "./tones";

interface SettingContainerProps {
  title: string;
  /** Optional one-line caption rendered under the title. Only for settings
   * whose name alone doesn't explain the behavior — most rows need none. */
  description?: string;
  /** Optional deep-dive help shown behind a small (i) icon. Use for
   * "extra-extra" detail (formats, examples, tradeoffs) that would clutter
   * the row as a caption. */
  info?: string;
  /** Optional leading icon rendered in a soft-tinted rounded tile (iOS-style).
   * Provide `tone` to color it; defaults to the brand teal. */
  icon?: SettingIcon;
  tone?: SettingTone;
  children: React.ReactNode;
  /** @deprecated Descriptions always render inline now; kept so existing
   * call sites keep compiling. */
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  layout?: "horizontal" | "stacked";
  disabled?: boolean;
  /** @deprecated Kept for call-site compatibility. */
  tooltipPosition?: "top" | "bottom";
}

/** Soft-tinted rounded tile holding a row's leading icon. */
const IconTile: React.FC<{
  icon: SettingIcon;
  tone: SettingTone;
  disabled?: boolean;
}> = ({ icon: Icon, tone, disabled }) => (
  <span
    aria-hidden
    className={`grid place-items-center h-9 w-9 rounded-[11px] shrink-0 elev-chip ${TONE_TILE[tone]} ${
      disabled ? "opacity-50" : ""
    }`}
  >
    <Icon className="w-[18px] h-[18px]" strokeWidth={2} />
  </span>
);

/** Small circled-i affordance revealing a tooltip with deep-dive help. */
const InfoHint: React.FC<{ text: string }> = ({ text }) => {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);
  return (
    <span
      ref={ref}
      className="relative inline-flex items-center text-muted-soft hover:text-muted transition-colors"
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
    >
      <svg
        className="w-[13px] h-[13px]"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.8}
        viewBox="0 0 24 24"
        aria-label="More information"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
        />
      </svg>
      {open && (
        <Tooltip targetRef={ref} position="top">
          <p className="text-xs leading-relaxed text-start">{text}</p>
        </Tooltip>
      )}
    </span>
  );
};

/**
 * A single settings row: title (plus optional muted caption) on the left,
 * control on the right. Stacked layout puts the control full-width below.
 * Info policy: self-evident rows get nothing; behavior notes are quiet
 * captions; deep detail hides behind the (i) hint.
 */
export const SettingContainer: React.FC<SettingContainerProps> = ({
  title,
  description,
  info,
  icon,
  tone = "teal",
  children,
  grouped = false,
  layout = "horizontal",
  disabled = false,
}) => {
  const titleClasses = `text-[13px] font-normal leading-snug ${disabled ? "text-muted-soft" : "text-ink"}`;
  const descriptionClasses = `mt-0.5 text-xs leading-snug max-w-md ${disabled ? "text-muted-soft" : "text-muted"}`;

  const titleRow = (
    <div className="flex items-center gap-1.5 min-w-0">
      <h3 className={`${titleClasses} truncate`}>{title}</h3>
      {info && <InfoHint text={info} />}
    </div>
  );

  if (layout === "stacked") {
    return (
      <div
        className={
          grouped
            ? "px-4 py-3"
            : "px-4 py-3 rounded-xl border border-hairline bg-surface"
        }
      >
        <div className="mb-2.5 flex items-start gap-3">
          {icon && <IconTile icon={icon} tone={tone} disabled={disabled} />}
          <div className="min-w-0 flex-1">
            {titleRow}
            {description && <p className={descriptionClasses}>{description}</p>}
          </div>
        </div>
        <div className="w-full">{children}</div>
      </div>
    );
  }

  return (
    <div
      className={
        grouped
          ? "flex items-center gap-3 px-4 py-3"
          : "flex items-center gap-3 px-4 py-3 rounded-xl border border-hairline bg-surface"
      }
    >
      {icon && <IconTile icon={icon} tone={tone} disabled={disabled} />}
      <div className="min-w-0 flex-1">
        {titleRow}
        {description && <p className={descriptionClasses}>{description}</p>}
      </div>
      <div className="relative shrink-0">{children}</div>
    </div>
  );
};
