/**
 * The decisions in the update flow that do not need a running app, kept apart
 * from the store so they can be tested on their own.
 */

/**
 * What a failed `check()` actually means.
 *
 * The updater plugin reports three quite different situations as errors:
 *  - `ReleaseNotFound` ("Could not fetch a valid release JSON…"): the endpoint
 *    answered but had no manifest — true of every release published before
 *    1.5.0, whose `releases/latest` carries no latest.json. There is nothing
 *    newer to offer, so that is "up to date", not a failure.
 *  - `TargetsNotFound` ("None of the fallback platforms…"): the release exists
 *    but has no build for this OS/architecture (a platform that failed in CI,
 *    or one that is not built at all). That is worth saying plainly.
 *  - anything else: network, TLS, a malformed manifest.
 */
export type CheckFailure = "noUpdate" | "noBuild" | "failed";

export function classifyCheckError(error: unknown): CheckFailure {
  const text = String(
    error instanceof Error ? error.message : (error ?? ""),
  ).toLowerCase();
  if (text.includes("valid release json") || text.includes("releasenotfound")) {
    return "noUpdate";
  }
  if (text.includes("fallback platforms") || text.includes("targetsnotfound")) {
    return "noBuild";
  }
  return "failed";
}

/** Whole percent, or null while the size is unknown. Never above 100. */
export function percentOf(done: number, total: number | null | undefined) {
  if (!total || total <= 0) return null;
  return Math.max(0, Math.min(100, Math.floor((done / total) * 100)));
}

/** How often an open app looks again. It can sit in the tray for weeks. */
export const RECHECK_INTERVAL_MS = 12 * 60 * 60 * 1000;

/** First check after launch waits for startup work (models, engines) to settle. */
export const FIRST_CHECK_DELAY_MS = 8_000;

/** Only https links from release notes are opened, and never in the app window. */
export function safeExternalHref(href: string | undefined): string | null {
  if (!href) return null;
  try {
    const url = new URL(href);
    return url.protocol === "https:" ? url.toString() : null;
  } catch {
    return null;
  }
}
