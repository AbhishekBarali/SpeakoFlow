/**
 * The overlay's working indicator as an estimate rather than a loop.
 *
 * It used to be a crest that crossed the bars every 1.6s and started over. One
 * crossing reads as a loading bar, so each restart read as "that wasn't it
 * either", and on a long cleanup the user watched it fail to finish a dozen
 * times. Now one fill runs left to right across the whole wait, never moves
 * backwards, slows as it goes, and only finishes when the work does.
 *
 * The wait it estimates cannot be known in advance. It depends on the speech
 * model or cloud service, the cleanup provider and model (any OpenAI-compatible
 * endpoint, any GGUF from Hugging Face), the machine, its drivers, and the
 * network, and the app has no business guessing any of that from a model's
 * name. So the rule is: **the estimate may only make the fill better; it must
 * never make it look stuck.** Concretely:
 *
 * - **It learns, per setup, on this computer.** The backend sends an opaque
 *   fingerprint of what is doing the work (`sttSetup` / `cleanupSetup`: the
 *   model or provider + model, never interpreted here), and each one keeps a
 *   small decaying least-squares fit of `seconds = a + b ÃƒÆ’Ã¢â‚¬â€ recording length`
 *   from real dictations. Hardware, platform and network are in the samples by
 *   construction, because they are measured where they happen. Switching
 *   between two setups does not wipe either.
 * - **Until it knows a setup, it does not pretend to.** The first real wait of
 *   a setup is taken at face value (the prior was only a guess), and so is a
 *   second surprise in a row, which means the setup itself changed (a driver,
 *   a network, a different engine behind the same name). A single surprise on
 *   a well-known setup ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â a model loading cold, one stalled request ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â is
 *   clamped, so it cannot make every later wait look slow.
 * - **It paces itself by how sure it is.** Each setup also learns how much its
 *   waits vary (`spreadOf`, the spread of log errors). The fill paces itself to
 *   a pessimistic duration (`pace`, about the 84th percentile), and the less
 *   sure it is, the more its shape moves toward a heavy-tailed curve
 *   (`unsureFraction`) that keeps advancing in long waits instead of sprinting
 *   to 90% and parking there. A consistent local model converges on a fill
 *   that lands with the text; a jittery cloud call keeps a looser one; a setup
 *   seen for the first time gets the cautious one. Running out from lower is
 *   the cheap mistake (a run-out of at most `SEAL_MS`); parking near the end
 *   for seconds is the expensive one.
 * - **Its speed never jumps.** The drawn fill rides the curve through a
 *   critically damped spring on its offset and relative speed
 *   (`FOLLOW_OMEGA`), so a hand-off bends the motion instead of kinking it, and
 *   the finish is a Hermite run-out that starts at the fill's current speed and
 *   lands with zero speed, as quickly as it can without snapping
 *   (`sealDuration`, 120ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Å“280ms). There is no check mark after it: a full,
 *   still row is the ending.
 * - **It ends with the text.** The run-out starts when the backend has the
 *   final text and is about to paste it (`seal-overlay`), not when the paste
 *   has returned.
 * - **It never claims to be done.** The fill is capped below the end
 *   (`PROGRESS_CEILING`) until the work is really done, and starts with a small
 *   head start (`HEAD_START`), so the first frame already shows motion.
 *
 * When cleanup follows transcription, the backend says so up front
 * (`cleanupNext`), and transcription only gets its share of the bar ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â otherwise
 * it would fill nearly to the end and leave cleanup, the long part, with no
 * room to move.
 */

export type WorkStage = "transcribing" | "processing";
export const WORK_STAGES: readonly WorkStage[] = ["transcribing", "processing"];

/** The fill never passes this until the work is actually done. */
export const PROGRESS_CEILING = 0.94;
/** Where a fresh run starts: about one bar of the pill, lit at once. */
export const HEAD_START = 0.06;
/** The confident curve: how far through its share a stage is when its pace
 * runs out, and how much faster it moves at the start than at that point
 * (1 + w : 1 ÃƒÂ¢Ã‹â€ Ã¢â‚¬â„¢ w). */
