import { useCallback, useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  idleUnloadDecision,
  idleUnloadDelayMs,
  isSpeechInFlight,
} from "./localTts";
import {
  audioLooksBroken,
  clearGpuBroken,
  gpuMarkedBroken,
  markGpuBroken,
  reportWebGpu,
  type KokoroDevice,
} from "./localVoice";

export type TtsStatus = "off" | "loading" | "ready" | "speaking" | "error";

/** Why local (Kokoro) speech failed, so the panel can show a precise, useful
 *  message instead of going silent:
 *  - `load`      — the model couldn't download / initialize.
 *  - `synthesis` — the model loaded but couldn't turn text into audio.
 *  - `blocked`   — the system blocked auto-play (needs a user gesture); the
 *                  clip is kept queued so a later click can replay it.
 *  - `playback`  — the audio element failed to play (output device issue).
 *  - `gpu`       — the graphics card produced audio that is not speech; it was
 *                  muted and the graphics card is no longer used for the voice. */
export type KokoroErrorReason =
  | "load"
  | "synthesis"
  | "blocked"
  | "playback"
  | "gpu";

/** How a spoken reply ended: synthesized in full, stopped or superseded, muted
 *  because the graphics card produced broken audio (the card is no longer used
 *  for the voice), withheld because a clip looked broken and nothing after it
 *  proved the card fine, or failed outright. */
export type SpeakOutcome = "done" | "stopped" | "gpu" | "muted" | "failed";
export interface KokoroError {
  reason: KokoroErrorReason;
}

export interface BrowserSpeechSink {
  enqueue: (blob: Blob, epoch: number | null) => Promise<void>;
  finish: (epoch: number | null) => void;
}

/** Minimal surface of the kokoro-js model we use (erases its strict voice
 *  union type so the voice id can come from settings). */
interface KokoroModel {
  stream(
    splitter: TextSplitter,
    options: { voice?: string; speed?: number },
  ): AsyncIterable<{ text: string; audio: KokoroAudio }>;
}

/** kokoro-js's RawAudio: a WAV encoder over the raw samples it also exposes. */
interface KokoroAudio {
  toBlob(): Blob;
  audio?: Float32Array;
  sampling_rate?: number;
}

interface TextSplitter {
  push(text: string): void;
  close(): void;
}

const KOKORO_MODEL_ID = "onnx-community/Kokoro-82M-v1.0-ONNX";

/** Carries the reply's cancellation epoch alongside a raw audio body, since the
 *  body itself is taken up by the audio. Must match `TTS_EPOCH_HEADER` in
 *  `src-tauri/src/commands/assistant.rs`. */
const TTS_EPOCH_HEADER = "x-tts-epoch";

interface ProgressEvent {
  status: string;
  file?: string;
  progress?: number;
}

function hasWebGpu(): boolean {
  return typeof navigator !== "undefined" && "gpu" in navigator;
}

/** A load that finished after its model was dropped; it was released, and a
 *  caller waiting on it simply stops. */
class SupersededLoad extends Error {
  constructor() {
    super("The voice was switched while it was loading.");
  }
}

/** Best-effort release of the kokoro-js model's ONNX session + WebGPU buffers.
 *  kokoro-js wraps a transformers.js model whose `.dispose()` frees the
 *  onnxruntime InferenceSession(s) (hundreds of MB, plus GPU buffers). The
 *  exact shape isn't in our minimal type, so probe the known locations and
 *  swallow errors — nulling the ref then lets GC reclaim the rest. Without
 *  this, changing precision / disabling TTS / closing the panel orphaned a
 *  full model in the WebView, a major contributor to the memory growth. */
async function disposeModel(model: KokoroModel | null): Promise<void> {
  if (!model) return;
  try {
    const anyModel = model as unknown as {
      dispose?: () => unknown;
      model?: { dispose?: () => unknown };
    };
    if (typeof anyModel.dispose === "function") {
      await anyModel.dispose();
    } else if (typeof anyModel.model?.dispose === "function") {
      await anyModel.model.dispose();
    }
  } catch {
    // best-effort; the GC reclaims the rest once the ref is dropped
  }
}

/**
 * Local TTS via kokoro-js. Prefers WebGPU (fp32, ~10x faster than wasm on a
 * discrete GPU) with wasm/q8 fallback. Sentences are synthesized as a stream
 * and queued for gapless playback, so the first words play almost instantly
 * instead of waiting for the whole clip.
 */
