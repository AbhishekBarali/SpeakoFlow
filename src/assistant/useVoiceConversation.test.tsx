import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { RealTimeVADOptions } from "@ricky0123/vad-web";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}
const events = new Map<string, (event: { payload: unknown }) => void>();
const calls: { command: string; args: unknown }[] = [];
let backendStart: Promise<unknown> | null = null;
let audioTurn: Promise<unknown> | null = null;
let turn = 0;
let nextSession = 0;
let vadOptions: RealTimeVADOptions;
const tracks: { stopped: boolean; stop: () => void }[] = [];
let microphone: Promise<MediaStream> | null = null;
let sourceStarts = 0;
let callbackBegins = 0;

mock.module("@tauri-apps/api/core", () => ({
  invoke: async (command: string, args: unknown) => {
    calls.push({ command, args });
    if (command === "assistant_conversation_start")
      return backendStart ?? { session: ++nextSession, turn: 0 };
    if (command === "assistant_conversation_interrupt")
      return { session: nextSession, turn: ++turn };
    if (command === "assistant_conversation_audio") return audioTurn;
  },
}));
mock.module("@tauri-apps/api/event", () => ({
  listen: async (
    name: string,
    handler: (event: { payload: unknown }) => void,
  ) => {
    events.set(name, handler);
    return () => events.delete(name);
  },
}));
mock.module("@ricky0123/vad-web", () => ({
  MicVAD: {
    new: async (options: RealTimeVADOptions) => {
      vadOptions = options;
      let stream = await options.getStream();
      return {
        destroy: async () => options.pauseStream(stream),
        pause: async () => options.pauseStream(stream),
        start: async () => {
          stream = await options.resumeStream(stream);
        },
        setOptions() {},
      };
    },
  },
}));
const { useVoiceConversation } = await import("./useVoiceConversation");
let voice: ReturnType<typeof useVoiceConversation>;
let renderer: ReactTestRenderer;
function Harness() {
  voice = useVoiceConversation({
    stopLocal() {},
    beginLocal: async () => {
      callbackBegins++;
    },
    pushLocal() {},
    endLocal() {},
  });
  return null;
}
const stream = () => {
  const track = {
    stopped: false,
    stop() {
      track.stopped = true;
    },
  };
  tracks.push(track);
  return {
    getTracks: () => [track],
    getAudioTracks: () => [track],
  } as unknown as MediaStream;
};
beforeEach(async () => {
  calls.length = 0;
  tracks.length = 0;
  turn = 0;
  nextSession = 0;
  sourceStarts = 0;
  callbackBegins = 0;
  backendStart = null;
  audioTurn = null;
  microphone = null;
  events.clear();
  globalThis.window = {
    addEventListener() {},
    removeEventListener() {},
  } as unknown as Window & typeof globalThis;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      mediaDevices: {
        getUserMedia: async () => microphone ?? stream(),
        enumerateDevices: async () => [],
      },
    },
  });
  globalThis.AudioContext = class {
    currentTime = 0;
    destination = {};
    async resume() {}
    async close() {}
    createGain() {
      return { gain: { value: 1 }, connect() {} };
    }
    async decodeAudioData() {
      return { duration: 1 };
    }
    createBufferSource() {
      return {
        connect() {},
        disconnect() {},
        stop() {},
        start() {
          sourceStarts++;
        },
      };
    }
  } as unknown as typeof AudioContext;
  await act(async () => {
    renderer = create(<Harness />);
  });
});
afterEach(async () => {
  await act(async () => renderer.unmount());
});
const emit = async (name: string, payload: unknown) => {
  await act(async () => events.get(name)?.({ payload }));
};
const speak = async () => {
  await act(async () => {
    vadOptions.onSpeechRealStart();
  });
  await act(async () => {
    vadOptions.onSpeechEnd(new Float32Array(16_000));
  });
};