const AT_ESTIMATE = 0.8;
const EASE = 0.4;
/** The unsure curve: a Lomax (Pareto II) shape, whose tail thins out like
 * t^-k rather than exponentially, so a wait many times longer than expected
 * still visibly advances. It is at `UNSURE_AT_PACE` when the pace runs out. */
const UNSURE_SHAPE = 0.6;
const UNSURE_AT_PACE = 0.6;
/** How tightly the drawn fill rides the curve, in 1/s: how quickly a change of
 * the curve's pace is absorbed (about 0.15s), which is what rounds it off. */
const FOLLOW_OMEGA = 12;
/** The longest run-out to the end on success. Keep in step with
 * `PILL_SEAL_HOLD_MS` in overlay.rs, which holds the compact pill for it. */
export const SEAL_MS = 260;
/** The shortest: a fill already near the end still settles rather than
 * blinking full. */
export const SEAL_MIN_MS = 120;
/** The run-out's acceleration limit, in bars per secondÃƒâ€šÃ‚Â². A cubic that covers
 * `d` in `T` peaks at 6d/TÃƒâ€šÃ‚Â², so this sets how quickly it may land without the
 * motion reading as a snap: at 60 Hz no frame's change of speed exceeds
 * ~1.7% of the bar. */
const SEAL_ACCEL = 70;

/** How long the run-out takes for the distance left. A fixed 280ms made a
 * fill that was nearly there crawl the last few percent well after the text
 * had landed. */
export function sealDuration(remaining: number): number {
  const ms = 1000 * Math.sqrt((6 * Math.max(0, remaining)) / SEAL_ACCEL);
  return clamp(ms, SEAL_MIN_MS, SEAL_MS);
}

/** A setup's learned timing: a decaying weighted least-squares fit of duration
 * on recording length, plus how far its predictions tend to miss. */
export type Fit = {
  w: number;
  sx: number;
  sy: number;
  sxx: number;
  sxy: number;
  /** Decayed weight of real samples (the prior's pseudo-samples excluded). */
  n: number;
  /** Decayed sum of squared log errors, and its weight (prior included). */
  e2: number;
  ew: number;
  /** Direction of the last surprise (ÃƒÂ¢Ã‹â€ Ã¢â‚¬â„¢1 faster, +1 slower, 0 none). */
  miss: number;
};
/** Every setup this computer has used, most recently used last. */
export type Timings = { setups: Record<string, Fit> };

/** What a stage costs before anything has been learned. Only ever a starting
 * point: the first real wait replaces it outright. */
const PRIORS: Record<WorkStage, { a: number; b: number }> = {
  transcribing: { a: 0.6, b: 0.06 },
  processing: { a: 1.6, b: 0.08 },
};
/** The prior enters as two light pseudo-samples at a short and a long
 * recording, which also keeps the fit's slope defined while every real sample
 * so far has had the same length. */
const PRIOR_AT = [3, 20];
const PRIOR_WEIGHT = 0.6;
/** How far an unseen setup may be from its guess, as a log-scale spread: eÃƒâ€šÃ‚Â¹,
 * so anywhere from a third of the guess to three times it is unremarkable.
 * It enters with the weight of one sample, so a consistent setup is trusted
 * within about four dictations. */
const PRIOR_SPREAD = 1;
const PRIOR_SPREAD_WEIGHT = 1;
/** The spread is never taken as tighter than this: nothing is that regular. */
const MIN_SPREAD = 0.12;
const MAX_SPREAD = 1.2;
/** Spread at or below which the fill uses only the confident curve, and at or
 * above which it uses only the unsure one. */
const SURE_SPREAD = 0.35;
const UNSURE_SPREAD = 0.8;
/** How many standard deviations of log error the pace allows for: about the
 * 84th percentile of this setup's waits. */
const PACE_Z = 0.6;
/** Each new sample shrinks the weight of everything before it. */
const DECAY = 0.72;
/** A wait this many times off its estimate (and by at least this many
 * seconds) is a surprise. */
