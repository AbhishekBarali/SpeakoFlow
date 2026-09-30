/**
 * Whether this install has been through first-run setup.
 *
 * `localStorage`, like the app's other "already seen" flags
 * (`useDismissibleNotice`): it describes this window, not configuration, and it
 * should not travel in a settings export. Losing it costs one extra look at
 * setup, and only on a machine with no speech model on it.
 */
const KEY = "speakoflow.onboarding.completed";
/** Bumped if a future setup is different enough to be worth showing again. */
const VERSION = "2";

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
