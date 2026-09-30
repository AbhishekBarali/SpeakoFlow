import { describe, expect, test } from "bun:test";
import {
  mergeSetupTasks,
  type SetupTask,
  type SetupTaskInput,
  type SetupTaskState,
} from "./setupQueuePlan";
import type { SetupJob } from "./recommend";

const input = (job: SetupJob, modelId = `${job}-model`): SetupTaskInput => ({
  job,
  modelId,
  label: job,
  sizeMb: 100,
  wiring:
    job === "voice"
      ? { kind: "voice", engine: "kokoro", model: null }
      : { kind: job },
});

const task = (job: SetupJob, state: SetupTaskState): SetupTask => ({
  ...input(job),
  state,
});

describe("onboarding feature queue", () => {
  test("dictation setup queues only the speech model", () => {
    expect(mergeSetupTasks([], [input("stt")])).toEqual([
      task("stt", "waiting"),
    ]);
  });

  test("feature exploration appends choices while dictation downloads", () => {
    const current = [task("stt", "downloading")];
    const next = mergeSetupTasks(current, [input("cleanup")]);
    expect(next).toEqual([
      task("stt", "downloading"),
      task("cleanup", "waiting"),
    ]);
    expect(next[0]).toBe(current[0]);
    expect(current).toEqual([task("stt", "downloading")]);
  });

  test("a call puts its assistant before its voice, and speech first", () => {
    const next = mergeSetupTasks(
      [],
      [input("voice"), input("assistant"), input("stt")],
    );
    expect(next.map((entry) => entry.job)).toEqual([
      "stt",
      "assistant",
      "voice",
    ]);
  });

  test("repeated actions never replace a queued, active, or ready model", () => {
    for (const state of [
      "waiting",
      "downloading",
      "switching",
      "ready",
    ] as const) {
      const current = [task("assistant", state)];
      const next = mergeSetupTasks(current, [
        input("assistant", "another-model"),
      ]);
      expect(next).toBe(current);
      expect(next[0].modelId).toBe("assistant-model");
    }
  });

  test("a failed or cancelled feature can be chosen again", () => {
    for (const state of ["failed", "cancelled"] as const) {
      const current = [task("assistant", state), task("stt", "ready")];
      const next = mergeSetupTasks(current, [
        input("assistant", "another-model"),
      ]);
      expect(next).toEqual([
        task("stt", "ready"),
        { ...input("assistant", "another-model"), state: "waiting" },
      ]);
      expect(current[0].state).toBe(state);
    }
  });

  test("one feature action cannot add the same job twice", () => {
    const next = mergeSetupTasks([], [input("assistant"), input("assistant")]);
    expect(next).toEqual([task("assistant", "waiting")]);
  });
});
