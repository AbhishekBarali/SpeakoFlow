import { useSyncExternalStore } from "react";
import type { ResolvedTheme } from "@/lib/theme";

/**
 * The theme actually applied to this window right now, read from the
 * `data-theme` attribute `lib/theme.ts` writes on <html>. For the rare
 * component that needs a different asset per theme (a banner's artwork);
 * colours belong in CSS, which follows the attribute on its own.
 */
const read = (): ResolvedTheme =>
  document.documentElement.dataset.theme === "dark" ? "dark" : "light";

const subscribe = (onChange: () => void): (() => void) => {
  const observer = new MutationObserver(onChange);
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme"],
  });
  return () => observer.disconnect();
};

export const useResolvedTheme = (): ResolvedTheme =>
  useSyncExternalStore(subscribe, read, () => "light");
