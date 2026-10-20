import type { ModelInfo } from "@/bindings";

/**
 * Moving off a retired speech model: one rule for every place that offers it.
 *
 * A retired model is one whose engine SpeakoFlow no longer includes (the ONNX
 * models; see `managers/legacy_models.rs`). Its files stay on disk and it can
 * stay the saved selection after an upgrade, but it never runs. Three
 * surfaces offer its new version (Home's prompt, the prompt at the top of the
 * Models page's speech tab, and the retired rows and notice in the catalog),
 * and they have to agree on what the button says and what pressing it does,
 * so the decisions live here as pure functions and `useReplaceRetired` only
 * wires them to the model store.
 */

/** Where moving to a retired model's new version stands. */
export type ReplacementPhase =
  /** Not on this computer: the action downloads it. */
  | "download"
  /** On this computer: the action switches to it. */
  | "use"
  | "downloading"
  | "verifying"
  | "extracting"
  /** Being made the active model. */
  | "switching"
  /** The last download failed: the action retries it. */
  | "failed";

export interface ReplacementView {
  /** The catalog model that replaces the retired one. */
  model: ModelInfo;
  phase: ReplacementPhase;
  /** 0 to 100 while downloading, once the first progress event arrived. */
  percent?: number;
  /** MB/s while downloading. */
  speed?: number;
  /** Why the last download failed, for `failed`. */
  error?: string;
}

/** The model store's download bookkeeping, as the view reads it. */
export interface DownloadSnapshot {
  downloading: Readonly<Record<string, true>>;
  verifying: Readonly<Record<string, true>>;
  extracting: Readonly<Record<string, true>>;
  progress: Readonly<Record<string, { percentage: number } | undefined>>;
  stats: Readonly<Record<string, { speed: number } | undefined>>;
  errors: Readonly<Record<string, string | undefined>>;
}

/** What pressing the action does. */
export type ReplacementPlan =
  | { kind: "select"; id: string }
  /** `selectWhenDone`: the retired model is the selected one, so its new
   *  version takes its place the moment it lands, wherever the user is. */
  | { kind: "download"; id: string; selectWhenDone: boolean };

/** Where `retired`'s new version stands, or null when it has none listed. */
export function replacementView(
  retired: ModelInfo,
  modelsById: ReadonlyMap<string, ModelInfo>,
  snapshot: DownloadSnapshot,
  switchingId: string | null,
): ReplacementView | null {
  const id = retired.replacement_id;
  const model = id ? modelsById.get(id) : undefined;
  if (!id || !model) return null;
  // The busy states first: a download in flight is what the user is waiting
  // on, whatever an earlier attempt's error said.
  if (id in snapshot.extracting) return { model, phase: "extracting" };
  if (id in snapshot.verifying) return { model, phase: "verifying" };
  if (id in snapshot.downloading) {
    return {
      model,
      phase: "downloading",
      percent: snapshot.progress[id]?.percentage,
      speed: snapshot.stats[id]?.speed,
    };
  }
  if (switchingId === id) return { model, phase: "switching" };
  if (model.is_downloaded) return { model, phase: "use" };
  const error = snapshot.errors[id];
  if (error) return { model, phase: "failed", error };
  return { model, phase: "download" };
}

/** What the action does for `retired`, or null when there is nothing to do. */
export function replacementPlan(
  retired: ModelInfo,
  modelsById: ReadonlyMap<string, ModelInfo>,
  currentModel: string,
): ReplacementPlan | null {
  const id = retired.replacement_id;
  const replacement = id ? modelsById.get(id) : undefined;
  if (!id || !replacement) return null;
  if (replacement.is_downloaded) return { kind: "select", id };
  return { kind: "download", id, selectWhenDone: retired.id === currentModel };
}

/** Whether a phase is a download in flight (no action, a progress bar). */
export const isReplacementBusy = (phase: ReplacementPhase): boolean =>
  phase === "downloading" || phase === "verifying" || phase === "extracting";