describe("hands-free session lifecycle", () => {
  test("opening starts listening; completed speech submits without a button", async () => {
    await act(async () => voice.start());
    expect(voice.phase).toBe("listening");
    await speak();
    expect(
      calls.filter((call) => call.command === "assistant_conversation_audio")
        .length,
    ).toBe(1);
    expect(voice.phase).toBe("listening");
  });
  test("end while microphone permission is pending releases late tracks", async () => {
    const mic = deferred<MediaStream>();
    microphone = mic.promise;
    let opening!: Promise<void>;
    await act(async () => {
      opening = voice.start();
    });
    await act(async () => voice.end());
    await act(async () => {
      mic.resolve(stream());
      await opening;
    });
    expect(tracks.every((track) => track.stopped)).toBe(true);
    expect(
      calls.some((call) => call.command === "assistant_conversation_start"),
    ).toBe(false);
    expect(voice.open).toBe(false);
  });
  test("a backend session arriving after End is immediately closed", async () => {
    const pending = deferred<unknown>();
    backendStart = pending.promise;
    let opening!: Promise<void>;
    await act(async () => {
      opening = voice.start();
    });
    await act(async () => voice.end());
    await act(async () => {
      pending.resolve({ session: 42, turn: 0 });
      await opening;
    });
    expect(
      calls.some(
        (call) =>
          call.command === "assistant_conversation_end" &&
          (call.args as { session: number }).session === 42,
      ),
    ).toBe(true);
    expect(voice.open).toBe(false);
  });
  test("mute stops capture and rejects an in-flight utterance and late speech", async () => {
    const pending = deferred<unknown>();
    audioTurn = pending.promise;
    await act(async () => voice.start());
    await speak();
    await act(async () => voice.toggleMute());
    expect(tracks.every((track) => track.stopped)).toBe(true);
    expect(voice.phase).toBe("muted");
    await emit("assistant-conversation-local", {
      ticket: { session: 1, turn: 1 },
      epoch: 7,
      kind: "begin",
    });
    await emit("assistant-conversation-audio", {
      ticket: { session: 1, turn: 1 },
      epoch: 7,
      audio: "AAAAAA==",
    });
    await act(async () => pending.resolve(undefined));
    expect(callbackBegins).toBe(0);
    expect(sourceStarts).toBe(0);
    expect(voice.phase).toBe("muted");
  });
  test("speech onset interrupts a reply and old completion cannot reset the new turn", async () => {
    const pending = deferred<unknown>();
    audioTurn = pending.promise;
    await act(async () => voice.start());
    await speak();
    await act(async () => vadOptions.onSpeechRealStart());
    await act(async () => pending.resolve(undefined));
    expect(voice.phase).toBe("hearing");
    expect(
      calls.filter(
        (call) => call.command === "assistant_conversation_interrupt",
      ).length,
    ).toBe(2);
    await emit("assistant-conversation-audio", {
      ticket: { session: 1, turn: 1 },
      epoch: 7,
      audio: "AAAAAA==",
    });
    expect(sourceStarts).toBe(0);
  });
  test("hidden and collapsed panels keep listening until explicitly ended", async () => {
    await act(async () => voice.start());
    await emit("assistant-panel-hidden", null);
    await emit("assistant-collapsed", true);
    expect(tracks.every((track) => !track.stopped)).toBe(true);
    expect(voice.open).toBe(true);
    await speak();
    expect(
      calls.filter((call) => call.command === "assistant_conversation_audio")
        .length,
    ).toBe(1);
    await emit("assistant-conversation-audio-begin", {
      ticket: { session: 1, turn: 1 },
      epoch: 7,
    });
    await emit("assistant-conversation-audio", {
      ticket: { session: 1, turn: 1 },
      epoch: 7,
      audio: "AAAAAA==",
    });
    expect(sourceStarts).toBe(1);
    await act(async () => voice.end());
    expect(tracks.every((track) => track.stopped)).toBe(true);
    await speak();
    expect(
      calls.filter((call) => call.command === "assistant_conversation_audio")
        .length,
    ).toBe(1);
  });
  test("a failed turn reports the error but keeps the microphone open", async () => {
    await act(async () => voice.start());
    await speak();
    await emit("assistant-error", {
      code: "provider",
      detail: "Conversation roles must alternate user/assistant",
    });
    // The session survives: still open, still listening, mic never released.
    expect(voice.open).toBe(true);
    expect(voice.phase).toBe("listening");
    expect(voice.error?.detail).toContain("roles must alternate");
    expect(tracks.every((track) => !track.stopped)).toBe(true);
    expect(
      calls.some((call) => call.command === "assistant_conversation_end"),
    ).toBe(false);
    // And the next utterance still reaches the backend, with the error cleared.
    await speak();
    expect(voice.error).toBe(null);
    expect(
      calls.filter((call) => call.command === "assistant_conversation_audio")
        .length,
    ).toBe(2);
  });
  test("a late ended event from the previous session cannot close a new one", async () => {
    await act(async () => voice.start());
    await act(async () => voice.end());
    await act(async () => voice.start());
    await emit("assistant-conversation-ended", 1);
    expect(voice.open).toBe(true);
    expect(voice.phase).toBe("listening");
    await emit("assistant-conversation-ended", 2);
    expect(voice.open).toBe(false);
  });
  test("remote synthesis remains Thinking after text generation ends", async () => {
    const pending = deferred<unknown>();
    audioTurn = pending.promise;
    await act(async () => voice.start());
    await speak();
    await emit("assistant-conversation-audio-begin", {
      ticket: { session: 1, turn: 1 },
      epoch: 7,
    });
    await act(async () => pending.resolve(undefined));
    expect(voice.phase).toBe("responding");
    await emit("assistant-conversation-audio-end", {
      ticket: { session: 1, turn: 1 },
      epoch: 7,
    });
    expect(voice.phase).toBe("listening");
  });
});
