import { describe, expect, test } from "bun:test";
import {
  HEAD_START,
  PROGRESS_CEILING,
  SEAL_MIN_MS,
  SEAL_MS,
  createWorkRun,
  emptyTimings,
  estimateSeconds,
  learn,
  priorFit,
  sealDuration,
  spreadOf,
  stageFraction,
  unsureFraction,
  type Timings,
  type TimingStore,
  type WorkRun,
} from "./workProgress";

const memoryStore = (initial?: Timings) => {
  let saved: Timings | null = initial ?? null;
  const store: TimingStore & { saved: () => Timings | null } = {
    load: () => saved ?? emptyTimings(),
    save: (timings) => {
      saved = timings;
    },
    saved: () => saved,
  };
  return store;
};

/** A store that has already watched `count` waits of `seconds` on `setup`. */
const trainedStore = (
  stage: "transcribing" | "processing",
  setup: string,
  recordingSec: number,
  seconds: number,
  count = 10,
) => {
  let fit = priorFit(stage);
  for (let i = 0; i < count; i++) fit = learn(fit, recordingSec, seconds);
  return memoryStore({ setups: { [`${stage}:${setup}`]: fit } });
};

const FRAME = 1000 / 60;

/** Draw a run the way a 60 Hz display does — one value per frame, in order —
 * firing `events` at their times. Returns the fill on each frame. */
const drawn = (
  run: WorkRun,
  until: number,
  events: [number, (at: number) => void][],
) => {
  const out: number[] = [];
  const pending = [...events];
  for (let t = 0; t <= until; t += FRAME) {
    while (pending.length && pending[0][0] <= t) pending.shift()![1](t);
    out.push(run.value(t));
  }
  return out;
};

describe("stage curve", () => {
  test("eases out gently, keeps creeping past the estimate, never reaches the end", () => {
    expect(stageFraction(0, 2)).toBe(0);
    expect(stageFraction(2, 2)).toBeCloseTo(0.8, 6);
    // About twice as fast at the start as at the estimate — not the ten times
    // of an exponential, which spent the end of every wait crawling.
    const early = stageFraction(0.02, 2) / 0.02;
    const late = (stageFraction(2, 2) - stageFraction(1.98, 2)) / 0.02;
    expect(early / late).toBeGreaterThan(1.8);
    expect(early / late).toBeLessThan(2.8);
    let previous = 0;
    let previousStep = Infinity;
    for (let s = 0.25; s <= 40; s += 0.25) {
      const value = stageFraction(s, 2);
      const step = value - previous;
      expect(value).toBeGreaterThan(previous);
      expect(value).toBeLessThan(1);
      expect(step).toBeLessThanOrEqual(previousStep + 1e-12);
      previous = value;
      previousStep = step;
    }
    // Still visibly moving at ten times the estimate: a long wait creeps.
    expect(stageFraction(20, 2) - stageFraction(18, 2)).toBeGreaterThan(0.001);
  });

  test("invalid input reads as not started", () => {
    for (const [elapsed, estimate] of [
      [NaN, 2],
      [-1, 2],
      [1, 0],
      [1, NaN],
    ]) {
      expect(stageFraction(elapsed, estimate)).toBe(0);
      expect(unsureFraction(elapsed, estimate)).toBe(0);
    }
  });

  // The curve used while a setup is unknown. Ten times longer than its pace
  // must still be visibly short of done and still moving, rather than parked
  // a hair under the end.
  test("the unsure curve keeps room and keeps moving in a long wait", () => {
    expect(unsureFraction(2, 2)).toBeGreaterThan(0.5);
    expect(unsureFraction(20, 2)).toBeLessThan(0.9);
    expect(unsureFraction(20, 2) - unsureFraction(10, 2)).toBeGreaterThan(0.05);
    let previous = 0;
    for (let s = 0.25; s <= 120; s += 0.25) {
      const value = unsureFraction(s, 2);
      expect(value).toBeGreaterThan(previous);
      expect(value).toBeLessThan(1);
      previous = value;
    }
  });
});

