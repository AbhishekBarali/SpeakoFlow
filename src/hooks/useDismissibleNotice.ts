import { useCallback, useState } from "react";

/**
 * Remembers that the user closed an advisory notice.
 *
 * Kept in `localStorage` rather than app settings on purpose: which hints a user
 * has already read is window chrome, not configuration. It should not travel in
 * a settings export, and it should not add a field that every settings
 * migration then has to carry.
 *
 * Only use this for notices whose content is reachable another way. A notice the
 * user has to act on should not be closable at all, and the caller is
 * responsible for that distinction.
 */
const storageKey = (id: string) => `speakoflow.notice.dismissed.${id}`;

const readDismissed = (id: string): boolean => {
  try {
    return window.localStorage.getItem(storageKey(id)) === "1";
  } catch {
    // Private modes and hardened configurations can throw on access. A notice
    // that shows again is a far better failure than a settings page that throws.
    return false;
  }
};

export interface DismissibleNotice {
  /** False once the user has closed it. */
  visible: boolean;
  dismiss: () => void;
  /** Brings it back, for a "show hints again" affordance. */
  restore: () => void;
}

export const useDismissibleNotice = (id: string): DismissibleNotice => {
  const [dismissed, setDismissed] = useState(() => readDismissed(id));

  const dismiss = useCallback(() => {
    setDismissed(true);
    try {
      window.localStorage.setItem(storageKey(id), "1");
    } catch {
      // Dismissal still applies for this session even if it cannot persist.
    }
  }, [id]);

  const restore = useCallback(() => {
    setDismissed(false);
    try {
      window.localStorage.removeItem(storageKey(id));
    } catch {
      // Nothing to undo if it was never written.
    }
  }, [id]);

  return { visible: !dismissed, dismiss, restore };
};