const SURPRISE_RATIO = 2.5;
const SURPRISE_SECONDS = 0.4;
/** Real samples a setup needs before a single surprise is treated as a
 * one-off rather than as news. */
const CONFIDENT_N = 1.5;
/** A recording of unknown length (a recovered dictation) is assumed typical. */
const TYPICAL_RECORDING_S = 8;
/** A local model often finishes a short dictation in about a quarter of a
 * second, and an estimate held above that leaves the fill far from the end
 * when the text arrives. */
const MIN_ESTIMATE_S = 0.3;
const MAX_ESTIMATE_S = 60;
/** Setups remembered; the least recently used is forgotten first. */
const MAX_SETUPS = 32;

const STORAGE_KEY = "speakoflow.overlay.workTimes.v2";
/** Per-stage timings without setups, which also learned the paste. */
const LEGACY_STORAGE_KEY = "speakoflow.overlay.workTimes.v1";

const finite = (value: number, fallback: number) =>
  Number.isFinite(value) ? value : fallback;
const clamp = (value: number, low: number, high: number) =>
  Math.max(low, Math.min(high, value));

function addPoint(fit: Fit, x: number, y: number, weight: number): Fit {
  return {
    ...fit,
    w: fit.w + weight,
    sx: fit.sx + weight * x,
    sy: fit.sy + weight * y,
    sxx: fit.sxx + weight * x * x,
    sxy: fit.sxy + weight * x * y,
  };
}

/** The pseudo-samples of a line `a + b ÃƒÆ’Ã¢â‚¬â€ x`, with an unsure spread. */
function lineFit(a: number, b: number): Fit {
  return PRIOR_AT.reduce(
    (fit, x) => addPoint(fit, x, Math.max(0.05, a + b * x), PRIOR_WEIGHT),
    {
      w: 0,
      sx: 0,
      sy: 0,
      sxx: 0,
      sxy: 0,
      n: 0,
      e2: PRIOR_SPREAD * PRIOR_SPREAD * PRIOR_SPREAD_WEIGHT,
      ew: PRIOR_SPREAD_WEIGHT,
      miss: 0,
    },
  );
}

export function priorFit(stage: WorkStage): Fit {
  const { a, b } = PRIORS[stage];
  return lineFit(a, b);
}

/** The fitted line's intercept and slope. */
function line(fit: Fit): { a: number; b: number } {
  const det = fit.w * fit.sxx - fit.sx * fit.sx;
  const b =
    det > 1e-6 ? clamp((fit.w * fit.sxy - fit.sx * fit.sy) / det, 0, 2) : 0;
  return { a: fit.w > 0 ? (fit.sy - b * fit.sx) / fit.w : 0, b };
}

/** Predicted seconds for a stage, given the recording's length. */
export function estimateSeconds(fit: Fit, recordingSec: number | null): number {
  const x = clamp(finite(recordingSec ?? TYPICAL_RECORDING_S, 8), 0, 600);
  if (!(fit.w > 0))
    return clamp(PRIORS.transcribing.a, MIN_ESTIMATE_S, MAX_ESTIMATE_S);
  // The floor applies to the prediction, not to the intercept. Flooring the
  // intercept lifted the whole line: a machine whose real waits all sat below
  // the prior's slope fitted a negative intercept, the floor raised it, and
  // every estimate came out long.
  const { a, b } = line(fit);
  return clamp(
    finite(Math.max(0.15, a + b * x), MIN_ESTIMATE_S),
    MIN_ESTIMATE_S,
    MAX_ESTIMATE_S,
  );
}

/** How far this setup's waits tend to land from their estimate, in log units:
 * 0.2 means about Ãƒâ€šÃ‚Â±20%. Wide until real samples say otherwise. */
export function spreadOf(fit: Fit): number {
  const spread = fit.ew > 0 ? Math.sqrt(fit.e2 / fit.ew) : PRIOR_SPREAD;
  return clamp(finite(spread, PRIOR_SPREAD), MIN_SPREAD, MAX_SPREAD);
}

/** Start over from one real wait: the fit's own line, scaled to pass through
 * it, carrying the weight of the prior plus that sample, with no claim yet
 * about how much the setup varies. */
