import React, { useEffect, useRef } from "react";
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
const OPTIONAL_SHORTCUTS = new Set(["cancel", "assistant_call", "hands_free"]);

export const isOptionalShortcut = (id: string): boolean =>
  OPTIONAL_SHORTCUTS.has(id);

/**
 * What the shortcut editor offers besides pressing new keys: going back to
 * the default, turning an optional shortcut off, and giving it other keys
 * that do the same thing (or removing them).
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
  onUseDefault?: () => void;
  onTurnOff?: () => void;
  /** Start recording other keys for the shortcut. */
  onAddAnother?: () => void;
  /**
   * Mouse buttons can be the other keys here (they are on, on the engine
   * that hears them), so "Add another" says so.
   */
  mouseButtons?: boolean;
  /** Remove the other keys being changed. */
  onRemove?: () => void;
}> = ({
  current,
  defaultBinding,
  optional,
  finish = "default",
  disabled = false,
  onUseDefault,
  onTurnOff,
  onAddAnother,
  mouseButtons = false,
  onRemove,
}) => {
  const { t } = useTranslation();
  const osType = useOsType();
  const hero = finish === "hero";
  const actions: {
    id: string;
    label: string;
    title?: string;
    onClick: () => void;
  }[] = [];
  if (
    onUseDefault &&
    defaultBinding.trim() !== "" &&
    current !== defaultBinding
  ) {
    actions.push({
      id: "default",
      label: t("shortcutEditor.useDefault"),
      title: formatKeyCombination(defaultBinding, osType),
      onClick: onUseDefault,
    });
  }
  if (onTurnOff && optional && current.trim() !== "") {
    actions.push({
      id: "off",
      label: t("shortcutEditor.turnOff"),
      onClick: onTurnOff,
    });
  }
  if (onAddAnother) {
    actions.push({
      id: "add",
      label: mouseButtons
        ? t("shortcutEditor.addButtonOrKeys")
        : t("shortcutEditor.addAnother"),
      title: mouseButtons
        ? t("shortcutEditor.addButtonOrKeysHint")
        : t("shortcutEditor.addAnotherHint"),
      onClick: onAddAnother,
    });
  }
  if (onRemove) {
    actions.push({
      id: "remove",
      label: t("shortcutEditor.remove"),
      onClick: onRemove,
    });
  }
  if (actions.length === 0) return null;

  const action = `shrink-0 cursor-pointer whitespace-nowrap rounded px-1 text-xs font-medium leading-5 transition-colors focus-visible:outline-none focus-visible:ring-2 disabled:cursor-not-allowed disabled:opacity-50 ${
    hero
      ? "text-hero-muted hover:text-hero-ink focus-visible:ring-hero-ink"
      : "text-muted hover:text-ink focus-visible:ring-accent/40"
  }`;
  return (
    <span className="flex flex-wrap items-center justify-end">
      {actions.map((item, index) => (
        <React.Fragment key={item.id}>
          {index > 0 && (
            <span
              aria-hidden="true"
              className={hero ? "text-hero-muted" : "text-muted"}
            >
              ·
            </span>
          )}
          <button
            type="button"
            className={action}
            disabled={disabled}
            title={item.title}
            onClick={item.onClick}
          >
            {item.label}
          </button>
        </React.Fragment>
      ))}
    </span>
  );
};

/** `MouseEvent.button` for the middle, Back and Forward buttons. */
const SHORTCUT_BUTTONS = new Set([1, 3, 4]);

/**
 * While keys are being recorded, the middle, Back and Forward buttons belong
 * to the recorder: left to the window, Back would navigate the settings page
 * and the middle button would start autoscroll, mid-recording. `onPress` hears
 * each press, for an editor that has to say it cannot record one.
 */
export const useMouseButtonsWhileRecording = (
  active: boolean,
  onPress?: () => void,
) => {
  const onPressRef = useRef(onPress);
  onPressRef.current = onPress;
  useEffect(() => {
    if (!active) return;
    const hold = (e: MouseEvent) => {
      if (!SHORTCUT_BUTTONS.has(e.button)) return;
      e.preventDefault();
      if (e.type === "mousedown") onPressRef.current?.();
    };
    const types = ["mousedown", "mouseup", "auxclick"] as const;
    for (const type of types) window.addEventListener(type, hold, true);
    return () => {
      for (const type of types) window.removeEventListener(type, hold, true);
    };
  }, [active]);
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
  /** The accessible name, when the keys are not the shortcut's own. */
  label?: string;
  /** The pencil goes after the last set of keys on a row, not after each. */
  pencil?: boolean;
  onClick: () => void;
}> = ({
  binding,
  name,
  finish = "default",
  size = "md",
  disabled = false,
  emptyLabel,
  label,
  pencil = true,
  onClick,
}) => {
  const { t } = useTranslation();
  const hero = finish === "hero";
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={label ?? t("shortcutEditor.change", { name })}
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
      {pencil && (
        <Pencil
          aria-hidden="true"
          className={`shrink-0 transition-opacity duration-150 group-hover/keys:opacity-100 group-focus-visible/keys:opacity-100 ${
            size === "lg" ? "h-4 w-4" : "h-3.5 w-3.5"
          } ${hero ? "text-hero-muted" : "text-muted opacity-50"}`}
        />
      )}
    </button>
  );
};

/**
 * A shortcut's keys at rest: its own keys and, when it has them, its other
 * keys after a quiet "or". Each set is its own button, so the one you click is
 * the one you change.
 */
export const ShortcutKeysAtRest: React.FC<{
  binding: string | null | undefined;
  /** The shortcut's other keys; nothing is shown for none. */
  alternate?: string | null;
  name: string;
  finish?: ShortcutFinish;
  size?: ShortcutSize;
  disabled?: boolean;
  emptyLabel?: string;
  onClick: () => void;
  onClickAlternate: () => void;
}> = ({ alternate, onClickAlternate, ...own }) => {
  const { t } = useTranslation();
  // A shortcut that is turned off is off by both sets of keys.
  const other = own.binding?.trim() ? (alternate?.trim() ?? "") : "";
  if (!other) return <ShortcutKeysButton {...own} />;
  const hero = own.finish === "hero";
  return (
    <span className="inline-flex flex-wrap items-center justify-end gap-x-1.5 gap-y-1">
      <ShortcutKeysButton {...own} pencil={false} />
      <span className={`text-xs ${hero ? "text-hero-muted" : "text-muted"}`}>
        {t("shortcutEditor.or")}
      </span>
      <ShortcutKeysButton
        binding={other}
        name={own.name}
        finish={own.finish}
        size={own.size}
        disabled={own.disabled}
        label={t("shortcutEditor.changeOther", { name: own.name })}
        onClick={onClickAlternate}
      />
    </span>
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
