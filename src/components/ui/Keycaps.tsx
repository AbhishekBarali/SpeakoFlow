import React from "react";
import { formatKeyCombination } from "@/lib/utils/keyboard";
import { useOsType } from "@/hooks/useOsType";

/**
 * A shortcut drawn as physical keys instead of the sentence "Left Ctrl + Left
 * Windows". People recognise keys faster than they read a string. Editing
 * still happens in Settings → Shortcuts.
 *
 * Two finishes: `default` is a light keycap with a pressed-in bottom edge for
 * cards, `glass` is a frosted key for the gradient heroes.
 */

type KeycapSize = "sm" | "md" | "lg";

interface KeycapsProps {
  /** Raw binding string as stored, e.g. `ctrl_left+super_left`. */
  binding: string | null | undefined;
  size?: KeycapSize;
  variant?: "default" | "glass";
  className?: string;
  /** Rendered when there is no binding. */
  fallback?: React.ReactNode;
}

/** Split a stored binding into display labels, one per key. */
export const bindingKeyLabels = (
  binding: string,
  osType: ReturnType<typeof useOsType>,
): string[] =>
  formatKeyCombination(binding, osType)
    .split(" + ")
    .map((label) => label.trim())
    // "Left Ctrl" reads as two things; the key says "Ctrl".
    .map((label) => label.replace(/^(Left|Right)\s+/i, ""))
    // The ⊞ key is printed "Win" on the keyboard itself.
    .map((label) => (label === "Windows" ? "Win" : label))
    .filter(Boolean);

const SIZES: Record<KeycapSize, string> = {
  sm: "h-[1.375rem] min-w-[1.375rem] px-1.5 text-[0.6875rem] rounded-[0.3125rem]",
  md: "h-7 min-w-7 px-2 text-[0.8125rem] rounded-md",
  lg: "h-11 min-w-11 px-3.5 text-[0.9375rem] rounded-[0.625rem]",
};

export const Keycaps: React.FC<KeycapsProps> = ({
  binding,
  size = "md",
  variant = "default",
  className = "",
  fallback = null,
}) => {
  const osType = useOsType();
  if (!binding) return <>{fallback}</>;
  const keys = bindingKeyLabels(binding, osType);
  if (keys.length === 0) return <>{fallback}</>;

  const finish =
    variant === "glass" ? "keycap-glass" : "keycap bg-surface text-ink";

  return (
    <span
      className={`inline-flex shrink-0 items-center ${size === "lg" ? "gap-1.5" : "gap-1"} ${className}`}
      aria-label={keys.join(" + ")}
    >
      {keys.map((key, index) => (
        <kbd
          key={`${key}-${index}`}
          aria-hidden="true"
          className={`inline-flex items-center justify-center font-sans font-medium whitespace-nowrap ${finish} ${SIZES[size]}`}
        >
          {key}
        </kbd>
      ))}
    </span>
  );
};
