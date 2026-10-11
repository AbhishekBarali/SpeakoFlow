import { describe, expect, test } from "bun:test";
import { waitForSaves } from "./waitForSaves";

/** A tiny store: a saving flag plus listeners, like a zustand store. */
const fakeStore = (saving: boolean) => {
  const listeners = new Set<() => void>();
  return {
    isSaving: () => saving,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    finish: () => {
      saving = false;
      listeners.forEach((listener) => listener());
    },
    listenerCount: () => listeners.size,
  };
};

describe("waitForSaves", () => {
  test("resolves at once when nothing is saving", async () => {
    const store = fakeStore(false);
    await waitForSaves(store.isSaving, store.subscribe);
    expect(store.listenerCount()).toBe(0);
  });

  test("waits for an in-flight save, then unsubscribes", async () => {
    const store = fakeStore(true);
    let resolved = false;
    const waiting = waitForSaves(store.isSaving, store.subscribe).then(() => {
      resolved = true;
    });
    await Promise.resolve();
    expect(resolved).toBe(false);

    store.finish();
    await waiting;
    expect(resolved).toBe(true);
    expect(store.listenerCount()).toBe(0);
  });

  test("gives up after the timeout so the button never spins forever", async () => {
    const store = fakeStore(true);
    const started = Date.now();
    await waitForSaves(store.isSaving, store.subscribe, 30);
    expect(Date.now() - started).toBeGreaterThanOrEqual(25);
    expect(store.listenerCount()).toBe(0);
  });
});