export function useKokoroTts(
  enabled: boolean,
  voice: string,
  dtype: string = "fp32",
  speed: number = 1,
  preload: boolean = true,
  browserSink?: BrowserSpeechSink,
  /** Where Kokoro may run in this WebView: "cpu" keeps it off WebGPU entirely,
   *  "gpu" always tries WebGPU, "auto" tries it unless this machine's graphics
   *  card was caught garbling the voice before. */
  device: KokoroDevice = "auto",
) {
  const browserSinkRef = useRef(browserSink);
  browserSinkRef.current = browserSink;
  const modelRef = useRef<KokoroModel | null>(null);
  const loadingRef = useRef<Promise<KokoroModel> | null>(null);
  const dtypeRef = useRef(dtype);
  const deviceRef = useRef(device);
  /** The loaded model runs on WebGPU, so its first clips are checked. */
  const loadedOnGpuRef = useRef(false);
  /** Clean clips still to see before WebGPU counts as proven on this load. */
  const gpuChecksLeftRef = useRef(0);
  /** Broken clips since the last clean one. One can be a false alarm (it is
   *  muted and the next clip decides); two mean the card garbles the voice. */
  const gpuStrikesRef = useRef(0);
  /** Bumped whenever the model is dropped, so a load that finishes after being
   *  dropped releases what it loaded instead of pinning it. */
  const loadEpochRef = useRef(0);
  // Latest speaking speed, read when a reply starts streaming so a change
  // applies to the next reply without re-creating the playback callbacks.
  const speedRef = useRef(speed);
  speedRef.current = speed;
  const [status, setStatus] = useState<TtsStatus>("off");
  /** Model download progress 0-100 while status === "loading". */
  const [progress, setProgress] = useState(0);
  /** Last failure reason, or null when healthy. Surfaced to the panel so a
   *  failure is explained rather than silent. */
  const [error, setError] = useState<KokoroError | null>(null);

  // Playback queue state (refs: updated from async generators)
  const queueRef = useRef<Blob[]>([]);
  const playingRef = useRef<HTMLAudioElement | null>(null);
  // Tauri desktop builds send Kokoro's WAV chunks to Rust/rodio. This remains
  // true while an awaited native chunk is playing so synthesis cannot start a
  // second pump in parallel.
  const nativePlayingRef = useRef(false);
  /** This hook instance actually handed audio to the native sink, so its
   *  teardown owns stopping it.
   *
   *  Settings mounts this hook too, purely for the Kokoro setup/test buttons,
   *  and `assistant_stop_local_tts` bumps the *shared* playback epoch — which
   *  silences remote engines as well. Combined with StrictMode's
   *  mount→cleanup→mount, that meant simply opening Settings → Assistant cut off
   *  whatever the assistant was saying. A hook that never spoke must not stop
   *  speech it doesn't own. */
  const usedNativeSinkRef = useRef(false);
  const generationRef = useRef(0);

  /** Open splitter for a reply that is still being written, if any. Text is fed
   *  in as the model produces it and the splitter stays open until the reply
   *  ends, so Kokoro sees one continuous utterance instead of isolated clips. */
  const streamRef = useRef<{
    splitter: TextSplitter;
    generation: number;
  } | null>(null);
  /** Text that arrived before the model finished loading. The first reply of a
   *  session can begin while weights are still initializing; buffering here
   *  means those words are spoken late rather than lost. */
  const pendingTextRef = useRef<string[]>([]);
  /** A reply is open: text may still arrive. */
  const streamOpenRef = useRef(false);
  /** The reply ended while the model was still loading, so the splitter must be
   *  closed as soon as it exists. */
  const closeWhenReadyRef = useRef(false);
  /** No more audio will be produced for the current reply. Together with an
   *  empty queue this means the native sink can drain and hand the audio device
   *  back; while it is false, an empty queue only means synthesis is behind. */
  const synthDoneRef = useRef(true);
  /** Cancellation epoch of the reply being streamed, handed out by the backend.
   *  Tagging each chunk with it means a clip still crossing the IPC boundary when
   *  the user hits Stop is dropped rather than played over the next reply. `null`
   *  for the one-shot replay path, which has no reply of its own. */
  const streamEpochRef = useRef<number | null>(null);

  /** Pending idle-unload timer, if any. */
  const idleTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** A load is genuinely running right now.
   *
   *  Deliberately not `loadingRef`: that holds the memoized load *promise* and
   *  keeps holding it after the promise resolves (it is only cleared on
   *  failure), so it says "loaded", not "loading". Using it as the idle-unload
   *  guard meant the timer rescheduled itself forever and the weights were
   *  never released. */
  const loadInFlightRef = useRef(false);
  /** Bumped by every `from_pretrained` attempt, including the processor
   *  fallback inside one load. Only the newest attempt may move the progress
   *  bar: a dropped download is not aborted (transformers.js fetches without a
   *  signal) and keeps reporting until its file is in, so with an ungated
   *  callback two downloads wrote the same state in turn and the bar swung
   *  between their percentages on every chunk. */
  const attemptRef = useRef(0);
  const cancelIdleUnload = useCallback(() => {
    if (idleTimerRef.current !== null) {
      clearTimeout(idleTimerRef.current);
      idleTimerRef.current = null;
    }
  }, []);

  /** True while anything would be cut off by unloading: a reply still being fed
   *  in, synthesis in flight, or audio queued/playing. */
  const isBusy = useCallback(
    () =>
      isSpeechInFlight({
        streamOpen: streamOpenRef.current,
        synthDone: synthDoneRef.current,
        queued: queueRef.current.length,
        elementPlaying: playingRef.current !== null,
        nativePlaying: nativePlayingRef.current,
      }),
    [],
  );

  const scheduleRef = useRef<(delayMs: number) => void>(() => {});
  const scheduleIdleUnload = useCallback(
    (delayMs: number) => {
      cancelIdleUnload();
      idleTimerRef.current = setTimeout(() => {
        idleTimerRef.current = null;
        const decision = idleUnloadDecision({
          speechInFlight: isBusy(),
          loadInFlight: loadInFlightRef.current,
          modelLoaded: modelRef.current !== null,
        });
        // Still working, or a load is in flight: try again later rather than
        // pulling the session out from under it.
        if (decision === "wait") {
          scheduleRef.current(delayMs);
          return;
        }
        if (decision === "nothing") return;
        void disposeModel(modelRef.current);
        modelRef.current = null;
        loadingRef.current = null;
        // Back to "not loaded". The next reply calls `ensureLoaded` itself, so
        // this only costs load time, never speech.
        setStatus((s) => (s === "loading" || s === "speaking" ? s : "off"));
      }, delayMs);
    },
    [cancelIdleUnload, isBusy],
  );
  scheduleRef.current = scheduleIdleUnload;

  const ensureLoaded = useCallback(async (): Promise<KokoroModel> => {
    cancelIdleUnload();
    if (modelRef.current) return modelRef.current;
    if (!loadingRef.current) {
      setError(null);
      setStatus("loading");
      setProgress(0);
      loadInFlightRef.current = true;
      const loadEpoch = loadEpochRef.current;
      loadingRef.current = (async () => {
        const { KokoroTTS } = await import("kokoro-js");
        const requestedDtype = dtypeRef.current;
        // A "-cpu" suffix is an explicit opt-out of WebGPU. Some Chromium and
        // GPU-driver combinations initialize WebGPU successfully and then emit
        // garbled audio instead of speech. That throws no error, so the catch
        // below never fires and auto-detection cannot see it. This is the
        // manual escape hatch for those machines.
        const forceCpu = requestedDtype.endsWith("-cpu");
        const baseDtype = forceCpu
          ? requestedDtype.slice(0, -"-cpu".length)
          : requestedDtype;
        // A quantized graph on the WebGPU execution provider is a KNOWN broken
        // combination, not a hardware lottery: onnxruntime#29807 reports q8 and
        // q4f16 producing unintelligible noise for Kokoro-82M on WebGPU while
        // the same files are clean on the WASM provider. That is the bug behind
        // the reports of "garbled sound instead of speech" in Chrome/Edge/Brave
        // (and it matches Firefox being fine — no WebGPU there, so it silently
        // ran the correct path). Corruption produces valid audio buffers and no
        // exception, so nothing downstream can detect it.
        //
        // So a quantized precision always runs on the CPU. It costs speed, not
        // correctness, and q8 on wasm is the configuration upstream treats as
        // known-good. fp32 (the default) still uses WebGPU; fp16 is left alone
        // because it is only reported broken on specific GPUs, and the "-cpu"
        // suffix stays as the manual escape hatch for those.
        const quantizedOnGpu = ["q8", "q4", "q4f16"].includes(baseDtype);
        // The device setting and this machine's history decide whether WebGPU
        // is tried at all: "cpu" never, "gpu" always, and Automatic unless the
        // graphics card was caught producing broken audio (remembered across
        // launches, because it is a property of the GPU and its driver).
        const device = deviceRef.current;
        const gpuAllowed =
          device === "gpu" || (device === "auto" && !gpuMarkedBroken());
        const useGpu =
          hasWebGpu() && gpuAllowed && !forceCpu && !quantizedOnGpu;
        if (!hasWebGpu()) reportWebGpu(false);
        // WebKitGTK commonly has no WebGPU. Loading the 325 MB fp32 graph into
        // WASM can appear to hang after the text answer is already visible;
        // use the cached 92 MB q8 graph directly on CPU instead of waiting for
        // fp32 initialization to fail first.
        const chosenDtype =
          !useGpu && (baseDtype === "fp32" || baseDtype === "fp16")
            ? "q8"
            : baseDtype;
        // Track download progress of the (largest) onnx weights file. Each
        // attempt gets its own callback, ignored once a newer attempt or a drop
        // has superseded it, and it only moves forward.
        const makeProgressCallback = () => {
          const attempt = ++attemptRef.current;
          let shown = 0;
          return (event: ProgressEvent) => {
            if (
              attempt !== attemptRef.current ||
              loadEpoch !== loadEpochRef.current
            ) {
              return;
            }
            if (
              event.status === "progress" &&
              event.file?.endsWith(".onnx") &&
              typeof event.progress === "number"
            ) {
              const next = Math.min(100, Math.round(event.progress));
              if (next > shown) {
                shown = next;
                setProgress(next);
              }
            }
          };
        };
        type LoadOptions = Parameters<typeof KokoroTTS.from_pretrained>[1];
        let model: unknown;
        let onGpu = useGpu;
        try {
          model = await KokoroTTS.from_pretrained(KOKORO_MODEL_ID, {
            dtype: chosenDtype,
            device: useGpu ? "webgpu" : "wasm",
            progress_callback: makeProgressCallback(),
          } as unknown as LoadOptions);
          console.info(
            `[Kokoro TTS] loaded on ${useGpu ? "webgpu" : "wasm/CPU"} (${chosenDtype})`,
          );
        } catch (gpuErr) {
          // WebGPU init can fail (driver/feature limits). Fall back to wasm.
          // fp32/fp16 are too heavy for CPU, so drop to q8 there.
          onGpu = false;
          const fallbackDtype =
            chosenDtype === "fp32" || chosenDtype === "fp16"
              ? "q8"
              : chosenDtype;
          if (useGpu) {
            console.warn(
              "[Kokoro TTS] WebGPU init failed — falling back to wasm/CPU, " +
                "which is much slower. Synthesis will not use the GPU:",
              gpuErr,
            );
            // Automatic can move the voice to the processor engine instead,
            // and remembers it: a card that cannot start the session fails
            // the same way next launch.
            if (deviceRef.current === "auto") markGpuBroken();
            reportWebGpu(false);
          }
          // The fallback is a different file, so the bar starts over once,
          // deliberately; the abandoned attempt can no longer move it.
          if (loadEpoch === loadEpochRef.current) setProgress(0);
          model = await KokoroTTS.from_pretrained(KOKORO_MODEL_ID, {
            dtype: fallbackDtype,
            device: "wasm",
            progress_callback: makeProgressCallback(),
          } as unknown as LoadOptions);
          console.info(
            `[Kokoro TTS] loaded on wasm/CPU (${fallbackDtype}) fallback`,
          );
        }
        // Dropped while loading (turned off, or another precision or device
        // was picked): release it rather than pin memory nothing will free.
        if (loadEpoch !== loadEpochRef.current) {
          void disposeModel(model as KokoroModel);
          throw new SupersededLoad();
        }
        // A WebGPU session's first clips are checked for broken audio before
        // they reach a speaker (see `consume`).
        loadedOnGpuRef.current = onGpu;
        gpuChecksLeftRef.current = onGpu ? 2 : 0;
        modelRef.current = model as KokoroModel;
        loadInFlightRef.current = false;
        setStatus("ready");
        return modelRef.current;
      })().catch((e: unknown) => {
        if (e instanceof SupersededLoad) {
          // Whatever dropped it already reset the state; only a load that
          // nothing replaced leaves the flags to tidy.
          if (loadingRef.current === null) {
            loadInFlightRef.current = false;
            setStatus((s) => (s === "loading" ? "off" : s));
          }
          throw e;
        }
        loadingRef.current = null;
        loadInFlightRef.current = false;
        setStatus("error");
        setError({ reason: "load" });
        throw e;
      });
    }
    return loadingRef.current;
  }, [cancelIdleUnload]);

  // Callers choose whether passive mounting should prepare the model. Settings
  // disables this and exposes an explicit setup action; the assistant panel
  // passes its own window visibility, so a panel nobody has opened doesn't pay
  // for weights it may never use.
  useEffect(() => {
    if (enabled && preload) {
      ensureLoaded().catch(() => {});
    } else if (!enabled) {
      setError(null);
      // "loading" too: the load is dropped below, and a status left on
      // "loading" kept the progress bar up for a download nothing would finish,
      // so the next Test or Download started a second one beside it.
      setStatus((s) =>
        s === "speaking" || s === "ready" || s === "loading" ? "off" : s,
      );
      setProgress(0);
      // Turned off: free the model so its ONNX/WebGPU memory isn't pinned for
      // the WebView's lifetime. It reloads on demand if re-enabled.
      cancelIdleUnload();
      loadEpochRef.current += 1;
      void disposeModel(modelRef.current);
      modelRef.current = null;
      loadingRef.current = null;
    }
  }, [enabled, preload, ensureLoaded, cancelIdleUnload]);

  // Give the weights back when they stop being used. Without this, one spoken
  // reply (or one preload) kept the ONNX session and its GPU buffers resident
  // for as long as the app ran, since this window is only ever hidden.
  useEffect(() => {
    if (!enabled || status !== "ready") {
      cancelIdleUnload();
      return;
    }
    scheduleIdleUnload(idleUnloadDelayMs(preload));
    return cancelIdleUnload;
  }, [enabled, preload, status, scheduleIdleUnload, cancelIdleUnload]);

  /**
   * Abandon whatever this hook is currently synthesizing or playing.
   *
   * `cancelNative` decides whether the native side is cancelled too. Cancelling
   * bumps the shared playback epoch, which is right for a real Stop but wrong
   * when starting a new reply: the backend has already issued that reply's epoch,
   * and bumping it here would invalidate the very chunks about to be sent.
   */
  const teardown = useCallback((cancelNative: boolean) => {
    generationRef.current += 1; // invalidate in-flight generation
    queueRef.current = [];
    const el = playingRef.current;
    nativePlayingRef.current = false;
    // Abandon any reply that is still being streamed in. Closing the splitter
    // lets kokoro-js finish its generator instead of leaving it suspended; the
    // bumped generation makes every clip it still produces a no-op.
    const open = streamRef.current;
    streamRef.current = null;
    streamOpenRef.current = false;
    closeWhenReadyRef.current = false;
    pendingTextRef.current = [];
    synthDoneRef.current = true;
    streamEpochRef.current = null;
    if (open) {
      try {
        open.splitter.close();
      } catch {
        // already closed
      }
    }
    if (cancelNative && isTauri()) {
      // Native playback polls the same cancellation epoch and stops within one
      // 50 ms tick. Fire-and-forget here so React cleanup stays synchronous.
      void invoke("assistant_stop_local_tts").catch(() => {});
    }
    if (el) {
      el.pause();
      // Revoke the in-flight clip's object URL. onended/onerror (which normally
      // revoke) don't fire on pause(), so without this every interrupted or
      // restarted spoken reply leaked a blob URL.
      if (el.src) {
        try {
          URL.revokeObjectURL(el.src);
        } catch {
          // ignore
        }
        el.removeAttribute("src");
      }
      playingRef.current = null;
    }
    setError(null);
    setStatus((s) => (s === "speaking" ? "ready" : s));
  }, []);

  const stop = useCallback(
    (cancelNative = true) => teardown(cancelNative && !browserSinkRef.current),
    [teardown],
  );

  // When the dtype (precision) or the device changes, drop the cached model so
  // the next synthesis reloads with the new choice.
  useEffect(() => {
    if (dtypeRef.current === dtype && deviceRef.current === device) return;
    dtypeRef.current = dtype;
    deviceRef.current = device;
    stop();
    // Release the old session before dropping the ref, or its ONNX/WebGPU
    // memory leaks on every change.
    loadEpochRef.current += 1;
    void disposeModel(modelRef.current);
    modelRef.current = null;
    loadingRef.current = null;
    setStatus("off");
    if (enabled && preload) {
      ensureLoaded().catch(() => {});
    }
  }, [dtype, device, enabled, preload, ensureLoaded, stop]);

  /**
   * The graphics card returned audio that is not speech. Nothing of it reaches
   * a speaker: this reply stops (its remaining clips would be the same), the
   * session is released, and Rust is told, so Automatic speaks the next reply on
   * the processor. Under Automatic the verdict is also remembered, so the next
   * launch does not try the graphics card again.
   */
  const handleGpuBroken = useCallback(() => {
    console.warn(
      "[Kokoro TTS] WebGPU produced audio that is not speech; muting it and " +
        "not using the graphics card for the voice",
    );
    if (deviceRef.current === "auto") markGpuBroken();
    reportWebGpu(false);
    // A call plays through the browser sink and waits for its end; the reply
    // it was waiting for is over.
    browserSinkRef.current?.finish(streamEpochRef.current);
    const model = modelRef.current;
    loadEpochRef.current += 1;
    modelRef.current = null;
    loadingRef.current = null;
    loadedOnGpuRef.current = false;
    gpuChecksLeftRef.current = 0;
    stop();
    void disposeModel(model);
    // After `stop()`, which clears the error.
    setError({ reason: "gpu" });
    setStatus("error");
  }, [stop]);

  // Release the model + audio when the hook unmounts (e.g. the panel window is
  // torn down). The ONNX session and its WebGPU buffers are hundreds of MB;
  // without this they leak for the lifetime of the WebView.
  useEffect(() => {
    return () => {
      generationRef.current += 1;
      queueRef.current = [];
      if (idleTimerRef.current !== null) {
        clearTimeout(idleTimerRef.current);
        idleTimerRef.current = null;
      }
      const el = playingRef.current;
      nativePlayingRef.current = false;
      // Only stop speech this hook actually started (see usedNativeSinkRef):
      // the Settings page mounts one that must never silence a live reply.
      if (isTauri() && usedNativeSinkRef.current) {
        void invoke("assistant_stop_local_tts").catch(() => {});
      }
      if (el) {
        el.pause();
        if (el.src) {
          try {
            URL.revokeObjectURL(el.src);
          } catch {
            // ignore
          }
        }
        playingRef.current = null;
      }
      loadEpochRef.current += 1;
      void disposeModel(modelRef.current);
      modelRef.current = null;
      loadingRef.current = null;
    };
  }, []);

  /** Play queued blobs back-to-back; exits when queue drains. */
  const pump = useCallback((generation: number) => {
    if (generation !== generationRef.current) return;
    const next = queueRef.current.shift();
    if (!next) {
      playingRef.current = null;
      // Everything synthesized has been handed over. Telling the native sink the
      // reply is complete lets it play out its queue and then release the audio
      // device; while synthesis is still running an empty queue only means we are
      // ahead, so the sink is left open to keep the next sentence gapless.
      if (browserSinkRef.current && synthDoneRef.current) {
        browserSinkRef.current.finish(streamEpochRef.current);
      } else if (isTauri() && synthDoneRef.current) {
        void invoke("assistant_finish_local_tts", {
          epoch: streamEpochRef.current,
        }).catch(() => {});
      }
      setStatus((s) => (s === "speaking" ? "ready" : s));
      return;
    }
    const url = URL.createObjectURL(next);

    if (browserSinkRef.current) {
      URL.revokeObjectURL(url);
      nativePlayingRef.current = true;
      void browserSinkRef.current
        .enqueue(next, streamEpochRef.current)
        .catch(() => {
          if (generation === generationRef.current)
            setError({ reason: "playback" });
        })
        .finally(() => {
          if (generation !== generationRef.current) return;
          nativePlayingRef.current = false;
          pump(generation);
        });
      return;
    }

    if (isTauri()) {
      // The HUD is deliberately non-focus-stealing. Route generated speech to
      // the native audio backend so Linux WebKit cannot suppress it as
      // background autoplay and so the configured output device is respected.
      // The call returns once the clip is queued, not once it has been heard, so
      // chunks are appended to one continuous sink without a gap between them.
      URL.revokeObjectURL(url);
      nativePlayingRef.current = true;
      usedNativeSinkRef.current = true;
      const epoch = streamEpochRef.current;
      void next
        .arrayBuffer()
        .then((buffer) =>
          // The buffer is the whole payload, so it crosses as raw bytes rather
          // than a JSON array of numbers. That encoding inflated four seconds of
          // speech from ~190 KB of audio to ~660 KB of text, built in the webview
          // and parsed again in Rust for every sentence. The epoch travels as a
          // header because the body is taken by the audio.
          invoke(
            "assistant_play_local_tts_chunk",
            buffer,
            epoch === null
              ? undefined
              : { headers: { [TTS_EPOCH_HEADER]: String(epoch) } },
          ),
        )
        .then(() => {
          // Only the current reply may clear the flag: a stale chunk resolving
          // late would otherwise make the queue look drained and let
          // `finishSynthesis` release the sink while audio is still coming.
          if (generation !== generationRef.current) return;
          nativePlayingRef.current = false;
          pump(generation);
        })
        .catch(() => {
          if (generation !== generationRef.current) return;
          nativePlayingRef.current = false;
          setError({ reason: "playback" });
          pump(generation);
        });
      return;
    }

    const el = new Audio(url);
    playingRef.current = el;

    // Guard against double-advancing if both the promise and an element event
    // fire for the same clip.
    let settled = false;
    const advance = () => {
      if (settled) return;
      settled = true;
      URL.revokeObjectURL(url);
      pump(generation);
    };
    el.onended = advance;
    el.onerror = advance;

    void el.play().catch((err: unknown) => {
      if (settled) return;
      // Superseded by a newer generation (Stop / new reply): just clean up.
      if (generation !== generationRef.current) {
        settled = true;
        URL.revokeObjectURL(url);
        return;
      }
      const blocked =
        !!err &&
        typeof err === "object" &&
        (err as { name?: string }).name === "NotAllowedError";
      if (blocked) {
        // The OS/WebView blocked auto-play because there was no recent user
        // gesture in this window. Keep the clip queued so a later click can
        // replay it, and surface WHY it went quiet instead of failing silently.
        settled = true;
        URL.revokeObjectURL(url);
        queueRef.current.unshift(next);
        playingRef.current = null;
        setError({ reason: "blocked" });
        setStatus((s) => (s === "speaking" ? "ready" : s));
        return;
      }
      // Any other playback failure (bad output device, decode error): report it,
      // then move on so one bad clip can't wedge the whole queue.
      setError({ reason: "playback" });
      advance();
    });
  }, []);

  /** Replay whatever is still queued — used after a `blocked` failure, once a
   *  user gesture has unlocked audio in this window. */
  const retry = useCallback(() => {
    setError(null);
    if (queueRef.current.length > 0) {
      setStatus("speaking");
      pump(generationRef.current);
    }
  }, [pump]);

  /** Drain a kokoro-js audio stream into the playback queue. Shared by the
   *  one-shot and streaming paths, which differ only in how text gets in. */
  const consume = useCallback(
    async (
      stream: AsyncIterable<{ audio: KokoroAudio }>,
      generation: number,
    ): Promise<SpeakOutcome> => {
      let started = false;
      let played = 0;
      let withheld = 0;
      for await (const { audio } of stream) {
        if (generation !== generationRef.current) return "stopped"; // superseded
        // A GPU that corrupts Kokoro throws nothing; the samples are the only
        // evidence, so the first clips of a WebGPU session are checked before
        // anyone hears them.
        if (
          loadedOnGpuRef.current &&
          gpuChecksLeftRef.current > 0 &&
          audio.audio &&
          audio.sampling_rate
        ) {
          const broken = audioLooksBroken(audio.audio, audio.sampling_rate);
          if (broken === true) {
            gpuStrikesRef.current += 1;
            // One odd clip may be a false alarm: it is muted and the next one
            // decides. A second means the card garbles the voice.
            if (gpuStrikesRef.current >= 2) {
              gpuStrikesRef.current = 0;
              handleGpuBroken();
              return "gpu";
            }
            withheld += 1;
            continue;
          }
          if (broken === false) {
            gpuStrikesRef.current = 0;
            gpuChecksLeftRef.current -= 1;
            if (gpuChecksLeftRef.current === 0) {
              reportWebGpu(true);
              // A deliberate "Graphics card" retry that works lifts the old
              // verdict, e.g. after a driver update.
              if (deviceRef.current === "gpu") clearGpuBroken();
            }
          } else if (gpuStrikesRef.current > 0) {
            // Too short to judge, while the card is under suspicion: nothing
            // unverified reaches the speaker.
            withheld += 1;
            continue;
          }
        }
        played += 1;
        queueRef.current.push(audio.toBlob());
        if (!started) {
          started = true;
          pump(generation);
        } else if (!playingRef.current && !nativePlayingRef.current) {
          pump(generation); // queue drained while synthesizing; resume
        }
      }
      // Nothing was queued (every clip withheld), so no drain will end the
      // "speaking" state for us.
      if (!started) setStatus((s) => (s === "speaking" ? "ready" : s));
      return played === 0 && withheld > 0 ? "muted" : "done";
    },
    [pump, handleGpuBroken],
  );

  /** Mark synthesis complete for `generation`, and release the native sink now
   *  if playback has already caught up (otherwise `pump` does it on drain). */
  const finishSynthesis = useCallback((generation: number) => {
    if (generation !== generationRef.current) return;
    synthDoneRef.current = true;
    if (browserSinkRef.current) {
      if (queueRef.current.length === 0 && !nativePlayingRef.current)
        browserSinkRef.current.finish(streamEpochRef.current);
      return;
    }
    if (
      isTauri() &&
      queueRef.current.length === 0 &&
      !nativePlayingRef.current
    ) {
      void invoke("assistant_finish_local_tts").catch(() => {});
    }
  }, []);

  /** Kokoro's own pace, rather than time-stretching the finished clip: the model
   *  adjusts its phoneme durations, so the result keeps the voice's pitch and
   *  natural pauses, and the setting is honoured whichever backend plays the
   *  audio (native playback has no `playbackRate` equivalent). Clamped because a
   *  hand-edited config could carry anything. */
  const currentSpeed = () => Math.min(4, Math.max(0.25, speedRef.current || 1));

  /**
   * Open a reply that is still being generated.
   *
   * The backend calls this the moment a turn starts speaking, then feeds
   * sentences in with [`pushText`] as the language model writes them. Loading the
   * model now — in parallel with generation, rather than after it — is itself a
   * large part of the latency win on a cold panel.
   */
  const beginStream = useCallback(
    async (epoch: number | null) => {
      if (!enabled) return;
      setError(null);
      // Local teardown only. The backend has already superseded any previous
      // reply and issued this reply's epoch; cancelling natively here would bump
      // that epoch and silence the reply we are about to speak.
      teardown(false);
      streamOpenRef.current = true;
      closeWhenReadyRef.current = false;
      pendingTextRef.current = [];
      synthDoneRef.current = false;
      streamEpochRef.current = epoch;
      const generation = generationRef.current;
      try {
        const model = await ensureLoaded();
        // Superseded (Stop, or a newer reply) while the model was loading.
        if (generation !== generationRef.current || !streamOpenRef.current)
          return;
        setStatus("speaking");

        const { TextSplitterStream } = await import("kokoro-js");
        const splitter = new TextSplitterStream();
        const stream = model.stream(splitter, { voice, speed: currentSpeed() });
        streamRef.current = { splitter, generation };

        // Anything that arrived during loading, in order.
        for (const buffered of pendingTextRef.current) splitter.push(buffered);
        pendingTextRef.current = [];
        if (closeWhenReadyRef.current) {
          closeWhenReadyRef.current = false;
          streamRef.current = null;
          streamOpenRef.current = false;
          splitter.close();
        }

        await consume(stream, generation);
        finishSynthesis(generation);
      } catch (e) {
        if (e instanceof SupersededLoad) {
          // The voice moved elsewhere while loading; end this reply quietly.
          finishSynthesis(generation);
          return;
        }
        console.error("Kokoro TTS stream failed:", e);
        finishSynthesis(generation);
        setError((prev) => prev ?? { reason: "synthesis" });
        setStatus("error");
      }
    },
    [enabled, voice, ensureLoaded, teardown, consume, finishSynthesis],
  );

  /** Feed the next sentence of the open reply. */
  const pushText = useCallback((text: string) => {
    if (!text.trim() || !streamOpenRef.current) return;
    const open = streamRef.current;
    if (open && open.generation === generationRef.current) {
      open.splitter.push(text);
    } else {
      // Model still loading — hold the text until the splitter exists.
      pendingTextRef.current.push(text);
    }
  }, []);

  /** Mark the reply complete so the last sentence is flushed and spoken. */
  const endStream = useCallback(() => {
    if (!streamOpenRef.current) return;
    const open = streamRef.current;
    if (open && open.generation === generationRef.current) {
      streamRef.current = null;
      streamOpenRef.current = false;
      open.splitter.close();
    } else {
      // The splitter appears after loading finishes; close it then.
      closeWhenReadyRef.current = true;
    }
  }, []);

  const speak = useCallback(
    async (text: string, force = false): Promise<SpeakOutcome> => {
      if ((!enabled && !force) || !text.trim()) return "stopped";
      setError(null);
      try {
        const model = await ensureLoaded();
        stop();
        const generation = generationRef.current;
        synthDoneRef.current = false;
        setStatus("speaking");

        const { TextSplitterStream } = await import("kokoro-js");
        const splitter = new TextSplitterStream();
        const stream = model.stream(splitter, { voice, speed: currentSpeed() });
        splitter.push(text);
        splitter.close();

        const outcome = await consume(stream, generation);
        finishSynthesis(generation);
        return outcome;
      } catch (e) {
        if (e instanceof SupersededLoad) return "stopped";
        console.error("Kokoro TTS failed:", e);
        synthDoneRef.current = true;
        // A load failure already set reason "load"; only mark synthesis when the
        // model was loaded but generating audio threw.
        setError((prev) => prev ?? { reason: "synthesis" });
        setStatus("error");
        return "failed";
      }
    },
    [enabled, voice, ensureLoaded, stop, consume, finishSynthesis],
  );

  return {
    status,
    progress,
    error,
    prepare: ensureLoaded,
    speak,
    beginStream,
    pushText,
    endStream,
    stop,
    retry,
  };
}