function reseed(fit: Fit, x: number, seconds: number): Fit {
  const { a, b } = line(fit);
  const scale = clamp(seconds / Math.max(0.05, a + b * x), 0.02, 50);
  const seeded = lineFit(a * scale, b * scale);
  return { ...addPoint(seeded, x, seconds, 1), n: 1 };
}

/** Fold one finished stage into its setup's fit. */
export function learn(fit: Fit, recordingSec: number, seconds: number): Fit {
  if (!Number.isFinite(recordingSec) || !Number.isFinite(seconds)) return fit;
  if (seconds <= 0) return fit;
  const x = clamp(recordingSec, 0, 600);
  // Nothing real is known about this setup: the estimate was only the prior's
  // guess, so the wait itself is the best information there is.
  if (!(fit.n > 0)) return reseed(fit, x, seconds);
  const predicted = estimateSeconds(fit, x);
  const surprise =
    (seconds > predicted * SURPRISE_RATIO ||
      seconds < predicted / SURPRISE_RATIO) &&
    Math.abs(seconds - predicted) > SURPRISE_SECONDS;
  const direction = surprise ? Math.sign(seconds - predicted) : 0;
  // Two surprises in a row, the same way: the setup changed under its name.
  if (surprise && fit.miss === direction) return reseed(fit, x, seconds);
  // One surprise on a setup it knows well is most likely a one-off (a cold
  // model load, a stalled request): it still moves the estimate, but clamped.
  const sample =
    surprise && fit.n >= CONFIDENT_N
      ? clamp(seconds, predicted / SURPRISE_RATIO, predicted * SURPRISE_RATIO)
      : seconds;
  const error = Math.log(sample / predicted);
  const decayed: Fit = {
    w: fit.w * DECAY,
    sx: fit.sx * DECAY,
    sy: fit.sy * DECAY,
    sxx: fit.sxx * DECAY,
    sxy: fit.sxy * DECAY,
    n: fit.n * DECAY + 1,
    e2: fit.e2 * DECAY + error * error,
    ew: fit.ew * DECAY + 1,
    miss: direction,
  };
  return addPoint(decayed, x, sample, 1);
}

/** The confident curve: how far through its share of the bar a stage is,
 * `elapsed` seconds in. A gentle ease-out up to the pace, then a hyperbolic
 * tail with the same speed at the joint, which never reaches the end. */
export function stageFraction(elapsedSec: number, paceSec: number): number {
  if (!(elapsedSec > 0) || !(paceSec > 0)) return 0;
  const x = elapsedSec / paceSec;
  if (x <= 1) return AT_ESTIMATE * ((1 - EASE) * x + EASE * x * (2 - x));
  const left = 1 - AT_ESTIMATE;
  const slope = AT_ESTIMATE * (1 - EASE);
  return 1 - left / (1 + ((x - 1) * slope) / left);
}

/** `stageFraction`'s speed, per second. */
function stageSlope(elapsedSec: number, paceSec: number): number {
  if (!(paceSec > 0)) return 0;
  const x = Math.max(0, elapsedSec) / paceSec;
  if (x <= 1) return (AT_ESTIMATE * (1 - EASE + 2 * EASE * (1 - x))) / paceSec;
  const left = 1 - AT_ESTIMATE;
  const slope = AT_ESTIMATE * (1 - EASE);
  const d = 1 + ((x - 1) * slope) / left;
  return slope / (d * d) / paceSec;
}

/** The unsure curve's time scale, chosen so it is at `UNSURE_AT_PACE` when
 * the pace runs out. */
const unsureScale = (paceSec: number) =>
  paceSec /
  (UNSURE_SHAPE * (Math.pow(1 - UNSURE_AT_PACE, -1 / UNSURE_SHAPE) - 1));

/** The unsure curve: 1 ÃƒÂ¢Ã‹â€ Ã¢â‚¬â„¢ (1 + t / kÃƒÂÃ¢â‚¬Å¾)^ÃƒÂ¢Ã‹â€ Ã¢â‚¬â„¢k. It starts at a finite speed and
 * slows smoothly forever, so a wait ten times longer than expected has still
 * left about a seventh of the share to move through rather than a hundredth. */