describe("estimate", () => {
  test("starts from the prior and scales with the recording", () => {
    const fit = priorFit("processing");
    expect(estimateSeconds(fit, 20)).toBeGreaterThan(estimateSeconds(fit, 3));
    expect(estimateSeconds(fit, 3)).toBeCloseTo(1.6 + 0.08 * 3, 4);
  });

  test("learns this machine's pace within a few dictations", () => {
    let fit = priorFit("processing");
    for (let i = 0; i < 5; i++) {
      fit = learn(fit, 5, 4 + 0.3 * 5);
      fit = learn(fit, 25, 4 + 0.3 * 25);
    }
    expect(estimateSeconds(fit, 15)).toBeGreaterThan(7.2);
    expect(estimateSeconds(fit, 15)).toBeLessThan(9.8);
  });

  // A fast local model's waits sit well under the prior's line. They used to
  // be overestimated by 70%, because the intercept's floor lifted the line.
  test("a machine faster than the prior is estimated at its own pace", () => {
    let fit = priorFit("transcribing");
    for (let i = 0; i < 8; i++) fit = learn(fit, 4, 0.3);
    expect(estimateSeconds(fit, 4)).toBeGreaterThan(0.27);
    expect(estimateSeconds(fit, 4)).toBeLessThan(0.34);
    // A recording far shorter than anything learned is still a real wait.
    expect(estimateSeconds(fit, 0)).toBeGreaterThanOrEqual(0.3);
  });

  test("one cold start cannot make every later wait look slow", () => {
    let fit = priorFit("transcribing");
    for (let i = 0; i < 6; i++) fit = learn(fit, 8, 0.8);
    const before = estimateSeconds(fit, 8);
    const after = estimateSeconds(learn(fit, 8, 40), 8);
    expect(after).toBeLessThan(before * 3);
  });

  // The prior is a guess about hardware it has never seen. The first real
  // wait is better information than any guess, whatever machine it came from.
  test("the first real wait of a setup replaces the guess outright", () => {
    for (const seconds of [0.25, 16.8]) {
      const fit = learn(priorFit("processing"), 8, seconds);
      expect(estimateSeconds(fit, 8)).toBeCloseTo(seconds, 1);
    }
  });

  // Same name, different reality: a GPU switched off, a network gone slow,
  // a different engine behind the same id. One surprise is a one-off; two the
  // same way is the new normal, and is adopted at once.
  test("two surprises in a row mean the setup itself changed", () => {
    let fit = priorFit("transcribing");
    for (let i = 0; i < 8; i++) fit = learn(fit, 8, 0.3);
    fit = learn(fit, 8, 4.5);
    expect(estimateSeconds(fit, 8)).toBeLessThan(1);
    fit = learn(fit, 8, 4.5);
    expect(estimateSeconds(fit, 8)).toBeCloseTo(4.5, 1);
  });

  test("it learns how much a setup's waits vary", () => {
    expect(spreadOf(priorFit("processing"))).toBeCloseTo(1, 6);
    let steady = priorFit("transcribing");
    let jittery = priorFit("transcribing");
    const jitter = [1, 0.5, 1.8, 0.7, 1.5, 0.6, 1.9, 0.8, 1.3, 0.55];
    for (let i = 0; i < 10; i++) {
      steady = learn(steady, 8, 0.3 * (i % 2 ? 1.05 : 0.95));
      jittery = learn(jittery, 8, 1.4 * jitter[i]);
    }
    expect(spreadOf(steady)).toBeLessThan(0.3);
    expect(spreadOf(jittery)).toBeGreaterThan(spreadOf(steady) + 0.2);
  });

  test("garbage samples are ignored and the estimate stays bounded", () => {
    const fit = priorFit("transcribing");
    for (const [x, y] of [
      [NaN, 2],
      [5, NaN],
      [5, -3],
      [5, 0],
    ])
      expect(learn(fit, x, y)).toEqual(fit);
    expect(estimateSeconds(fit, 1e9)).toBeLessThanOrEqual(60);
    expect(estimateSeconds(fit, null)).toBeGreaterThan(0);
  });
});

