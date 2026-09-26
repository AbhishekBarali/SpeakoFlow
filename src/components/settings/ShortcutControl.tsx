import React from "react";
import { useTranslation } from "react-i18next";
import { Pencil } from "lucide-react";
import { Keycaps } from "../ui/Keycaps";

/** `glass` is for the gradient heroes; `default` for cards and tables. */
export type ShortcutFinish = "default" | "glass";
export type ShortcutSize = "sm" | "md" | "lg";

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
  onClick: () => void;
}> = ({
  binding,
  name,
  finish = "default",
  size = "md",
  disabled = false,
  onClick,
}) => {
  const { t } = useTranslation();
  const glass = finish === "glass";
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={t("shortcutEditor.change", { name })}
      title={t("settings.general.shortcut.clickToChange")}
      className={`group/keys -m-1 inline-flex cursor-pointer items-center gap-2 rounded-xl p-1 transition-colors focus-visible:outline-none focus-visible:ring-2 disabled:cursor-not-allowed disabled:opacity-50 ${
        glass
          ? "hover:bg-white/10 focus-visible:ring-white/60"
          : "hover:bg-ink/[0.05] focus-visible:ring-accent/40"
      }`}
    >
      <Keycaps
        binding={binding}
        size={size}
        variant={glass ? "glass" : "default"}
        fallback={
          <span
            className={`px-1 text-sm ${glass ? "text-white/80" : "text-muted"}`}
          >
            {t("settings.general.shortcut.notSet")}
          </span>
        }
      />
      <Pencil
        aria-hidden="true"
        className={`shrink-0 transition-opacity duration-150 group-hover/keys:opacity-100 group-focus-visible/keys:opacity-100 ${
          size === "lg" ? "h-4 w-4" : "h-3.5 w-3.5"
        } ${glass ? "text-white opacity-55" : "text-muted opacity-50"}`}
      />
    </button>
  );
};

const RECORDING_SIZES: Record<ShortcutSize, string> = {
  sm: "h-7 px-2.5 text-xs",
  md: "h-8 px-3 text-sm",
  lg: "h-11 px-4 text-[0.9375rem]",
};

/** The keys being pressed while a new shortcut is recorded. */
export const RecordingKeys = React.forwardRef<
  HTMLDivElement,
  { text: string; finish?: ShortcutFinish; size?: ShortcutSize }
>(({ text, finish = "default", size = "md" }, ref) => {
  const glass = finish === "glass";
  return (
    <div
      ref={ref}
      data-shortcut-recording="true"
      role="status"
      aria-live="polite"
      className={`inline-flex items-center gap-2 rounded-[0.625rem] border font-medium ${RECORDING_SIZES[size]} ${
        glass
          ? "border-white/55 bg-white/15 text-white backdrop-blur-sm"
          : "border-accent bg-accent/10 text-accent"
      }`}
    >
      <span aria-hidden="true" className="relative flex h-2 w-2">
        <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-current opacity-60 motion-reduce:hidden" />
        <span className="relative inline-flex h-2 w-2 rounded-full bg-current" />
      </span>
      {text}
    </div>
  );
});
RecordingKeys.displayName = "RecordingKeys";
