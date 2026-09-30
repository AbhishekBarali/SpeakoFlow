import { useEffect, useState } from "react";
import { arch, locale, platform } from "@tauri-apps/plugin-os";
import { commands } from "@/bindings";
import {
  gpuMarkedBroken,
  probeWebGpu,
  reportWebGpu,
} from "@/assistant/localVoice";
import { pickGpu, prefersEnglish, type HardwareFacts } from "./recommend";

const safe = <T>(read: () => T, fallback: T): T => {
  try {
    return read();
  } catch {
    return fallback;
  }
};

/**
 * The few facts about this computer that setup's suggestions depend on, or
 * null while they are still being read (well under a second: the GPU list is
 * pre-warmed at launch, and the WebGPU probe gives up after 2.5 s).
 *
 * The WebGPU answer is also reported to the backend, so the voice's Automatic
 * routing knows before the first reply where Kokoro can run.
 */
export function useHardwareFacts(): HardwareFacts | null {
  const [facts, setFacts] = useState<HardwareFacts | null>(null);

  useEffect(() => {
    let cancelled = false;
    const os = safe(platform, "");

    void Promise.allSettled([
      commands.getSystemMemoryGb(),
      commands.getAvailableAccelerators(),
      commands.getLocalVoiceStatus(),
      probeWebGpu(2500),
      // The OS locale, not the app's language: i18n starts in English and only
      // follows the system when a translation exists for it.
      locale(),
    ]).then(([memory, accelerators, voice, webgpu, osLocale]) => {
      if (cancelled) return;
      const gpuUsable =
        webgpu.status === "fulfilled" && webgpu.value && !gpuMarkedBroken();
      reportWebGpu(gpuUsable);
      const voiceStatus =
        voice.status === "fulfilled" && voice.value ? voice.value : null;
      const primaryLocale =
        (osLocale.status === "fulfilled" && osLocale.value) ||
        (typeof navigator !== "undefined" ? navigator.language : "");
      setFacts({
        memoryGb:
          memory.status === "fulfilled" && Number.isFinite(memory.value)
            ? memory.value
            : 0,
        gpu:
          accelerators.status === "fulfilled" && accelerators.value
            ? pickGpu(accelerators.value.gpu_devices ?? [])
            : null,
        cores:
          typeof navigator !== "undefined"
            ? (navigator.hardwareConcurrency ?? 0)
            : 0,
        unifiedMemory: os === "macos" && safe(arch, "") === "aarch64",
        webgpu: gpuUsable,
        // Not suggested on macOS: an ad-hoc-signed build is expected to have
        // the native library refused by library validation, which cannot be
        // known until it is loaded, so the pack would download for nothing.
        nativeVoice:
          os !== "macos" &&
          !!voiceStatus?.native_supported &&
          !voiceStatus.native_load_failed,
        prefersEnglish: prefersEnglish([primaryLocale]),
      });
    });
    return () => {
      cancelled = true;
    };
  }, []);

  return facts;
}
