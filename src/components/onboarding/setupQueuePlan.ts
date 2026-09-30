import { JOB_ORDER, type SetupJob } from "./recommend";

export type SetupTaskState =
  | "waiting"
  | "downloading"
  /** On disk, and its settings are being written. Too late to cancel. */
  | "switching"
  | "ready"
  | "failed"
  | "cancelled";

/** What a finished download switches on. */
export type SetupWiring =
  | { kind: "stt" }
  | { kind: "cleanup" }
  | { kind: "assistant" }
  | { kind: "voice"; engine: string; model: string | null };

export interface SetupTask {
  job: SetupJob;
  /** Catalog id, or null for the web-view voice (fetched by `VoicePrefetch`). */
  modelId: string | null;
  label: string;
  sizeMb: number;
  wiring: SetupWiring;
  state: SetupTaskState;
}

export type SetupTaskInput = Omit<SetupTask, "state">;

/**
 * Add explicit feature choices without disturbing downloads already running.
 * A failed or cancelled choice can be retried with a new model; active and
 * completed choices keep their original model and settings.
 */
export function mergeSetupTasks(
  current: SetupTask[],
  choices: readonly SetupTaskInput[],
): SetupTask[] {
  let tasks = current;
  for (const choice of choices) {
    const index = tasks.findIndex((task) => task.job === choice.job);
    const previous = tasks[index];
    if (
      previous &&
      previous.state !== "failed" &&
      previous.state !== "cancelled"
    ) {
      continue;
    }
    if (tasks === current) tasks = [...current];
    const task: SetupTask = { ...choice, state: "waiting" };
    if (index < 0) tasks.push(task);
    else tasks[index] = task;
  }
  return tasks === current
    ? current
    : tasks.sort((a, b) => JOB_ORDER.indexOf(a.job) - JOB_ORDER.indexOf(b.job));
}
