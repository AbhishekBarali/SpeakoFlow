import { useCallback, useEffect, useRef, useState } from "react";

export type TtsStatus = "off" | "loading" | "ready" | "speaking" | "error";

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

  // Playback queue state (refs: updated from async generators)
  const queueRef = useRef<Blob[]>([]);
  const playingRef = useRef<HTMLAudioElement | null>(null);
  const generationRef = useRef(0);

  const ensureLoaded = useCallback(async (): Promise<KokoroModel> => {
    if (modelRef.current) return modelRef.current;
    if (!loadingRef.current) {
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
        throw e;
      });
    }
    return loadingRef.current;
  }, []);

  // Preload as soon as TTS is switched on so the first answer speaks fast.
  useEffect(() => {
    if (enabled) {
      ensureLoaded().catch(() => {});
    } else {
      setStatus((s) => (s === "speaking" || s === "ready" ? "off" : s));
    }
  }, [enabled, ensureLoaded]);

  const stop = useCallback(() => {
    generationRef.current += 1; // invalidate in-flight generation
    queueRef.current = [];
    const el = playingRef.current;
    if (el) {
      el.pause();
      playingRef.current = null;
    }
    setStatus((s) => (s === "speaking" ? "ready" : s));
  }, []);

  // When the dtype (precision) changes, drop the cached model so the next
  // synthesis reloads at the new precision.
  useEffect(() => {
    if (dtypeRef.current === dtype) return;
    dtypeRef.current = dtype;
    stop();
    modelRef.current = null;
    loadingRef.current = null;
    setStatus("off");
    if (enabled) {
      ensureLoaded().catch(() => {});
    }
  }, [dtype, enabled, ensureLoaded, stop]);

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
    const done = () => {
      URL.revokeObjectURL(url);
      pump(generation);
    };
    el.onended = done;
    el.onerror = done;
    void el.play().catch(done);
  }, []);

  const speak = useCallback(
    async (text: string, force = false) => {
      if ((!enabled && !force) || !text.trim()) return;
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
        setStatus("error");
      }
    },
    [enabled, voice, ensureLoaded, stop, pump],
  );

  return { status, progress, speak, stop };
}
