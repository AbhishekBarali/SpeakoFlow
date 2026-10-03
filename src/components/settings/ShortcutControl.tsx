import React from "react";
import { useTranslation } from "react-i18next";
import { Pencil } from "lucide-react";
import { toast } from "sonner";
import { Keycaps } from "../ui/Keycaps";
import { formatKeyCombination } from "../../lib/utils/keyboard";
import { useOsType } from "../../hooks/useOsType";

/** `hero` is for feature banners; `default` for cards and tables. */
export type ShortcutFinish = "default" | "hero";
export type ShortcutSize = "sm" | "md" | "lg";

/**
 * Shortcuts that may be left without a key. Mirrors `OPTIONAL_BINDINGS` in
 * `settings.rs`, which is what actually accepts the empty binding.
 */
const OPTIONAL_SHORTCUTS = new Set(["cancel", "assistant_call"]);

export const isOptionalShortcut = (id: string): boolean =>
  OPTIONAL_SHORTCUTS.has(id);

/**
 * What the shortcut editor offers besides pressing new keys: going back to
 * the default, and turning an optional shortcut off.
 *
 * These only appear while a shortcut is being changed. A row at rest shows its
 * keys and nothing else, because a remove button and a reset button sitting
 * beside every set of keys were easy to hit by accident and made each row a
 * cluster of icons. Turning a shortcut off now takes two deliberate clicks
 * (change, then Turn off), and is announced with an Undo.
 *
 * Esc cannot be how you leave the editor: it is itself a key you may want to
 * record (it is Cancel's default). Clicking anywhere else leaves it, keeping
 * the shortcut as it was. That click handling is the editors' own, and it
 * treats these buttons as part of the editor, so render them inside the
 * element the editor watches.
 */
export const ShortcutEditActions: React.FC<{
  /** The binding as it was when editing began. */
  current: string;
  defaultBinding: string;
  optional: boolean;
  finish?: ShortcutFinish;
  disabled?: boolean;
  onUseDefault: () => void;
  onTurnOff: () => void;
}> = ({
  current,
  defaultBinding,
  optional,
  finish = "default",
  disabled = false,
  onUseDefault,
  onTurnOff,
}) => {
  const { t } = useTranslation();
  const osType = useOsType();
  const hero = finish === "hero";
  const showDefault =
    defaultBinding.trim() !== "" && current !== defaultBinding;
  const showOff = optional && current.trim() !== "";
  if (!showDefault && !showOff) return null;

  const action = `shrink-0 cursor-pointer whitespace-nowrap rounded px-1 text-xs font-medium leading-5 transition-colors focus-visible:outline-none focus-visible:ring-2 disabled:cursor-not-allowed disabled:opacity-50 ${
    hero
      ? "text-hero-muted hover:text-hero-ink focus-visible:ring-hero-ink"
      : "text-muted hover:text-ink focus-visible:ring-accent/40"
  }`;
  return (
    <span className="flex items-center justify-end">
      {showDefault && (
        <button
          type="button"
          className={action}
          disabled={disabled}
          title={formatKeyCombination(defaultBinding, osType)}
          onClick={onUseDefault}
        >
          {t("shortcutEditor.useDefault")}
        </button>
      )}
      {showDefault && showOff && (
        <span
          aria-hidden="true"
          className={hero ? "text-hero-muted" : "text-muted"}
        >
          ·
        </span>
      )}
      {showOff && (
        <button
          type="button"
          className={action}
          disabled={disabled}
          onClick={onTurnOff}
        >
          {t("shortcutEditor.turnOff")}
        </button>
      )}
    </span>
  );
};

/**
 * Say that a shortcut was turned off, with a way back. `restore` puts the keys
 * it had back.
 */
export const announceTurnedOff = (
  t: (key: string, options?: Record<string, unknown>) => string,
  name: string,
  restore: () => void,
) => {
  toast(t("shortcutEditor.turnedOff", { name }), {
    action: { label: t("shortcutEditor.undo"), onClick: restore },
  });
};

/**
 * A shortcut you can see *and* change: the keys themselves are the button.
 *
 * The keys used to be drawn in one place and edited in another (Settings →
 * Shortcuts), so the hero told you the shortcut and gave you no way to change
 * it. A small pencil sits beside the keys at all times, because a control that
 * only reveals itself on hover is one a new user never finds.
 */
export const ShortcutKeysButton: React.FC<{
  binding: string | null | undefined;
  /** What the shortcut does, for the accessible name ("Dictate"). */
  name: string;
  finish?: ShortcutFinish;
  size?: ShortcutSize;
  disabled?: boolean;
  /** Shown with no keys: "Off" for a shortcut turned off on purpose. */
  emptyLabel?: string;
  onClick: () => void;
}> = ({
  binding,
  name,
  finish = "default",
  size = "md",
  disabled = false,
  emptyLabel,
  onClick,
}) => {
  const { t } = useTranslation();
  const hero = finish === "hero";
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={t("shortcutEditor.change", { name })}
      title={t("settings.general.shortcut.clickToChange")}
      className={`group/keys -m-1 inline-flex cursor-pointer items-center gap-2 rounded-xl p-1 transition-colors focus-visible:outline-none focus-visible:ring-2 disabled:cursor-not-allowed disabled:opacity-50 ${
        hero
          ? "hover:bg-hero-hover focus-visible:ring-hero-ink"
          : "hover:bg-ink/[0.05] focus-visible:ring-accent/40"
      }`}
    >
      <Keycaps
        binding={binding}
        size={size}
        variant={hero ? "hero" : "default"}
        fallback={
          <span
            className={`px-1 text-sm ${hero ? "text-hero-muted" : "text-muted"}`}
          >
            {emptyLabel ?? t("settings.general.shortcut.notSet")}
          </span>
        }
      />
      <Pencil
        aria-hidden="true"
        className={`shrink-0 transition-opacity duration-150 group-hover/keys:opacity-100 group-focus-visible/keys:opacity-100 ${
          size === "lg" ? "h-4 w-4" : "h-3.5 w-3.5"
        } ${hero ? "text-hero-muted" : "text-muted opacity-50"}`}
      />
    </button>
  );
};

const RECORDING_SIZES: Record<ShortcutSize, string> = {
  sm: "min-h-7 px-2.5 py-1 text-xs",
  md: "min-h-8 px-3 py-1 text-sm",
  lg: "min-h-11 px-4 py-2 text-[0.9375rem]",
};

/** The keys being pressed while a new shortcut is recorded. */
export const RecordingKeys = React.forwardRef<
  HTMLDivElement,
  { text: string; finish?: ShortcutFinish; size?: ShortcutSize }
>(({ text, finish = "default", size = "md" }, ref) => {
  const hero = finish === "hero";
  return (
    <div
      ref={ref}
      data-shortcut-recording="true"
      role="status"
      aria-live="polite"
      className={`inline-flex shrink-0 items-center gap-2 whitespace-nowrap rounded-[0.625rem] border font-medium ${RECORDING_SIZES[size]} ${
        hero
          ? "border-hero-border bg-hero-control text-hero-ink"
          : "border-accent bg-accent/10 text-accent"
      }`}
    >
      <span aria-hidden="true" className="relative flex h-2 w-2 shrink-0">
        <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-current opacity-60 motion-reduce:hidden" />
        <span className="relative inline-flex h-2 w-2 rounded-full bg-current" />
      </span>
      <span>{text}</span>
    </div>
  );
});
RecordingKeys.displayName = "RecordingKeys";
