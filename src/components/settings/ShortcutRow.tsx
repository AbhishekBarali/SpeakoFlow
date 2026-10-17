import React from "react";
import { SettingContainer } from "../ui/SettingContainer";
import { ShortcutInput } from "./ShortcutInput";

/**
 * A quiet text button the height of a keycap, for what a shortcut row offers
 * in place of keys: "Turn on" for a feature that is off. A full secondary
 * button there was the tallest thing on the row, and drew the eye away from
 * the keys.
 */
export const RowAction: React.FC<{
  onClick: () => void;
  children: React.ReactNode;
  disabled?: boolean;
}> = ({ onClick, children, disabled = false }) => (
  <button
    type="button"
    onClick={onClick}
    disabled={disabled}
    className="inline-flex h-7 shrink-0 cursor-pointer items-center whitespace-nowrap rounded-md px-2 text-[0.8125rem] font-medium text-accent transition-colors hover:bg-accent/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-50"
  >
    {children}
  </button>
);

/** The muted words in front of the hands-free key: "While dictating, press". */
export const KeysPrefix: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => (
  <span className="shrink-0 whitespace-nowrap text-[0.8125rem] text-muted">
    {children}
  </span>
);

interface ShortcutRowProps {
  shortcutId: string;
  title: string;
  /** Behind the (i). */
  info?: string;
  /** Under the title, for a line the user has to see to use the row (why the
   *  keys do nothing). */
  caption?: string;
  /** Before the keys, only where the keys alone would mislead (the
   *  hands-free key is pressed while dictating, not on its own). */
  prefix?: React.ReactNode;
  disabled?: boolean;
  /** Shown instead of the keys. */
  children?: React.ReactNode;
}

/**
 * One shortcut in Settings → Shortcuts: its name and (i) on the left, its keys
 * on the right, and nothing else, so the keys are the one thing to look at.
 */
export const ShortcutRow: React.FC<ShortcutRowProps> = ({
  shortcutId,
  title,
  info,
  caption,
  prefix,
  disabled = false,
  children,
}) => (
  <SettingContainer
    title={title}
    description={caption ?? info}
    info={caption ? info : undefined}
    descriptionMode={caption ? "caption" : "tooltip"}
    grouped={true}
    disabled={disabled}
  >
    {children ?? (
      <span className="flex items-center gap-2">
        {prefix}
        <ShortcutInput shortcutId={shortcutId} bare disabled={disabled} />
      </span>
    )}
  </SettingContainer>
);