export function unsureFraction(elapsedSec: number, paceSec: number): number {
  if (!(elapsedSec > 0) || !(paceSec > 0)) return 0;
  const k = UNSURE_SHAPE;
  return 1 - Math.pow(1 + elapsedSec / (k * unsureScale(paceSec)), -k);
}

function unsureSlope(elapsedSec: number, paceSec: number): number {
  if (!(paceSec > 0)) return 0;
  const k = UNSURE_SHAPE;
  const tau = unsureScale(paceSec);
  return Math.pow(1 + Math.max(0, elapsedSec) / (k * tau), -k - 1) / tau;
}

/** How a stage's share of the bar fills, decided from what is known about the
 * setup doing the work. */
export interface Curve {
  /** The duration the fill paces itself to, in seconds. */
  pace: number;
  /** 0 = nothing known, use the unsure curve; 1 = use the confident one. */
  sure: number;
}

export function curveFor(fit: Fit, recordingSec: number | null): Curve {
  const spread = spreadOf(fit);
  return {
    pace: estimateSeconds(fit, recordingSec) * Math.exp(PACE_Z * spread),
    sure: clamp((UNSURE_SPREAD - spread) / (UNSURE_SPREAD - SURE_SPREAD), 0, 1),
  };
}

const curveFraction = (curve: Curve, elapsedSec: number) =>
  curve.sure * stageFraction(elapsedSec, curve.pace) +
  (1 - curve.sure) * unsureFraction(elapsedSec, curve.pace);
const curveSlope = (curve: Curve, elapsedSec: number) =>
  curve.sure * stageSlope(elapsedSec, curve.pace) +
  (1 - curve.sure) * unsureSlope(elapsedSec, curve.pace);

/** A cubic from `p0` at speed `m0` (per unit of `s`) to 1 at rest. With
 * `m0 ÃƒÂ¢Ã¢â‚¬Â°Ã‚Â¤ 3 ÃƒÆ’Ã¢â‚¬â€ (1 ÃƒÂ¢Ã‹â€ Ã¢â‚¬â„¢ p0)` it never overshoots, so it only moves forward. */
const hermiteToEnd = (p0: number, m0: number, s: number) => {
  const s2 = s * s,
    s3 = s2 * s;
  return (
    (2 * s3 - 3 * s2 + 1) * p0 + (s3 - 2 * s2 + s) * m0 + (3 * s2 - 2 * s3)
  );
};

export interface TimingStore {
  load: () => Timings;
  save: (timings: Timings) => void;
}

export const emptyTimings = (): Timings => ({ setups: {} });

const isFit = (value: unknown): value is Fit =>
  !!value &&
  typeof value === "object" &&
  ["w", "sx", "sy", "sxx", "sxy", "n", "e2", "ew", "miss"].every((key) =>
    Number.isFinite((value as Record<string, unknown>)[key]),
  ) &&
  (value as Fit).w > 0 &&
  (value as Fit).ew > 0;

/** The key a setup's fit is stored under. A dictation the backend could not
 * fingerprint still learns, under one shared entry per stage. */
const setupKey = (stage: WorkStage, setup: string | null | undefined) =>
  `${stage}:${setup && setup.length <= 64 ? setup : "default"}`;

/** Timings persisted in the overlay webview's own storage. Anything missing or
 * malformed is simply not known yet, and starts from the prior. */
export function browserTimingStore(): TimingStore {
  const storage = () =>
    typeof localStorage === "undefined" ? null : localStorage;
  return {
    load: () => {
      try {
        storage()?.removeItem(LEGACY_STORAGE_KEY);
        const raw = storage()?.getItem(STORAGE_KEY);
        const parsed: unknown = raw ? JSON.parse(raw) : null;
        const setups =
          parsed && typeof parsed === "object"
            ? (parsed as { setups?: unknown }).setups
            : null;
        const timings = emptyTimings();
        if (setups && typeof setups === "object")
          for (const [key, fit] of Object.entries(setups))
            if (isFit(fit)) timings.setups[key] = fit;
        return timings;
      } catch {
        return emptyTimings();
      }
    },
    save: (timings) => {
      try {
        storage()?.setItem(STORAGE_KEY, JSON.stringify(timings));
      } catch {
        // Storage full or unavailable: the estimate simply stops learning.
      }
    },
  };
}

