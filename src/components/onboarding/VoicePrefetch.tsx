import React, { useEffect, useRef } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { useKokoroTts } from "@/assistant/useKokoroTts";
import { parseKokoroDevice } from "@/assistant/localVoice";
import { useSettingsStore } from "@/stores/settingsStore";
import { useSetupQueue } from "./setupQueue";

/**
 * Loads Kokoro into the web view's cache while setup's queue reaches it.
 *
 * Kokoro normally speaks inside the assistant window and downloads itself on
 * the first spoken reply, which would make someone's first call open with a
 * progress bar. This does the same load ahead of time, from the main window
 * (the two windows share one origin and one cache, which is how Settings'
 * Download button already works), then releases the model: unmounting the
 * hook frees the ONNX session.
 *
 * Mounted by `App` for the app's lifetime, because the queue outlives the
 * setup screens. Renders nothing.
 */
export const VoicePrefetch: React.FC = () => {
  const active = useSetupQueue((s) =>
    s.tasks.some(
      (task) =>
        task.job === "voice" &&
        task.modelId === null &&
        task.state === "downloading",
    ),
  );
  return active ? <Loader /> : null;
};

/** A load whose progress has not moved for this long is treated as failed,
 *  so a stalled fetch cannot hold the assistant's download behind it. The
 *  window covers the WebGPU warm-up after the last byte, which reports none. */
const STALL_MS = 120_000;

const Loader: React.FC = () => {
  const settings = useSettingsStore((s) => s.settings);
  const dtype = settings?.assistant_tts_kokoro_dtype || "fp32";
  const device = parseKokoroDevice(settings?.assistant_tts_kokoro_device);
  const voice = settings?.assistant_tts_voice || "af_heart";
  const kokoro = useKokoroTts(true, voice, dtype, 1, false, undefined, device);
  const { setWebviewVoiceProgress, finishWebviewVoice } =
    useSetupQueue.getState();
  const lastMove = useRef(Date.now());

  useEffect(() => {
    lastMove.current = Date.now();
    if (kokoro.status === "loading") setWebviewVoiceProgress(kokoro.progress);
  }, [kokoro.status, kokoro.progress, setWebviewVoiceProgress]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      if (Date.now() - lastMove.current > STALL_MS) {
        window.clearInterval(timer);
        console.warn("The voice download stalled during setup.");
        finishWebviewVoice(false);
      }
    }, 5_000);
    return () => window.clearInterval(timer);
  }, [finishWebviewVoice]);

  const { prepare } = kokoro;
  useEffect(() => {
    // Set on cleanup, so a load this effect no longer owns (a StrictMode
    // remount in development, or a device change, drops the model mid-load)
    // settles nothing: the next run of the effect starts its own.
    let cancelled = false;
    // The browser preview has no app cache to fill and no reason to pull
    // 300 MB from Hugging Face; it walks the bar instead.
    if (!isTauri()) {
      let progress = 0;
      const timer = window.setInterval(() => {
        progress += 12;
        lastMove.current = Date.now();
        setWebviewVoiceProgress(progress);
        if (progress >= 100) {
          window.clearInterval(timer);
          finishWebviewVoice(true);
        }
      }, 250);
      return () => window.clearInterval(timer);
    }
    // The web view's cache is the record that the voice is ready (Settings
    // reads it), so nothing else is written here.
    void prepare()
      .then(() => {
        if (!cancelled) finishWebviewVoice(true);
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        console.warn("Could not prepare the voice during setup:", error);
        finishWebviewVoice(false);
      });
    return () => {
      cancelled = true;
    };
  }, [prepare, dtype, device, setWebviewVoiceProgress, finishWebviewVoice]);

  return null;
};
