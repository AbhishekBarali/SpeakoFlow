import { create } from "zustand";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { listen } from "@tauri-apps/api/event";
import {
  INSTALLER_PROGRESS_EVENT,
  downloadUpdateInstaller,
  getUpdateSupport,
  openUpdateInstaller,
  revealUpdateInstaller,
  type InstallerProgress,
  type UpdateMode,
} from "./updateCommands";
import { classifyCheckError, percentOf } from "./updateLogic";

/**
 * The whole update flow in one place, so the sidebar pill, the About card and
 * the tray all show the same state instead of each running its own check.
 *
 * Before 1.5.0 the only update UI was a component rendered with
 * `className="hidden"`: a check could succeed and nobody would ever see it.
 * Everything here is visible by construction — a found update puts a pill in
 * the sidebar until it is installed.
 */

export type UpdatePhase =
  | "idle"
  | "checking"
  | "upToDate"
  | "available"
  | "downloading"
  | "installing"
  | "restartRequired"
  | "savingInstaller"
  | "installerSaved";

export type UpdateErrorKind = "check" | "noBuild" | "install" | "download";

export interface AvailableUpdate {
  version: string;
  notes: string;
  date: string | null;
}

interface UpdateState {
  phase: UpdatePhase;
  available: AvailableUpdate | null;
  /** 0–100 while downloading; null when the size is unknown. */
  progress: number | null;
  /** Shown only for things the person asked for; background failures are logged. */
  error: { kind: UpdateErrorKind; detail: string } | null;
  mode: UpdateMode | null;
  releasePage: string;
  installerPath: string | null;
  lastCheckedAt: number | null;

  loadSupport: () => Promise<void>;
  check: (options?: { manual?: boolean }) => Promise<void>;
  install: () => Promise<void>;
  downloadInstaller: () => Promise<void>;
  openInstaller: () => Promise<void>;
  revealInstaller: () => Promise<void>;
  restart: () => Promise<void>;
}

const DEFAULT_RELEASE_PAGE =
  "https://github.com/AbhishekBarali/SpeakoFlow/releases/latest";

/** The plugin's handle for the found update. Not serialisable, so not in state. */
let pending: Update | null = null;

const BUSY: UpdatePhase[] = [
  "checking",
  "downloading",
  "installing",
  "savingInstaller",
];

const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

export const useUpdateStore = create<UpdateState>((set, get) => ({
  phase: "idle",
  available: null,
  progress: null,
  error: null,
  mode: null,
  releasePage: DEFAULT_RELEASE_PAGE,
  installerPath: null,
  lastCheckedAt: null,

  loadSupport: async () => {
    if (get().mode) return;
    try {
      const support = await getUpdateSupport();
      set({ mode: support.mode, releasePage: support.release_page });
    } catch (error) {
      // An older backend without the command: the in-app path is the one
      // every packaged build had.
      console.warn("get_update_support failed:", error);
      set({ mode: "in_app" });
    }
  },

  check: async ({ manual = false } = {}) => {
    const { phase } = get();
    if (BUSY.includes(phase)) return;
    // A found update stays found; a background re-check must not hide the pill.
    if (
      !manual &&
      (phase === "available" ||
        phase === "installerSaved" ||
        phase === "restartRequired")
    ) {
      return;
    }
    await get().loadSupport();
    set({ phase: "checking", error: null });

    try {
      const update = await check({ timeout: 30_000 });
      set({ lastCheckedAt: Date.now() });
      if (!update) {
        set({ phase: "upToDate", available: null });
        return;
      }
      if (pending && pending !== update) void pending.close().catch(() => {});
      pending = update;
      set({
        phase: "available",
        available: {
          version: update.version,
          notes: update.body ?? "",
          date: update.date ?? null,
        },
      });
    } catch (error) {
      set({ lastCheckedAt: Date.now() });
      const failure = classifyCheckError(error);
      console.warn("Update check:", failure, errorText(error));
      if (failure === "noUpdate") {
        set({ phase: "upToDate", available: null });
        return;
      }
      set({
        phase: get().available ? "available" : "idle",
        error: manual
          ? {
              kind: failure === "noBuild" ? "noBuild" : "check",
              detail: errorText(error),
            }
          : null,
      });
    }
  },

  install: async () => {
    const { mode } = get();
    if (mode !== "in_app") {
      await get().downloadInstaller();
      return;
    }
    if (!pending) {
      await get().check({ manual: true });
      if (!pending) return;
    }
    const update = pending;
    set({ phase: "downloading", progress: 0, error: null });

    let total: number | null = null;
    let done = 0;
    try {
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? null;
            done = 0;
            set({ progress: percentOf(0, total) });
            break;
          case "Progress":
            done += event.data.chunkLength;
            set({ progress: percentOf(done, total) });
            break;
          case "Finished":
            // On Windows the installer takes over from here and the app exits.
            set({ phase: "installing", progress: 100 });
            break;
        }
      });
    } catch (error) {
      console.error("Update install failed:", error);
      set({
        phase: "available",
        progress: null,
        error: { kind: "install", detail: errorText(error) },
      });
      return;
    }

    // macOS and Linux replace the app while it runs; it needs a restart to
    // become the new version.
    set({ phase: "restartRequired", progress: null });
    try {
      await relaunch();
    } catch (error) {
      console.warn("Relaunch after update failed:", error);
    }
  },

  downloadInstaller: async () => {
    if (BUSY.includes(get().phase)) return;
    const previous = get().phase;
    set({ phase: "savingInstaller", progress: 0, error: null });
    const unlisten = await listen<InstallerProgress>(
      INSTALLER_PROGRESS_EVENT,
      (event) =>
        set({
          progress: percentOf(event.payload.downloaded, event.payload.total),
        }),
    );
    try {
      const saved = await downloadUpdateInstaller();
      set({
        phase: "installerSaved",
        installerPath: saved.path,
        progress: 100,
      });
    } catch (error) {
      set({
        phase: previous === "installerSaved" ? "idle" : previous,
        progress: null,
        error: { kind: "download", detail: errorText(error) },
      });
    } finally {
      unlisten();
    }
  },

  openInstaller: async () => {
    const path = get().installerPath;
    if (!path) return;
    try {
      await openUpdateInstaller(path);
    } catch (error) {
      set({ error: { kind: "download", detail: errorText(error) } });
    }
  },

  revealInstaller: async () => {
    const path = get().installerPath;
    if (!path) return;
    try {
      await revealUpdateInstaller(path);
    } catch (error) {
      console.warn("Reveal installer failed:", error);
    }
  },

  restart: async () => {
    await relaunch();
  },
}));

/** Phases in which the sidebar shows the update pill. */
export const PILL_PHASES: UpdatePhase[] = [
  "available",
  "downloading",
  "installing",
  "restartRequired",
  "savingInstaller",
  "installerSaved",
];
