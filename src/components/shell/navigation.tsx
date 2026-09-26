import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
} from "react";

/**
 * Where the main window is, how to get somewhere else, and how to get back.
 *
 * Pages link to each other constantly — Home's model list opens a Models tab,
 * AI cleanup's model row opens the same tab, History's storage link opens a
 * Settings tab — so navigation is a context rather than props threaded through
 * every page.
 *
 * **Back goes where you came from.** A deep link pushes the current location
 * onto `history`; the target page's header then offers "← Home" (or whatever
 * the previous page was) and `goBack` pops it. A sidebar click is a fresh start
 * and clears the stack, the way a browser's address bar would. The old model
 * slot pages hard-coded their back button to Models, so a user who opened the
 * speech model from Home and pressed Back landed on a page they had never
 * visited.
 */

export type PageId =
  | "home"
  | "insights"
  | "history"
  | "assistant"
  | "meetings"
  | "cleanup"
  | "dictionary"
  | "models";

/** The four jobs a model does in SpeakoFlow — one tab each on Models. */
export type ModelSlot = "stt" | "cleanup" | "assistant" | "voice";

export const MODEL_SLOTS: ModelSlot[] = [
  "stt",
  "cleanup",
  "assistant",
  "voice",
];

export type SettingsTab =
  | "general"
  | "shortcuts"
  | "audio"
  | "dictation"
  | "privacy"
  | "advanced"
  | "about"
  | "debug";

export interface NavLocation {
  page: PageId;
  /** The Models tab, when `page` is "models". */
  modelSlot: ModelSlot | null;
}

export interface NavigationState extends NavLocation {
  /** Open Settings tab; null when the Settings dialog is closed. */
  settingsTab: SettingsTab | null;
  /** Locations a deep link left behind, newest last. */
  history: NavLocation[];
  /**
   * Bumped when the user clicks the sidebar item of the page they are already
   * on. Pages with a drill-down (an open meeting) listen for it and return to
   * their top level — the thing a second click on a nav item is expected to do.
   */
  resets: Partial<Record<PageId, number>>;
}

export const INITIAL_NAVIGATION: NavigationState = {
  page: "home",
  modelSlot: null,
  settingsTab: null,
  history: [],
  resets: {},
};

/** Deepest back stack kept. A cycle of deep links cannot grow it forever. */
const MAX_HISTORY = 12;

export interface NavigationValue extends NavigationState {
  /** Follow a link to another page. Remembers where you were. */
  navigate: (page: PageId) => void;
  /** A sidebar click: go to a page and forget the back stack. */
  navigateRoot: (page: PageId) => void;
  /** Open Models on one job's tab. Remembers where you were. */
  openModelSlot: (slot: ModelSlot) => void;
  /** Switch tabs within Models without adding a back step. */
  setModelTab: (slot: ModelSlot) => void;
  /** The location Back would return to, if any. */
  back: NavLocation | null;
  goBack: () => void;
  openSettings: (tab?: SettingsTab) => void;
  closeSettings: () => void;
}

const NavigationContext = createContext<NavigationValue | null>(null);

const sameLocation = (a: NavLocation, b: NavLocation) =>
  a.page === b.page && (a.page !== "models" || a.modelSlot === b.modelSlot);

const pushed = (state: NavigationState, next: NavLocation): NavigationState => {
  const here: NavLocation = { page: state.page, modelSlot: state.modelSlot };
  if (sameLocation(here, next)) {
    return { ...state, ...next, settingsTab: null };
  }
  // Coming back to a page that is already on the stack unwinds to it, so
  // Home → Models → Home → Models does not leave four steps to click through.
  const existing = state.history.findIndex((entry) =>
    sameLocation(entry, next),
  );
  const history =
    existing >= 0
      ? state.history.slice(0, existing)
      : [...state.history, here].slice(-MAX_HISTORY);
  return { ...state, ...next, history, settingsTab: null };
};

