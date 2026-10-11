/**
 * Resolve once `isSaving()` turns false, or after `timeoutMs`, whichever comes
 * first. `subscribe` is called with a listener that runs on every store change
 * and must return the function that removes it (a zustand store's `subscribe`
 * has exactly this shape).
 *
 * Used before a connection test: provider fields save on blur, and clicking
 * the test button is what blurs them, so the save of a just-pasted key is still
 * in flight when the click lands. Testing then would check the previous key.
 * The timeout keeps a save that never settles from leaving the button spinning.
 */
export const waitForSaves = (
  isSaving: () => boolean,
  subscribe: (listener: () => void) => () => void,
  timeoutMs = 5000,
): Promise<void> =>
  new Promise((resolve) => {
    if (!isSaving()) {
      resolve();
      return;
    }
    let settled = false;
    let unsubscribe: () => void = () => {};
    const done = () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      unsubscribe();
      resolve();
    };
    const timer = setTimeout(done, timeoutMs);
    unsubscribe = subscribe(() => {
      if (!isSaving()) done();
    });
    // The save may have finished between the first check and subscribing.
    if (!isSaving()) done();
  });
