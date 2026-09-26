import React from "react";
import { InfoTip } from "./InfoTip";
import { type SettingIcon, type SettingTone } from "./tones";

interface SettingContainerProps {
  title: string;
  /** What the setting does. Shown behind the (i) next to the title — see
   *  `descriptionMode` for the one exception. */
  description?: string;
  /** Extra detail appended to the same (i). */
  info?: string;
  /** @deprecated Rows no longer draw icon tiles; kept for call sites. */
  icon?: SettingIcon;
  /** @deprecated See `icon`. */
  tone?: SettingTone;
  children: React.ReactNode;
  /**
   * Where the description goes. Every mode except `caption` puts it behind the
   * (i): a caption under every row is what made the old pages walls of text.
   * Use `caption` only for a line the user must see to operate the control
   * (a status such as "Key saved"), never for an explanation.
   *
   * `inline` is accepted for old call sites and now means the same as
   * `tooltip`.
   */
  descriptionMode?: "inline" | "tooltip" | "caption";
  grouped?: boolean;
  layout?: "horizontal" | "stacked";
  disabled?: boolean;
  /** @deprecated Kept for call-site compatibility. */
  tooltipPosition?: "top" | "bottom";
  /**
   * Content under the label-and-control line, in the same row (a list the
   * setting manages, a picker that only matters once it is on). Omit or pass a
   * falsy value and the row is a single line.
   */
  details?: React.ReactNode;
}

/**
 * A single settings row: label (with its (i)) on the left, control on the
 * right. Stacked layout puts the control full-width below the label.
 */
export const SettingContainer: React.FC<SettingContainerProps> = ({
  title,
  description,
  info,
  children,
  descriptionMode = "tooltip",
  grouped = false,
  layout = "horizontal",
  disabled = false,
  details,
}) => {
  const caption = descriptionMode === "caption" ? description : undefined;
  // Both parts go behind the (i). The tip shows the first sentence and keeps
  // the rest behind "More" (see InfoTip), so a detail such as a privacy note
  // is one click away instead of either lost or a wall of text.
  const tipParts = [caption ? undefined : description, info].filter(Boolean);
  const tip = tipParts.length > 0 ? tipParts.join(" ") : undefined;

  const titleClasses = `text-sm font-medium leading-snug ${disabled ? "text-muted-soft" : "text-ink"}`;

  const titleRow = (
    <div className="flex min-w-0 items-center gap-1">
      <h3 className={`${titleClasses} min-w-0 text-pretty`}>{title}</h3>
      {tip && <InfoTip text={tip} />}
    </div>
  );
  const captionLine = caption && (
    <p
      className={`mt-0.5 max-w-lg text-[0.8125rem] leading-snug ${disabled ? "text-muted-soft" : "text-muted"}`}
    >
      {caption}
    </p>
  );

  const frame = grouped
    ? ""
    : "rounded-2xl border border-hairline bg-surface elev-card";

  if (layout === "stacked") {
    return (
      <div className={`px-5 py-4 ${frame}`}>
        <div className="mb-3 min-w-0">
          {titleRow}
          {captionLine}
        </div>
        <div className="w-full">{children}</div>
      </div>
    );
  }

  // Wraps rather than squeezing: when a wide control and the label cannot
  // share a line, the control moves under the label instead of crushing it.
  const line = (
    <div
      className={`flex flex-wrap items-center gap-x-4 gap-y-2.5 ${details ? "min-h-8" : "min-h-[3.5rem] px-5 py-3"} ${details ? "" : frame}`}
    >
      <div className="min-w-[9rem] flex-1">
        {titleRow}
        {captionLine}
      </div>
      <div className="relative ms-auto max-w-full shrink-0">{children}</div>
    </div>
  );

  if (!details) return line;

  return (
    <div className={`px-5 py-3 ${frame}`}>
      {line}
      <div className="mt-3">{details}</div>
    </div>
  );
};