describe("work run", () => {
  test("starts with a head start and already moving", () => {
    const run = createWorkRun(memoryStore());
    run.begin("transcribing", 0, { recordingSec: 6 });
    expect(run.value(0)).toBeCloseTo(HEAD_START, 9);
    expect(run.value(FRAME)).toBeGreaterThan(HEAD_START + 0.004);
  });

  test("fills forward only, never claims done, and runs out on finish", () => {
    const run = createWorkRun(memoryStore());
    run.begin("transcribing", 0, { recordingSec: 6 });
    let previous = 0;
    for (let t = 50; t <= 120_000; t += 50) {
      const value = run.value(t);
      expect(value).toBeGreaterThanOrEqual(previous);
      expect(value).toBeLessThanOrEqual(PROGRESS_CEILING);
      previous = value;
    }
    expect(run.finish(120_000)).toBeGreaterThanOrEqual(SEAL_MIN_MS);
    expect(run.running()).toBe(false);
    let last = previous;
    for (let t = 120_000; t <= 120_000 + SEAL_MS; t += FRAME) {
      const value = run.value(t);
      expect(value).toBeGreaterThanOrEqual(last - 1e-12);
      expect(value).toBeLessThanOrEqual(1);
      last = value;
    }
    expect(run.value(120_000 + SEAL_MS)).toBe(1);
  });

  // The complaint: the text was in the app and the fill was still finishing.
  // A fill that is nearly there must land in a beat, not take the same long
  // run-out as one that is barely started.
  test("the run-out is as short as the distance left allows", () => {
    expect(sealDuration(0)).toBe(SEAL_MIN_MS);
    expect(sealDuration(0.25)).toBeLessThan(180);
    expect(sealDuration(0.6)).toBeGreaterThan(sealDuration(0.25));
    expect(sealDuration(1)).toBe(SEAL_MS);
    let previous = 0;
    for (let d = 0; d <= 1; d += 0.05) {
      expect(sealDuration(d)).toBeGreaterThanOrEqual(previous);
      previous = sealDuration(d);
    }

    const run = createWorkRun(memoryStore());
    run.begin("transcribing", 0, { recordingSec: 6 });
    run.value(120_000);
    const took = run.finish(120_000);
    expect(took).toBe(SEAL_MIN_MS);
    expect(run.value(120_000 + SEAL_MIN_MS)).toBe(1);
  });

  // The two halves of the trade-off, as the user sees them.
  test("an unknown setup never parks near the end, however long it takes", () => {
    const run = createWorkRun(memoryStore());
    run.begin("transcribing", 0, { recordingSec: 8, setup: "new-model" });
    expect(run.value(10_000)).toBeLessThan(0.85);
    // Still advancing visibly between eight and ten seconds in.
    expect(run.value(10_000) - run.value(8_000)).toBeGreaterThanOrEqual(0);
    const at8 = createWorkRun(memoryStore());
    at8.begin("transcribing", 0, { recordingSec: 8, setup: "new-model" });
    const eight = at8.value(8_000);
    expect(at8.value(10_000) - eight).toBeGreaterThan(0.01);
  });

  test("a setup it knows lands near the end as the text arrives", () => {
    const store = trainedStore("transcribing", "parakeet", 4, 0.3);
    const run = createWorkRun(store);
    run.begin("transcribing", 0, { recordingSec: 4, setup: "parakeet" });
    for (let t = 0; t <= 300; t += FRAME) run.value(t);
    expect(run.value(300)).toBeGreaterThan(0.6);
    // Knowledge of one setup says nothing about another.
    const other = createWorkRun(store);
    other.begin("transcribing", 0, { recordingSec: 4, setup: "whisper" });
    for (let t = 0; t <= 300; t += FRAME) other.value(t);
    expect(other.value(300)).toBeLessThan(run.value(300) - 0.2);
  });

  test("each setup is learned on its own, and switching back keeps it", () => {
    const store = memoryStore();
    for (const [setup, seconds] of [
      ["fast", 0.3],
      ["slow", 5],
      ["fast", 0.3],
    ] as const) {
      const run = createWorkRun(store);
      run.begin("transcribing", 0, { recordingSec: 8, setup });
      run.finish(seconds * 1000);
    }
    const setups = store.saved()!.setups;
    expect(estimateSeconds(setups["transcribing:fast"], 8)).toBeCloseTo(0.3, 1);
    expect(estimateSeconds(setups["transcribing:slow"], 8)).toBeCloseTo(5, 1);
    // Most recently used last, which is the order forgetting works from.
    expect(Object.keys(setups).at(-1)).toBe("transcribing:fast");
  });

  // The backend runs the fill out when the text is ready, before the paste,
  // and the finish arrives once the paste has returned. That second finish
  // must neither restart the run-out nor learn the stage a second time.
  test("a finish after the run-out has begun changes nothing", () => {
    const store = memoryStore();
    const run = createWorkRun(store);
    run.begin("transcribing", 0, { recordingSec: 5 });
    run.value(300);
    const took = run.finish(300);
    const learned = store.saved();
    expect(learned).not.toBeNull();
    const mid = run.value(300 + took / 2);
    expect(run.finish(300 + took / 2)).toBe(0);
    expect(store.saved()).toBe(learned);
    expect(run.value(300 + took / 2)).toBe(mid);
    expect(run.value(300 + took)).toBe(1);
  });

  // The complaint this exists for: the fill crawled, then shot to the end
  // (×13 in speed at the finish, ×2.5 at the hand-off to cleanup). The
  // per-frame step may grow or shrink, but only gradually.
  for (const [name, events] of [
    [
      "a fast dictation",
      (run: WorkRun) =>
        [
          [
            0,
            (at: number) => run.begin("transcribing", at, { recordingSec: 5 }),
          ],
          [300, (at: number) => void run.finish(at)],
        ] as [number, (at: number) => void][],
    ],
    [
      "a dictation with cleanup",
      (run: WorkRun) =>
        [
          [
            0,
            (at: number) =>
              run.begin("transcribing", at, {
                recordingSec: 5,
                cleanupNext: true,
              }),
          ],
          [300, (at: number) => run.begin("processing", at)],
          [1800, (at: number) => void run.finish(at)],
        ] as [number, (at: number) => void][],
    ],
    [
      "a cleanup far longer than expected",
      (run: WorkRun) =>
        [
          [
            0,
            (at: number) =>
              run.begin("transcribing", at, {
                recordingSec: 5,
                cleanupNext: true,
              }),
          ],
          [400, (at: number) => run.begin("processing", at)],
          [9000, (at: number) => void run.finish(at)],
        ] as [number, (at: number) => void][],
    ],
  ] as const) {
    test(`${name}: the speed never jumps`, () => {
      const run = createWorkRun(memoryStore());
      const frames = drawn(run, 10_000, events(run));
      const steps = frames.slice(1).map((v, i) => v - frames[i]);
      for (let i = 1; i < steps.length; i++)
        expect(Math.abs(steps[i] - steps[i - 1])).toBeLessThan(0.02);
      expect(frames[frames.length - 1]).toBe(1);
    });
  }

  test("transcription leaves room for the cleanup that follows it", () => {
    const run = createWorkRun(memoryStore());
    run.begin("transcribing", 0, { recordingSec: 10, cleanupNext: true });
    // Transcription running long still stays inside its share.
    const share = run.value(60_000);
    expect(share).toBeLessThan(PROGRESS_CEILING * 0.6);
    run.begin("processing", 60_000);
    // Cleanup carries on from exactly where transcription left the fill.
    expect(run.value(60_000)).toBeCloseTo(share, 9);
    expect(run.value(65_000)).toBeGreaterThan(share);
  });

  test("without cleanup, transcription gets the whole bar", () => {
    const run = createWorkRun(memoryStore());
    run.begin("transcribing", 0, { recordingSec: 10 });
    expect(run.value(30_000)).toBeGreaterThan(PROGRESS_CEILING * 0.9);
  });

  test("a completion with nothing running still runs out to the end", () => {
    const store = memoryStore();
    const run = createWorkRun(store);
    expect(run.finish(0)).toBe(SEAL_MS);
    expect(run.value(SEAL_MS / 2)).toBeGreaterThan(0);
    expect(run.value(SEAL_MS)).toBe(1);
    expect(store.saved()).toBeNull();
  });

  test("learns from a successful run only", () => {
    const store = memoryStore();
    const run = createWorkRun(store);
    run.begin("transcribing", 0, { recordingSec: 8, cleanupNext: true });
    run.begin("processing", 1_000);
    run.abandon();
    expect(store.saved()).toBeNull();

    run.begin("transcribing", 0, { recordingSec: 8, cleanupNext: true });
    run.begin("processing", 1_000);
    run.finish(4_000, { learn: false });
    expect(store.saved()).toBeNull();

    run.begin("transcribing", 0, { recordingSec: 8, cleanupNext: true });
    run.begin("processing", 1_000);
    run.finish(4_000);
    const saved = store.saved();
    expect(saved).not.toBeNull();
    expect(saved!.setups["processing:default"]).not.toEqual(
      priorFit("processing"),
    );
    expect(saved!.setups["transcribing:default"]).not.toEqual(
      priorFit("transcribing"),
    );
  });

  test("a recording of unknown length is estimated but not learned", () => {
    const store = memoryStore();
    const run = createWorkRun(store);
    run.begin("transcribing", 0, { recordingSec: null });
    expect(run.value(500)).toBeGreaterThan(HEAD_START);
    run.finish(1_000);
    expect(store.saved()).toBeNull();
  });

  test("a new run after a run-out starts again from the head start", () => {
    const run = createWorkRun(memoryStore());
    run.begin("transcribing", 0, { recordingSec: 4 });
    run.finish(2_000);
    run.begin("transcribing", 10_000, { recordingSec: 4 });
    expect(run.value(10_000)).toBeCloseTo(HEAD_START, 9);
  });
});
