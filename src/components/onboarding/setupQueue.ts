import { create } from "zustand";
import { commands } from "@/bindings";
import { useModelStore } from "@/stores/modelStore";
import { useSettingsStore } from "@/stores/settingsStore";
import type { SetupJob } from "./recommend";
import {
  mergeSetupTasks,
  type SetupTask,
  type SetupTaskInput,
  type SetupTaskState,
} from "./setupQueuePlan";

export type {
  SetupTask,
  SetupTaskInput,
  SetupTaskState,
  SetupWiring,
} from "./setupQueuePlan";

/**
 * The downloads first-run setup starts, and what each one switches on once it
 * lands.
 *
 * A module-level store rather than component state, because the whole point is
 * that setup ends long before a 5 GB model finishes: the queue has to keep
 * running through the tour, the final screen and the main window, and wire the
 * model up whenever it arrives.
 *
 * Downloads run **one at a time**, in the order they were queued. Every model
 * download already fetches in parallel chunks, so running four at once would
 * only split the same connection four ways and make the speech model, the one
 * thing dictation needs, arrive last instead of first.
 *
 * Nothing is pointed at a model until its file is on disk: a download that fails
 * or is cancelled must not leave the assistant "set" to a model that does not
 * exist.
 */

interface SetupQueueState {
  tasks: SetupTask[];
  /** Download progress of the web-view voice, 0–100. */
  webviewVoiceProgress: number;
  /** Add explicit choices; active and ready jobs are never replaced. */
  start: (tasks: SetupTaskInput[]) => void;
  /** Run every failed task again. */
  retryFailed: () => void;
  /** Stop one download and drop it from the queue. */
  cancel: (job: SetupJob) => Promise<void>;
  /** Called by `VoicePrefetch` as the web-view voice downloads. */
  setWebviewVoiceProgress: (progress: number) => void;
  /** Called by `VoicePrefetch` when the web-view voice is ready or failed. */
  finishWebviewVoice: (ok: boolean) => void;
  /**
   * Forget every task that is over (ready, failed or cancelled), so a replayed
   * onboarding starts from a clean summary. Anything still waiting or
   * downloading is kept: a replay must never abandon a download.
   */
  clearSettled: () => void;
}

/** Longest wait for verification and unpacking after a download returns. */
const SETTLE_CAP_MS = 5 * 60_000;

let running = false;
let webviewVoiceDone: ((ok: boolean) => void) | null = null;
const cancellations = new Map<SetupJob, Promise<void>>();

const setState = (job: SetupJob, state: SetupTaskState) =>
  useSetupQueue.setState((s) => ({
    tasks: s.tasks.map((task) =>
      task.job === job ? { ...task, state } : task,
    ),
  }));

const isBusy = (modelId: string): boolean => {
  const s = useModelStore.getState();
  return (
    modelId in s.downloadingModels ||
    modelId in s.verifyingModels ||
    modelId in s.extractingModels
  );
};

const isOnDisk = (modelId: string): boolean =>
  useModelStore.getState().models.find((m) => m.id === modelId)
    ?.is_downloaded ?? false;

/**
 * Resolve once the model is on disk and nothing is still happening to it.
 *
 * `downloadModel` resolving `true` is not quite proof: the backend answers a
 * duplicate request for a download already in flight with success straight
 * away, and a voice pack is unpacked after its archive arrives. So this waits
 * for the store to agree.
 */
const settleOnDisk = async (modelId: string): Promise<boolean> => {
  await useModelStore.getState().loadModels();
  if (!isBusy(modelId)) return isOnDisk(modelId);
  return new Promise<boolean>((resolve) => {
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      stop();
      window.clearTimeout(cap);
      void useModelStore
        .getState()
        .loadModels()
        .then(() => resolve(isOnDisk(modelId)));
    };
    const stop = useModelStore.subscribe(() => {
      if (!isBusy(modelId)) finish();
    });
    // An event that never arrives must not hold the rest of the queue: after
    // this long, the file on disk decides.
    const cap = window.setTimeout(finish, SETTLE_CAP_MS);
  });
};

const download = async (task: SetupTask): Promise<boolean> => {
  if (task.modelId === null) {
    // The web-view voice has no catalog entry; `VoicePrefetch` (mounted for
    // the app's lifetime) loads it into the web view's cache.
    return new Promise<boolean>((resolve) => {
      webviewVoiceDone = resolve;
    });
  }
  const models = useModelStore.getState();
  if (isOnDisk(task.modelId) && !isBusy(task.modelId)) {
    return true;
  }
  const ok = await models.downloadModel(task.modelId);
  if (!ok) return false;
  return settleOnDisk(task.modelId);
};

