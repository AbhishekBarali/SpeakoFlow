import type { ThinkingLevel } from "@/bindings";

/** The levels, in the order the dropdown lists them. */
export const THINKING_LEVELS: readonly ThinkingLevel[] = [
  "off",
  "low",
  "medium",
  "high",
] as const;

/**
 * Why a provider cannot take a thinking level, or `null` when it can.
 * Mirrors `provider_supports_thinking` in `src-tauri/src/settings.rs`.
 */
export type ThinkingUnsupportedReason = "builtin" | "anthropic" | "apple";

export const thinkingUnsupportedReason = (
  providerId: string | null | undefined,
): ThinkingUnsupportedReason | null => {
  switch (providerId) {
    case "builtin":
      return "builtin";
    case "anthropic":
      return "anthropic";
    case "apple_intelligence":
      return "apple";
    default:
      return null;
  }
};

export const providerSupportsThinking = (
  providerId: string | null | undefined,
): boolean => thinkingUnsupportedReason(providerId) === null;
