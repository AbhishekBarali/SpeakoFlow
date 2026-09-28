import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { LocalVoiceStatus } from "@/bindings";

/**
 * Where the local voice runs, seen from a WebView.
 *
 * Kokoro has two homes: this WebView (kokoro-js, on WebGPU, or on WebAssembly
 * when there is no usable GPU) and the native engine in `native_tts.rs`, which
 * runs it on every processor core. Rust decides which one speaks
 * (`native_tts::route`), but only a WebView can see the GPU, so the WebView
 * reports what it finds — and keeps its hands off the Kokoro weights (~310 MB,
 * plus a copy as GPU buffers) whenever the answer is "native".
 */

export type KokoroDevice = "auto" | "gpu" | "cpu";

/** Catalog ids of the native voice packs (`native_tts::PACKS`). */
export const KOKORO_NATIVE_MODEL_ID = "kokoro-82m-native";
export const KITTEN_MODEL_ID = "kitten-nano-0.8";

export const parseKokoroDevice = (
  value: string | null | undefined,
): KokoroDevice => (value === "gpu" || value === "cpu" ? value : "auto");

/**
 * Remembered across launches, because garbled audio is a property of this
 * machine's GPU and driver rather than of one reply. Only Automatic reads it:
 * choosing "Graphics card" explicitly tries WebGPU again (after a driver
 * update, say), and a clean run there clears it.
 */
const GPU_BROKEN_KEY = "speakoflow.kokoro.gpuBroken";

/** This WebView's own verdict, which holds even when storage is unavailable. */
let gpuBrokenThisSession = false;

export function gpuMarkedBroken(): boolean {
  if (gpuBrokenThisSession) return true;
  try {
    return window.localStorage.getItem(GPU_BROKEN_KEY) === "1";
  } catch {
    return false;
  }
}

export function markGpuBroken(): void {
  gpuBrokenThisSession = true;
  try {
    window.localStorage.setItem(GPU_BROKEN_KEY, "1");
  } catch {
    // Storage unavailable: the session flag and the report to Rust still hold.
  }
}

export function clearGpuBroken(): void {
  gpuBrokenThisSession = false;
  try {
    window.localStorage.removeItem(GPU_BROKEN_KEY);
  } catch {
    // ignore
  }
}

/** Tell Rust whether this WebView can run Kokoro on the graphics card. */
export function reportWebGpu(usable: boolean): void {
  if (!isTauri()) return;
  void invoke("assistant_report_webgpu", { usable }).catch(() => {});
}

/** Whether this WebView hands out a WebGPU adapter. Cheap: no model is loaded,
 *  so it can run before the first reply decides where to speak. A driver that
 *  never answers counts as no adapter after `timeoutMs`; if the card does work,
 *  its first clean replies report it usable again. */
export async function probeWebGpu(timeoutMs = 3000): Promise<boolean> {
  const gpu = (
    navigator as Navigator & {
      gpu?: { requestAdapter(): Promise<unknown> };
    }
  ).gpu;
  if (!gpu) return false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const adapter = await Promise.race([
      gpu.requestAdapter(),
      new Promise<null>((resolve) => {
        timer = setTimeout(() => resolve(null), timeoutMs);
      }),
    ]);
    return adapter != null;
  } catch {
    return false;
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/** Below this a sample is pause-level hiss and never counts as a crossing. */
const CROSSING_LEVEL = 0.01;

/**
 * Whether a synthesized clip is certainly not speech, or `null` when it is too
 * short to judge.
 *
 * A GPU that corrupts Kokoro throws no error and returns well-formed buffers,
 * so the only evidence is the samples. Real speech from Kokoro and Kitten
 * measured a zero-crossing rate of 0.05–0.14, RMS 0.06–0.13, and no samples at
 * full scale. Each check below sits far outside that, so a clean voice is never
 * flagged: non-finite samples or samples pinned at full scale (judged from
 * 0.1 s, since no real clip has either); and, from half a second, where a
 * short clip can be one hissy consonant, near-silence where a sentence should
 * be (the "no audio" failure) or a crossing rate that belongs to noise (white
 * noise is ~0.5). Crossings only count above a small level, so the quiet hiss
 * between words cannot add up to "noise". A corruption that still sounds like
 * a slow, smooth signal would pass, which is why "Processor" stays selectable.
 */
export function audioLooksBroken(
  samples: ArrayLike<number>,
  sampleRate: number,
): boolean | null {
  const n = samples.length;
  if (!sampleRate || n < sampleRate * 0.1) return null;
  let sumSquares = 0;
  let crossings = 0;
  let clipped = 0;
  // Sign of the last sample loud enough to count; 0 until there is one.
  let lastSign = 0;
  for (let i = 0; i < n; i++) {
    const value = samples[i];
    if (!Number.isFinite(value)) return true;
    sumSquares += value * value;
    const magnitude = Math.abs(value);
    if (magnitude >= 0.99) clipped++;
    if (magnitude >= CROSSING_LEVEL) {
      const sign = value > 0 ? 1 : -1;
      if (lastSign !== 0 && sign !== lastSign) crossings++;
      lastSign = sign;
    }
  }
  if (clipped / n > 0.02) return true;
  if (n < sampleRate * 0.5) return null;
  if (Math.sqrt(sumSquares / n) < 0.01) return true;
  return crossings / n > 0.3;
}

/**
 * The backend's view of the local voice, kept current.
 *
 * With `probe` set and nothing reported yet this session, it also checks for a
 * WebGPU adapter and reports the answer, so Automatic can pick the processor
 * before the first reply rather than after a slow one.
 */
export function useLocalVoiceStatus(options: {
  enabled?: boolean;
  probe?: boolean;
}): LocalVoiceStatus | null {
  const { enabled = true, probe = false } = options;
  const [status, setStatus] = useState<LocalVoiceStatus | null>(null);
  const probed = useRef(false);

  useEffect(() => {
    if (!enabled || !isTauri()) return;
    let alive = true;
    const refresh = () => {
      void invoke<LocalVoiceStatus>("get_local_voice_status")
        .then((next) => {
          if (alive) setStatus(next);
        })
        .catch(() => {});
    };
    refresh();
    const unlisteners = [
      listen<LocalVoiceStatus>("local-voice-status-changed", (event) => {
        if (alive) setStatus(event.payload);
      }),
      // A pack finishing its download (or being removed) can change the route.
      listen("model-download-complete", refresh),
      listen("model-deleted", refresh),
    ];
    return () => {
      alive = false;
      for (const unlisten of unlisteners) {
        void unlisten.then((stop) => stop()).catch(() => {});
      }
    };
  }, [enabled]);

  useEffect(() => {
    if (!enabled || !probe || probed.current) return;
    if (status?.webgpu !== "unknown") return;
    probed.current = true;
    void (async () => {
      const usable = !gpuMarkedBroken() && (await probeWebGpu());
      reportWebGpu(usable);
    })();
  }, [enabled, probe, status?.webgpu]);

  return status;
}
