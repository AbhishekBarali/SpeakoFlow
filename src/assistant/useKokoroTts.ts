import { useCallback, useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";

export type TtsStatus = "off" | "loading" | "ready" | "speaking" | "error";

/** Why local (Kokoro) speech failed, so the panel can show a precise, useful
 *  message instead of going silent:
 *  - `load`      — the model couldn't download / initialize.
 *  - `synthesis` — the model loaded but couldn't turn text into audio.
 *  - `blocked`   — the system blocked auto-play (needs a user gesture); the
 *                  clip is kept queued so a later click can replay it.
 *  - `playback`  — the audio element failed to play (output device issue). */
export type KokoroErrorReason = "load" | "synthesis" | "blocked" | "playback";
export interface KokoroError {
  reason: KokoroErrorReason;
}

/** Minimal surface of the kokoro-js model we use (erases its strict voice
 *  union type so the voice id can come from settings). */
interface KokoroModel {
  stream(
    splitter: TextSplitter,
    options: { voice?: string; speed?: number },
  ): AsyncIterable<{ text: string; audio: { toBlob(): Blob } }>;
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
) {
  const modelRef = useRef<KokoroModel | null>(null);
  const loadingRef = useRef<Promise<KokoroModel> | null>(null);
  const dtypeRef = useRef(dtype);
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

  const ensureLoaded = useCallback(async (): Promise<KokoroModel> => {
    if (modelRef.current) return modelRef.current;
    if (!loadingRef.current) {
      setError(null);
      setStatus("loading");
      setProgress(0);
      loadingRef.current = (async () => {
        const { KokoroTTS } = await import("kokoro-js");
        const useGpu = hasWebGpu();
        const requestedDtype = dtypeRef.current;
        // WebKitGTK commonly has no WebGPU. Loading the 325 MB fp32 graph into
        // WASM can appear to hang after the text answer is already visible;
        // use the cached 92 MB q8 graph directly on CPU instead of waiting for
        // fp32 initialization to fail first.
        const chosenDtype =
          !useGpu && (requestedDtype === "fp32" || requestedDtype === "fp16")
            ? "q8"
            : requestedDtype;
        // Track download progress of the (largest) onnx weights file.
        const progress_callback = (event: ProgressEvent) => {
          if (
            event.status === "progress" &&
            event.file?.endsWith(".onnx") &&
            typeof event.progress === "number"
          ) {
            setProgress(Math.round(event.progress));
          }
        };
        type LoadOptions = Parameters<typeof KokoroTTS.from_pretrained>[1];
        let model: unknown;
        try {
          model = await KokoroTTS.from_pretrained(KOKORO_MODEL_ID, {
            dtype: chosenDtype,
            device: useGpu ? "webgpu" : "wasm",
            progress_callback,
          } as unknown as LoadOptions);
          console.info(
            `[Kokoro TTS] loaded on ${useGpu ? "webgpu" : "wasm/CPU"} (${chosenDtype})`,
          );
        } catch (gpuErr) {
          // WebGPU init can fail (driver/feature limits). Fall back to wasm.
          // fp32/fp16 are too heavy for CPU, so drop to q8 there.
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
          }
          model = await KokoroTTS.from_pretrained(KOKORO_MODEL_ID, {
            dtype: fallbackDtype,
            device: "wasm",
            progress_callback,
          } as unknown as LoadOptions);
          console.info(
            `[Kokoro TTS] loaded on wasm/CPU (${fallbackDtype}) fallback`,
          );
        }
        modelRef.current = model as KokoroModel;
        setStatus("ready");
        return modelRef.current;
      })().catch((e: unknown) => {
        loadingRef.current = null;
        setStatus("error");
        setError({ reason: "load" });
        throw e;
      });
    }
    return loadingRef.current;
  }, []);

  // Callers choose whether passive mounting should prepare the model. Settings
  // disables this and exposes an explicit setup action; the live assistant keeps
  // it enabled so an actual spoken reply can start promptly.
  useEffect(() => {
    if (enabled && preload) {
      ensureLoaded().catch(() => {});
    } else if (!enabled) {
      setError(null);
      setStatus((s) => (s === "speaking" || s === "ready" ? "off" : s));
      // Turned off: free the model so its ONNX/WebGPU memory isn't pinned for
      // the WebView's lifetime. It reloads on demand if re-enabled.
      void disposeModel(modelRef.current);
      modelRef.current = null;
      loadingRef.current = null;
    }
  }, [enabled, preload, ensureLoaded]);

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

  const stop = useCallback(() => teardown(true), [teardown]);

  // When the dtype (precision) changes, drop the cached model so the next
  // synthesis reloads at the new precision.
  useEffect(() => {
    if (dtypeRef.current === dtype) return;
    dtypeRef.current = dtype;
    stop();
    // Release the old-precision session before dropping the ref, or its
    // ONNX/WebGPU memory leaks on every precision change.
    void disposeModel(modelRef.current);
    modelRef.current = null;
    loadingRef.current = null;
    setStatus("off");
    if (enabled && preload) {
      ensureLoaded().catch(() => {});
    }
  }, [dtype, enabled, preload, ensureLoaded, stop]);

  // Release the model + audio when the hook unmounts (e.g. the panel window is
  // torn down). The ONNX session and its WebGPU buffers are hundreds of MB;
  // without this they leak for the lifetime of the WebView.
  useEffect(() => {
    return () => {
      generationRef.current += 1;
      queueRef.current = [];
      const el = playingRef.current;
      nativePlayingRef.current = false;
      if (isTauri()) {
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
      if (isTauri() && synthDoneRef.current) {
        void invoke("assistant_finish_local_tts", {
          epoch: streamEpochRef.current,
        }).catch(() => {});
      }
      setStatus((s) => (s === "speaking" ? "ready" : s));
      return;
    }
    const url = URL.createObjectURL(next);

    if (isTauri()) {
      // The HUD is deliberately non-focus-stealing. Route generated speech to
      // the native audio backend so Linux WebKit cannot suppress it as
      // background autoplay and so the configured output device is respected.
      // The call returns once the clip is queued, not once it has been heard, so
      // chunks are appended to one continuous sink without a gap between them.
      URL.revokeObjectURL(url);
      nativePlayingRef.current = true;
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
      stream: AsyncIterable<{ audio: { toBlob(): Blob } }>,
      generation: number,
    ) => {
      let started = false;
      for await (const { audio } of stream) {
        if (generation !== generationRef.current) return; // superseded
        queueRef.current.push(audio.toBlob());
        if (!started) {
          started = true;
          pump(generation);
        } else if (!playingRef.current && !nativePlayingRef.current) {
          pump(generation); // queue drained while synthesizing; resume
        }
      }
    },
    [pump],
  );

  /** Mark synthesis complete for `generation`, and release the native sink now
   *  if playback has already caught up (otherwise `pump` does it on drain). */
  const finishSynthesis = useCallback((generation: number) => {
    if (generation !== generationRef.current) return;
    synthDoneRef.current = true;
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
    async (text: string, force = false) => {
      if ((!enabled && !force) || !text.trim()) return;
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

        await consume(stream, generation);
        finishSynthesis(generation);
      } catch (e) {
        console.error("Kokoro TTS failed:", e);
        synthDoneRef.current = true;
        // A load failure already set reason "load"; only mark synthesis when the
        // model was loaded but generating audio threw.
        setError((prev) => prev ?? { reason: "synthesis" });
        setStatus("error");
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