export const NavigationProvider: React.FC<{
  state: NavigationState;
  setState: React.Dispatch<React.SetStateAction<NavigationState>>;
  children: React.ReactNode;
}> = ({ state, setState, children }) => {
  const navigate = useCallback(
    (page: PageId) =>
      setState((current) =>
        pushed(current, {
          page,
          // Models keeps the tab it was last on; other pages leave it alone.
          modelSlot:
            page === "models"
              ? (current.modelSlot ?? "stt")
              : current.modelSlot,
        }),
      ),
    [setState],
  );

  const navigateRoot = useCallback(
    (page: PageId) =>
      setState((current) => {
        const resets =
          current.page === page
            ? { ...current.resets, [page]: (current.resets[page] ?? 0) + 1 }
            : current.resets;
        return {
          ...current,
          page,
          modelSlot:
            page === "models"
              ? current.page === "models"
                ? current.modelSlot
                : (current.modelSlot ?? "stt")
              : current.modelSlot,
          history: [],
          resets,
        };
      }),
    [setState],
  );

  const openModelSlot = useCallback(
    (slot: ModelSlot) =>
      setState((current) =>
        current.page === "models"
          ? { ...current, modelSlot: slot, settingsTab: null }
          : pushed(current, { page: "models", modelSlot: slot }),
      ),
    [setState],
  );

  const setModelTab = useCallback(
    (slot: ModelSlot) =>
      setState((current) => ({ ...current, page: "models", modelSlot: slot })),
    [setState],
  );

  const goBack = useCallback(
    () =>
      setState((current) => {
        const previous = current.history[current.history.length - 1];
        if (!previous) return current;
        return {
          ...current,
          page: previous.page,
          modelSlot:
            previous.page === "models" ? previous.modelSlot : current.modelSlot,
          history: current.history.slice(0, -1),
        };
      }),
    [setState],
  );

  const openSettings = useCallback(
    (tab: SettingsTab = "general") =>
      setState((current) => ({ ...current, settingsTab: tab })),
    [setState],
  );
  const closeSettings = useCallback(
    () => setState((current) => ({ ...current, settingsTab: null })),
    [setState],
  );

  // Mouse "back" button and Alt+← behave like they do everywhere else.
  const canGoBack = state.history.length > 0 && state.settingsTab === null;
  useEffect(() => {
    if (!canGoBack) return;
    const onMouseUp = (event: MouseEvent) => {
      if (event.button === 3) {
        event.preventDefault();
        goBack();
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.altKey && event.key === "ArrowLeft") {
        event.preventDefault();
        goBack();
      }
    };
    window.addEventListener("mouseup", onMouseUp);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mouseup", onMouseUp);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [canGoBack, goBack]);

  const value = useMemo<NavigationValue>(
    () => ({
      ...state,
      navigate,
      navigateRoot,
      openModelSlot,
      setModelTab,
      back: state.history[state.history.length - 1] ?? null,
      goBack,
      openSettings,
      closeSettings,
    }),
    [
      state,
      navigate,
      navigateRoot,
      openModelSlot,
      setModelTab,
      goBack,
      openSettings,
      closeSettings,
    ],
  );

  return (
    <NavigationContext.Provider value={value}>
      {children}
    </NavigationContext.Provider>
  );
};

/** No-op fallback so components still render outside the shell (tests). */
const FALLBACK: NavigationValue = {
  ...INITIAL_NAVIGATION,
  navigate: () => {},
  navigateRoot: () => {},
  openModelSlot: () => {},
  setModelTab: () => {},
  back: null,
  goBack: () => {},
  openSettings: () => {},
  closeSettings: () => {},
};

export const useNavigation = (): NavigationValue =>
  useContext(NavigationContext) ?? FALLBACK;

/**
 * Run `onReset` when the user re-clicks this page's sidebar item. Skips the
 * initial value so mounting a page never counts as a reset.
 */
export const usePageReset = (page: PageId, onReset: () => void): void => {
  const count = useNavigation().resets[page] ?? 0;
  const handler = React.useRef(onReset);
  handler.current = onReset;
  const seen = React.useRef(count);
  useEffect(() => {
    if (count === seen.current) return;
    seen.current = count;
    handler.current();
  }, [count]);
};