export interface BeginOptions {
  recordingSec?: number | null;
  cleanupNext?: boolean;
  /** Fingerprint of what does this stage's work. */
  setup?: string | null;
  /** For transcription followed by cleanup: the cleanup's fingerprint, so its
   * share of the bar is sized from its own history. */
  cleanupSetup?: string | null;
}

export interface WorkRun {
  /** A stage started. Within one run the fill carries on from where it is. */
  begin: (stage: WorkStage, now: number, options?: BeginOptions) => void;
  /** The work succeeded: learn from it and run the fill out. Returns how long
   * the run-out takes, or 0 if one is already under way. A completion with
   * nothing running (a streaming engine that finished straight from
   * recording) still runs out from empty, so every dictation ends the same
   * way. */
  finish: (now: number, options?: { learn?: boolean }) => number;
  /** Cancelled, failed, or replaced: forget it without learning anything. */
  abandon: () => void;
  /** The fill to draw at `now`, 0..1. Called once per frame, with time moving
   * forward; it advances the follower, so it is not a pure read. */
  value: (now: number) => number;
  running: () => boolean;
}

export function createWorkRun(store: TimingStore): WorkRun {
  let timings: Timings | null = null;
  const loaded = () => (timings ??= store.load());
  const fitFor = (stage: WorkStage, setup: string | null | undefined) =>
    loaded().setups[setupKey(stage, setup)] ?? priorFit(stage);

  let stage: WorkStage | null = null;
  let setup: string | null = null;
  let stageStart = 0;
  let from = 0;
  let to = PROGRESS_CEILING;
  let curve: Curve = { pace: 1, sure: 0 };
  let recordingSec: number | null = null;
  /** Stages already finished in this run, waiting to be learned on success. */
  let done: { stage: WorkStage; setup: string | null; seconds: number }[] = [];
  /** The fill as drawn, and its speed per second, as of `lastAt`. */
  let position = 0;
  let speed = 0;
  let lastAt = 0;
  let sealStart: number | null = null;
  let sealFrom = 0;
  /** The run-out's starting speed, per run-out duration. */
  let sealSlope = 0;
  let sealMs = SEAL_MS;

  /** Where the curve says the fill should be. */
  const target = (now: number) =>
    from + (to - from) * curveFraction(curve, (now - stageStart) / 1000);
  /** How fast the curve is moving there, per second. */
  const targetSpeed = (now: number) =>
    (to - from) * curveSlope(curve, (now - stageStart) / 1000);

  /** Move the drawn fill toward the curve up to `now`, in steps of at most a
   * 60 Hz frame (coarser over a long gap, which only a test produces). Each
   * step is the exact critically damped response of the fill's distance from
   * the curve *and its speed relative to the curve's*, so it rides on the
   * curve rather than chasing it: a curve moving at a steady pace is followed
   * with no lag, and only a change of pace (the hand-off to cleanup) is
   * rounded off. Damping the fill's absolute speed instead trailed a moving
   * curve by 2/ÃƒÂÃ¢â‚¬Â° ÃƒÂ¢Ã¢â‚¬Â°Ã‹â€  0.17s, more than half of a quick local dictation's wait.
   * Forward only, and never past the ceiling until the run-out. */
  const advance = (now: number) => {
    if (stage === null || !(now > lastAt)) return;
    const gap = (now - lastAt) / 1000;
    const steps = Math.max(1, Math.ceil(Math.min(gap * 60, 4000)));
    const dt = gap / steps;
    for (let i = 0; i < steps; i++) {
      const at = lastAt + i * dt * 1000;
      const goal = target(at);
      const pace = targetSpeed(at);
      const offset = position - goal;
      const relative = speed - pace;
      const momentum = relative + FOLLOW_OMEGA * offset;
      const decay = Math.exp(-FOLLOW_OMEGA * dt);
      const next = goal + pace * dt + (offset + momentum * dt) * decay;
      speed = pace + (relative - FOLLOW_OMEGA * momentum * dt) * decay;
      if (next > position) position = Math.min(PROGRESS_CEILING, next);
      else speed = Math.max(0, speed);
    }
    lastAt = now;
  };

  const value = (now: number) => {
    if (sealStart !== null) {
      const s = clamp((now - sealStart) / sealMs, 0, 1);
      return s >= 1 ? 1 : hermiteToEnd(sealFrom, sealSlope, s);
    }
    if (stage === null) return 0;
    advance(now);
    return position;
  };

  const reset = () => {
    stage = null;
    setup = null;
    sealStart = null;
    sealFrom = 0;
    sealSlope = 0;
    sealMs = SEAL_MS;
    position = 0;
    speed = 0;
    done = [];
    recordingSec = null;
  };

  /** Fold this run's stages into their setups, most recently used last, and
   * forget the setups used longest ago. */
  const learnAll = () => {
    const setups = { ...loaded().setups };
    for (const sample of done) {
      const key = setupKey(sample.stage, sample.setup);
      const fit = learn(
        setups[key] ?? priorFit(sample.stage),
        recordingSec ?? TYPICAL_RECORDING_S,
        sample.seconds,
      );
      delete setups[key];
      setups[key] = fit;
    }
    const keys = Object.keys(setups);
    for (const key of keys.slice(0, Math.max(0, keys.length - MAX_SETUPS)))
      delete setups[key];
    timings = { setups };
    store.save(timings);
  };

  return {
    begin: (next, now, options = {}) => {
      const carrying = stage !== null && sealStart === null;
      if (carrying && stage !== null) {
        advance(now);
        done.push({ stage, setup, seconds: (now - stageStart) / 1000 });
      }
      // The new segment picks up the curve where it is (or the drawn fill, if
      // that has run slightly ahead), so the curve stays continuous and the
      // follower only has to bend, never catch up.
      const start = carrying ? Math.max(target(now), position) : HEAD_START;
      if (!carrying) reset();
      if (options.recordingSec !== undefined)
        recordingSec = options.recordingSec;
      stage = next;
      setup = options.setup ?? null;
      stageStart = now;
      from = start;
      curve = curveFor(fitFor(next, setup), recordingSec);
      if (next === "transcribing" && options.cleanupNext) {
        const cleanup = curveFor(
          fitFor("processing", options.cleanupSetup),
          recordingSec,
        );
        to =
          start +
          (PROGRESS_CEILING - start) *
            (curve.pace / (curve.pace + cleanup.pace));
      } else {
        to = PROGRESS_CEILING;
      }
      if (!carrying) {
        // Already moving at the curve's own pace: no ease-in from a standstill.
        position = start;
        speed = targetSpeed(now);
        lastAt = now;
      }
    },
    finish: (now, options = {}) => {
      if (sealStart !== null) return 0;
      if (stage === null) {
        sealFrom = 0;
        sealSlope = 0;
        sealMs = SEAL_MS;
        sealStart = now;
        return sealMs;
      }
      advance(now);
      done.push({ stage, setup, seconds: (now - stageStart) / 1000 });
      // A recording of unknown length would teach the fit a guess, and a
      // dictation that ended in a notice (a cleanup that timed out and fell
      // back) is not what this setup usually costs.
      if (recordingSec !== null && options.learn !== false) learnAll();
      done = [];
      sealFrom = position;
      sealMs = sealDuration(1 - position);
      // Leave at the speed it was already going, capped so it cannot overshoot.
      sealSlope = Math.min(
        Math.max(0, speed) * (sealMs / 1000),
        3 * (1 - position),
      );
      sealStart = now;
      return sealMs;
    },
    abandon: reset,
    value,
    running: () => stage !== null && sealStart === null,
  };
}