const wire = async (task: SetupTask): Promise<void> => {
  const check = (result: { status: string; error?: unknown }) => {
    if (result.status !== "ok") throw new Error(String(result.error));
  };
  switch (task.wiring.kind) {
    case "stt":
      // Select only after the download settles. Setting a pending selection
      // during download also lets the completion event select it, which races
      // this queue and can report "ready" before selection has succeeded.
      if (!task.modelId) throw new Error("Missing speech model");
      useModelStore.getState().setPendingSttSelection(task.modelId);
      await useModelStore.getState().finalizePendingSttSelection(task.modelId);
      if (useModelStore.getState().currentModel !== task.modelId) {
        throw new Error("The speech model could not be selected");
      }
      break;
    case "cleanup":
      // Model first, then the feature: the reverse would briefly turn on a
      // cleanup pass with nothing to run it. `setCleanupLocalModel` also
      // pairs SpeakoFlow Mini with the prompt it was trained on.
      check(await commands.setCleanupLocalModel(task.modelId ?? ""));
      check(await commands.changePostProcessEnabledSetting(true));
      break;
    case "assistant":
      check(
        await commands.changeAssistantModelSetting(
          "builtin",
          task.modelId ?? "",
        ),
      );
      check(await commands.setAssistantProvider("builtin"));
      check(await commands.setAssistantEnabled(true));
      break;
    case "voice":
      // If a call requested both downloads, a failed assistant must not leave
      // spoken replies enabled for a model that never became usable.
      if (
        useSetupQueue
          .getState()
          .tasks.some(
            (queued) => queued.job === "assistant" && queued.state !== "ready",
          )
      ) {
        throw new Error("The assistant model is not ready");
      }
      // The engine first: the model setting is written to the current
      // engine's own slot.
      check(await commands.setAssistantTtsEngine(task.wiring.engine));
      if (task.wiring.model) {
        check(await commands.setAssistantTtsModel(task.wiring.model));
      }
      check(await commands.setAssistantTtsEnabled(true));
      break;
  }
  const settings = useSettingsStore.getState();
  await settings.refreshSettings();
  if (task.wiring.kind === "cleanup")
    await settings.refreshPostProcessReadiness();
};

const run = async (): Promise<void> => {
  if (running) return;
  running = true;
  try {
    for (;;) {
      let next = useSetupQueue
        .getState()
        .tasks.find((task) => task.state === "waiting");
      if (!next) break;
      // A retry can be queued while cancellation's backend command is still
      // returning. Let that request finish before starting the same model,
      // otherwise its late cleanup can erase the retry's progress/state.
      const cancellation = cancellations.get(next.job);
      if (cancellation) {
        await cancellation;
        next = useSetupQueue
          .getState()
          .tasks.find(
            (task) => task.job === next?.job && task.state === "waiting",
          );
        if (!next) continue;
      }
      setState(next.job, "downloading");
      let ok = false;
      try {
        ok = await download(next);
      } catch (error) {
        console.error(`Setup download failed for ${next.job}:`, error);
      }
      // A cancelled job may have been chosen again while its old request was
      // finishing. Never wire that old recipe or overwrite the new waiting one.
      const current = useSetupQueue
        .getState()
        .tasks.find((task) => task.job === next.job);
      if (current?.state !== "downloading") continue;
      if (!ok) {
        setState(next.job, "failed");
        continue;
      }
      setState(next.job, "switching");
      try {
        await wire(next);
        setState(next.job, "ready");
      } catch (error) {
        console.error(`Setup could not switch on ${next.job}:`, error);
        setState(next.job, "failed");
      }
    }
  } finally {
    running = false;
  }
};

export const useSetupQueue = create<SetupQueueState>()((set, get) => ({
  tasks: [],
  webviewVoiceProgress: 0,

  start: (tasks) => {
    const current = get().tasks;
    const next = mergeSetupTasks(current, tasks);
    if (next !== current) {
      set({
        tasks: next,
        ...(next.some(
          (task) => task.job === "voice" && task.state === "waiting",
        )
          ? { webviewVoiceProgress: 0 }
          : {}),
      });
    }
    void run();
  },

  retryFailed: () => {
    set((s) => ({
      tasks: s.tasks.map((task) =>
        task.state === "failed" ? { ...task, state: "waiting" } : task,
      ),
      webviewVoiceProgress: 0,
    }));
    void run();
  },

  cancel: async (job) => {
    const task = get().tasks.find((t) => t.job === job);
    if (!task || (task.state !== "waiting" && task.state !== "downloading")) {
      return;
    }
    const wasRunning = task.state === "downloading";
    let releaseCancellation = () => {};
    if (wasRunning) {
      cancellations.set(
        job,
        new Promise<void>((resolve) => {
          releaseCancellation = resolve;
        }),
      );
    }
    setState(job, "cancelled");
    if (!wasRunning) return;
    try {
      if (task.modelId === null) {
        webviewVoiceDone?.(false);
        webviewVoiceDone = null;
        return;
      }
      const models = useModelStore.getState();
      if (models.pendingSttSelection === task.modelId) {
        models.setPendingSttSelection(null);
      }
      await models.cancelDownload(task.modelId);
    } finally {
      cancellations.delete(job);
      releaseCancellation();
    }
  },

  setWebviewVoiceProgress: (progress) =>
    set({ webviewVoiceProgress: Math.max(0, Math.min(100, progress)) }),

  finishWebviewVoice: (ok) => {
    const done = webviewVoiceDone;
    webviewVoiceDone = null;
    done?.(ok);
  },

  clearSettled: () =>
    set((s) => ({
      tasks: s.tasks.filter(
        (task) =>
          task.state === "waiting" ||
          task.state === "downloading" ||
          task.state === "switching",
      ),
    })),
}));
