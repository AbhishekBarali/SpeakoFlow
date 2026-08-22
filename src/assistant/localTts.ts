/**
 * Policy for the *local* (in-WebView) text-to-speech engine.
 *
 * Kokoro runs inside the assistant WebView: ~310 MB of fp32 ONNX weights, the
 * onnxruntime-web WASM module, and — on WebGPU — a second copy of the weights as
 * GPU buffers in WebView2's shared GPU process. The assistant window is created
 * at launch and only ever hidden, so anything it loads stays resident for the
 * lifetime of the app. These rules decide when that cost is actually worth
 * paying, and they live here (rather than inline in each component) because the
 * panel and the Settings page previously answered the same question differently:
 * Settings correctly checked the selected engine, the panel did not, so a user on
 * a remote engine still paid for a local model that could never be asked to
 * speak.
 */

/** The one engine that synthesizes inside the WebView. Every other engine
 *  (OpenAI, ElevenLabs, Azure, OpenRouter, …) is spoken by Rust, and the backend
 *  only emits the `assistant-tts-*` events this hook listens for when the engine
 *  is this one. */
export const LOCAL_TTS_ENGINE = "kokoro";

/** Idle window before a loaded model is released while the panel is hidden.
 *  Dismissing the panel is a strong "done for now" signal, so the memory goes
 *  back quickly. */
export const IDLE_UNLOAD_HIDDEN_MS = 60_000;

/** Idle window while the panel is on screen. Much longer: the user is plausibly
 *  mid-conversation, and reloading costs a few seconds of silence before the
 *  next reply. */
export const IDLE_UNLOAD_VISIBLE_MS = 10 * 60_000;

/** Is spoken output produced inside this WebView? Settings written before the
 *  engine field existed have no value, and Kokoro was the only engine then, so
 *  an absent engine means local. */
export function usesLocalTtsEngine(engine: string | null | undefined): boolean {
  return (engine ?? LOCAL_TTS_ENGINE) === LOCAL_TTS_ENGINE;
}

/** Should the local model be mounted at all? Only when spoken replies are on
 *  *and* they are produced locally. */
export function localTtsActive(
  ttsEnabled: boolean,
  engine: string | null | undefined,
): boolean {
  return ttsEnabled && usesLocalTtsEngine(engine);
}

/** Everything that would be cut off by unloading the model. */
export interface SpeechProgress {
  /** A reply is still open: more text may arrive for the same utterance. */
  streamOpen: boolean;
  /** Synthesis for the current reply has finished producing audio. */
  synthDone: boolean;
  /** Clips synthesized but not yet handed to an audio sink. */
  queued: number;
  /** An `<audio>` element is playing (browser sink). */
  elementPlaying: boolean;
  /** A chunk is in flight to / playing on the native sink (Tauri sink). */
  nativePlaying: boolean;
}

/** True while unloading would interrupt speech. Note that an empty queue is not
 *  idle on its own: during a streaming reply the queue routinely drains between
 *  sentences while the model is still synthesizing the next one. */
export function isSpeechInFlight(progress: SpeechProgress): boolean {
  return (
    progress.streamOpen ||
    !progress.synthDone ||
    progress.queued > 0 ||
    progress.elementPlaying ||
    progress.nativePlaying
  );
}

/** How long an idle model may stay loaded, given whether the panel is visible. */
export function idleUnloadDelayMs(panelVisible: boolean): number {
  return panelVisible ? IDLE_UNLOAD_VISIBLE_MS : IDLE_UNLOAD_HIDDEN_MS;
}

/** What an expired idle-unload timer should do:
 *  - `release`  — drop the ONNX session and its GPU buffers now.
 *  - `wait`     — something is in flight; check again after another window.
 *  - `nothing`  — no model is loaded, so there is nothing to give back. */
export type IdleUnloadDecision = "release" | "wait" | "nothing";

/** State an idle-unload timer must weigh before dropping the weights.
 *  `loadInFlight` means a load is *running* — not that one has ever run. Reading
 *  it off the memoized load promise (which is kept after it resolves) is what
 *  made the timer defer forever, so the model was never actually released. */
export interface IdleUnloadState {
  speechInFlight: boolean;
  loadInFlight: boolean;
  modelLoaded: boolean;
}

export function idleUnloadDecision(state: IdleUnloadState): IdleUnloadDecision {
  if (state.speechInFlight || state.loadInFlight) return "wait";
  return state.modelLoaded ? "release" : "nothing";
}
