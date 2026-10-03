/**
 * Whether this install has been through first-run setup.
 *
 * This flag is the only thing that decides whether onboarding shows: it is not
 * skipped for someone who already has a model or dictates in the cloud. Every
 * install sees it once.
 *
 * `localStorage`, like the app's other "already seen" flags
 * (`useDismissibleNotice`): it describes this window, not configuration, and it
 * should not travel in a settings export. Losing it costs one more pass through
 * onboarding.
 */
const KEY = "speakoflow.onboarding.completed";
/**
 * Bumped when onboarding changes enough to be worth showing everyone again.
 * "3" is the redesigned setup and tour, so existing installs see it once.
 */
const VERSION = "3";

export function hasCompletedOnboarding(): boolean {
  try {
    return window.localStorage.getItem(KEY) === VERSION;
  } catch {
    return false;
  }
}

export function markOnboardingComplete(): void {
  try {
    window.localStorage.setItem(KEY, VERSION);
  } catch {
    // Private or locked-down storage: setup simply shows again next time.
  }
}
