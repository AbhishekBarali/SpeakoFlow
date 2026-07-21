import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

/**
 * Setup phase of the built-in (llama.cpp) engine binary. This is the ENGINE
 * download — the one-time fetch of `llama-server` on first use — which is
 * separate from downloading a GGUF model. `null` means nothing is happening.
 */
export type LocalLlmEnginePhase =
  | "downloading"
  | "extracting"
  | "ready"
  | "error"
  | null;

export interface LocalLlmEngineStatus {
  phase: LocalLlmEnginePhase;
  /** Bytes downloaded so far (0 until the first progress event). */
  downloaded: number;
  /** Total bytes, or 0 when the server didn't report a Content-Length. */
  total: number;
  /** 0–100, or 0 when the total is unknown (show an indeterminate bar then). */
  pct: number;
  /** True while the engine is actively downloading or extracting. */
  active: boolean;
}

const IDLE: LocalLlmEngineStatus = {
  phase: null,
  downloaded: 0,
  total: 0,
  pct: 0,
  active: false,
};

/**
 * Subscribe to the built-in engine's setup lifecycle. The backend
 * (`managers/local_llm.rs`) emits `local-llm-engine-status`
 * ("downloading" | "extracting" | "ready" | "error") and
 * `local-llm-engine-progress` ({ downloaded, total }) during the first-run
 * engine download. Tauri broadcasts app-level events to every window, so this
 * works in both the settings window and the floating assistant panel.
 *
 * Before this, those events were emitted but nothing listened — so the very
 * first assistant turn silently downloaded ~100MB with zero feedback, and a
 * failure surfaced only as a cryptic "Model couldn't start". This hook is the
 * consumer that makes that setup visible.
 */
export function useLocalLlmEngineStatus(): LocalLlmEngineStatus {
  const [status, setStatus] = useState<LocalLlmEngineStatus>(IDLE);

  useEffect(() => {
    let disposed = false;
    let unlistenStatus: (() => void) | undefined;
    let unlistenProgress: (() => void) | undefined;
    let clearTimer: ReturnType<typeof setTimeout> | undefined;

    const stopClearTimer = () => {
      if (clearTimer) {
        clearTimeout(clearTimer);
        clearTimer = undefined;
      }
    };

    void (async () => {
      const s = await listen<string>("local-llm-engine-status", (event) => {
        const phase = event.payload as LocalLlmEnginePhase;
        stopClearTimer();
        if (phase === "ready" || phase === "error") {
          // Show the terminal state briefly, then reset so a stale banner never
          // lingers over a later, healthy turn.
          setStatus((prev) => ({ ...prev, phase, active: false }));
          clearTimer = setTimeout(() => {
            if (!disposed) setStatus(IDLE);
          }, 2000);
          return;
        }
        setStatus((prev) => ({
          ...prev,
          phase,
          active: phase === "downloading" || phase === "extracting",
        }));
      });

      const p = await listen<{ downloaded: number; total: number }>(
        "local-llm-engine-progress",
        (event) => {
          const { downloaded, total } = event.payload;
          const pct =
            total > 0
              ? Math.max(
                  0,
                  Math.min(100, Math.round((downloaded / total) * 100)),
                )
              : 0;
          stopClearTimer();
          setStatus((prev) => ({
            // A progress tick means we're downloading, unless we've already
            // advanced to extracting.
            phase: prev.phase === "extracting" ? "extracting" : "downloading",
            downloaded,
            total,
            pct,
            active: true,
          }));
        },
      );

      if (disposed) {
        s();
        p();
        return;
      }
      unlistenStatus = s;
      unlistenProgress = p;
    })();

    return () => {
      disposed = true;
      stopClearTimer();
      unlistenStatus?.();
      unlistenProgress?.();
    };
  }, []);

  return status;
}
