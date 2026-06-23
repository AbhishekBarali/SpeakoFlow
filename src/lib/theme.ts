/**
 * Appearance / theme handling shared by every window (main settings, assistant
 * panel, recording overlay).
 *
 * The actual colors live in CSS as `data-theme`-scoped custom properties
 * (see src/App.css and src/assistant/AssistantPanel.css). This module is only
 * responsible for resolving the user's preference to a concrete "light" |
 * "dark" value and writing it to `document.documentElement` so the CSS can
 * react.
 *
 * A copy of the preference is cached in localStorage so each window can apply
 * the correct theme synchronously on load — before React renders — which
 * avoids a flash of the wrong palette. The cache is shared across windows
 * (same origin); correctness still comes from re-applying once settings load.
 */

export type ThemePreference = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

const STORAGE_KEY = "speakoflow-theme";
const DARK_QUERY = "(prefers-color-scheme: dark)";

const isValidPreference = (value: unknown): value is ThemePreference =>
  value === "light" || value === "dark" || value === "system";

const prefersDark = (): boolean => {
  if (typeof window === "undefined" || !window.matchMedia) return false;
  return window.matchMedia(DARK_QUERY).matches;
};

/** Resolve a preference to the concrete theme that should be applied now. */
export const resolveTheme = (preference: ThemePreference): ResolvedTheme =>
  preference === "system" ? (prefersDark() ? "dark" : "light") : preference;

/** Read the cached preference (defaults to "system"). */
export const getCachedPreference = (): ThemePreference => {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (isValidPreference(stored)) return stored;
  } catch {
    // localStorage unavailable — fall back to system.
  }
  return "system";
};

const setResolvedAttribute = (resolved: ResolvedTheme): void => {
  if (typeof document !== "undefined") {
    document.documentElement.dataset.theme = resolved;
  }
};

/**
 * Apply a preference: write the resolved theme to <html> and cache the choice.
 * Call this whenever the user's setting is known or changes.
 */
export const applyThemePreference = (preference: ThemePreference): void => {
  setResolvedAttribute(resolveTheme(preference));
  try {
    localStorage.setItem(STORAGE_KEY, preference);
  } catch {
    // Best-effort cache only.
  }
};

/**
 * Apply the cached preference synchronously. Call this from a window entry
 * point before React renders to avoid a flash of the wrong theme.
 */
export const applyCachedTheme = (): void => {
  setResolvedAttribute(resolveTheme(getCachedPreference()));
};

/**
 * Keep the resolved theme in sync with the OS while the preference is
 * "system". Returns an unsubscribe function. `getPreference` is read lazily so
 * the latest setting is always used.
 */
export const watchSystemTheme = (
  getPreference: () => ThemePreference,
): (() => void) => {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const media = window.matchMedia(DARK_QUERY);
  const handler = () => {
    if (getPreference() === "system") applyThemePreference("system");
  };
  media.addEventListener("change", handler);
  return () => media.removeEventListener("change", handler);
};

/**
 * Assistant-panel appearance.
 *
 * The floating panel follows the app-wide theme by default ("auto"), but the
 * user can override it to force light or dark for the panel only. The resolved
 * concrete theme is cached under a dedicated key so the panel window can paint
 * the right palette before React mounts (no flash) without clobbering the
 * app-wide preference (both windows share localStorage on the same origin).
 */
export type AssistantThemePref = "auto" | "light" | "dark";

const ASSISTANT_STORAGE_KEY = "speakoflow-assistant-theme";

const isResolvedTheme = (value: unknown): value is ResolvedTheme =>
  value === "light" || value === "dark";

/**
 * Resolve the panel's effective concrete theme. "auto" follows the app-wide
 * preference (which itself may be "system"); an explicit light/dark wins.
 */
export const resolveAssistantTheme = (
  pref: AssistantThemePref,
  appPref: ThemePreference,
): ResolvedTheme => (pref === "auto" ? resolveTheme(appPref) : pref);

/**
 * Apply the panel's effective theme and cache the resolved value so the next
 * panel load can paint it before React mounts.
 */
export const applyAssistantTheme = (
  pref: AssistantThemePref,
  appPref: ThemePreference,
): void => {
  const resolved = resolveAssistantTheme(pref, appPref);
  setResolvedAttribute(resolved);
  try {
    localStorage.setItem(ASSISTANT_STORAGE_KEY, resolved);
  } catch {
    // Best-effort cache only.
  }
};

/**
 * Apply the cached panel theme synchronously, before React renders, to avoid a
 * flash. Falls back to the app-wide cached preference when the panel has no
 * cached value yet (i.e. it has never been overridden).
 */
export const applyCachedAssistantTheme = (): void => {
  try {
    const stored = localStorage.getItem(ASSISTANT_STORAGE_KEY);
    if (isResolvedTheme(stored)) {
      setResolvedAttribute(stored);
      return;
    }
  } catch {
    // localStorage unavailable — fall through to the app preference.
  }
  setResolvedAttribute(resolveTheme(getCachedPreference()));
};
