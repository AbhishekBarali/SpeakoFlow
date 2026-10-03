import { beforeEach, describe, expect, mock, test } from "bun:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

type Progress = (event: {
  status: string;
  file?: string;
  progress?: number;
}) => void;

/** Every `from_pretrained` call: its progress callback, and a way to finish it. */
const loads: {
  progress: Progress;
  resolve: (model: unknown) => void;
}[] = [];

mock.module("kokoro-js", () => ({
  KokoroTTS: {
    from_pretrained: (
      _id: string,
      options: { progress_callback: Progress },
    ): Promise<unknown> =>
      new Promise((resolve) => {
        loads.push({ progress: options.progress_callback, resolve });
      }),
  },
}));
mock.module("@tauri-apps/api/core", () => ({
  invoke: async () => {},
  // Module mocks leak into later test files; localVoice.ts imports this.
  isTauri: () => false,
}));
mock.module("@tauri-apps/api/event", () => ({
  emit: async () => {},
  listen: async () => () => {},
}));

const { useKokoroTts } = await import("./useKokoroTts");

type Hook = ReturnType<typeof useKokoroTts>;
let hook: Hook;
const Probe: React.FC<{ enabled: boolean }> = ({ enabled }) => {
  hook = useKokoroTts(enabled, "af_heart", "q8", 1, false);
  return null;
};

const onnx = (progress: number) => ({
  status: "progress",
  file: "onnx/model_quantized.onnx",
  progress,
});

describe("useKokoroTts download progress", () => {
  let renderer: ReactTestRenderer;

  beforeEach(async () => {
    loads.length = 0;
    await act(async () => {
      renderer = create(<Probe enabled />);
    });
  });

  test("a download only moves its bar forward", async () => {
    await act(async () => {
      void hook.prepare().catch(() => {});
    });
    expect(loads).toHaveLength(1);
    await act(async () => loads[0].progress(onnx(40)));
    await act(async () => loads[0].progress(onnx(55)));
    await act(async () => loads[0].progress(onnx(12)));
    expect(hook.status).toBe("loading");
    expect(hook.progress).toBe(55);
    // Files other than the weights do not move it.
    await act(async () =>
      loads[0].progress({
        status: "progress",
        file: "config.json",
        progress: 99,
      }),
    );
    expect(hook.progress).toBe(55);
  });

  test("pressing Test mid-download joins it instead of starting another", async () => {
    await act(async () => {
      void hook.prepare().catch(() => {});
    });
    await act(async () => loads[0].progress(onnx(30)));
    await act(async () => {
      void hook.prepare().catch(() => {});
    });
    expect(loads).toHaveLength(1);
    expect(hook.progress).toBe(30);
  });

  // The reported bug: a dropped download is never aborted and keeps reporting.
  // With one shared callback it and its replacement took turns writing the
  // bar, which swung between their percentages on every chunk.
  test("a dropped download cannot move the bar of the one that replaced it", async () => {
    await act(async () => {
      void hook.prepare().catch(() => {});
    });
    await act(async () => loads[0].progress(onnx(60)));

    // Turning the voice off drops the load and takes the bar down with it.
    await act(async () => renderer.update(<Probe enabled={false} />));
    expect(hook.status).toBe("off");
    expect(hook.progress).toBe(0);

    await act(async () => renderer.update(<Probe enabled />));
    await act(async () => {
      void hook.prepare().catch(() => {});
    });
    expect(loads).toHaveLength(2);

    await act(async () => loads[1].progress(onnx(5)));
    await act(async () => loads[0].progress(onnx(70)));
    await act(async () => loads[1].progress(onnx(8)));
    await act(async () => loads[0].progress(onnx(80)));
    expect(hook.status).toBe("loading");
    expect(hook.progress).toBe(8);
  });
});
