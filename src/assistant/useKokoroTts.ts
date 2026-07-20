import { useCallback, useEffect, useRef, useState } from "react";

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
    options: { voice?: string },
  ): AsyncIterable<{ text: string; audio: { toBlob(): Blob } }>;
}

interface TextSplitter {
  push(text: string): void;
  close(): void;
}

const KOKORO_MODEL_ID = "onnx-community/Kokoro-82M-v1.0-ONNX";

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
  // Latest playback speed, read when each audio chunk starts so changes apply
  // to the next clip without re-creating the playback callbacks.
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
  const generationRef = useRef(0);

  const ensureLoaded = useCallback(async (): Promise<KokoroModel> => {
    if (modelRef.current) return modelRef.current;
    if (!loadingRef.current) {
      setError(null);
      setStatus("loading");
      setProgress(0);
      loadingRef.current = (async () => {
        const { KokoroTTS } = await import("kokoro-js");
        const useGpu = hasWebGpu();
        const chosenDtype = dtypeRef.current;
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

  const stop = useCallback(() => {
    generationRef.current += 1; // invalidate in-flight generation
    queueRef.current = [];
    const el = playingRef.current;
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
      setStatus((s) => (s === "speaking" ? "ready" : s));
      return;
    }
    const url = URL.createObjectURL(next);
    const el = new Audio(url);
    // Pitch-preserved time-stretch so faster/slower speech still sounds natural
    // (preservesPitch defaults to true in Chromium, set explicitly for clarity).
    // Clamp defensively: the setting is already clamped to 0.25–4 on write, but
    // playbackRate throws for out-of-range values (e.g. a hand-edited config).
    el.playbackRate = Math.min(4, Math.max(0.25, speedRef.current || 1));
    el.preservesPitch = true;
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

  const speak = useCallback(
    async (text: string, force = false) => {
      if ((!enabled && !force) || !text.trim()) return;
      setError(null);
      try {
        const model = await ensureLoaded();
        stop();
        const generation = generationRef.current;
        setStatus("speaking");

        const { TextSplitterStream } = await import("kokoro-js");
        const splitter = new TextSplitterStream();
        const stream = model.stream(splitter, { voice });
        splitter.push(text);
        splitter.close();

        let started = false;
        for await (const { audio } of stream) {
          if (generation !== generationRef.current) return; // superseded
          queueRef.current.push(audio.toBlob());
          if (!started) {
            started = true;
            pump(generation);
          } else if (!playingRef.current) {
            pump(generation); // queue drained while synthesizing; resume
          }
        }
      } catch (e) {
        console.error("Kokoro TTS failed:", e);
        // A load failure already set reason "load"; only mark synthesis when the
        // model was loaded but generating audio threw.
        setError((prev) => prev ?? { reason: "synthesis" });
        setStatus("error");
      }
    },
    [enabled, voice, ensureLoaded, stop, pump],
  );

  return {
    status,
    progress,
    error,
    prepare: ensureLoaded,
    speak,
    stop,
    retry,
  };
}
