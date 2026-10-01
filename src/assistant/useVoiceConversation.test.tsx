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
/** What `assistant_conversation_dictation_active` answers. */
let dictationActive = false;

mock.module("@tauri-apps/api/core", () => ({
  invoke: async (command: string, args: unknown) => {
    calls.push({ command, args });
    if (command === "assistant_conversation_start")
      return backendStart ?? { session: ++nextSession, turn: 0 };
    if (command === "assistant_conversation_interrupt")
      return { session: nextSession, turn: ++turn };
    if (command === "assistant_conversation_audio") return audioTurn;
    if (command === "assistant_conversation_dictation_active")
      return dictationActive;
  },
}));
mock.module("@tauri-apps/api/event", () => ({
  emit: async () => {},
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
const { TURN_PAUSE_MS, VAD_SENSITIVITY, VAD_FRAME_MS } = await import(
  "./conversationPolicy"
);
let voice: ReturnType<typeof useVoiceConversation>;
let renderer: ReactTestRenderer;
/** The persisted pace the harness feeds the hook, and what it asked to save. */
let settingsPace: "quick" | "natural" | "patient" | null = null;
let paceWrites: string[] = [];
let settingsSensitivity: "low" | "normal" | "high" | null = null;
let settingsSpeakerOn: boolean | undefined;
/** The persisted voice volume, and every gain node the session created. */
let settingsVolume: number | undefined;
const gains: { gain: { value: number } }[] = [];
function Harness() {
  voice = useVoiceConversation({
    stopLocal() {},
    beginLocal: async () => {
      callbackBegins++;
    },
    pushLocal() {},
    endLocal() {},
    volume: settingsVolume,
    pace: settingsPace,
    onPaceChange: (pace) => paceWrites.push(pace),
    sensitivity: settingsSensitivity,
    onSensitivityChange() {},
    speakerOn: settingsSpeakerOn,
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
  dictationActive = false;
  backendStart = null;
  audioTurn = null;
  microphone = null;
  settingsPace = null;
  paceWrites = [];
  settingsSensitivity = null;
  settingsSpeakerOn = undefined;
  settingsVolume = undefined;
  gains.length = 0;
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
      const node = { gain: { value: 1 }, connect() {} };
      gains.push(node);
      return node;
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
/** Feed the detector `count` frames, each speech or not. */
const frames = async (count: number, speech: (index: number) => boolean) => {
  await act(async () => {
    for (let i = 0; i < count; i++)
      vadOptions.onFrameProcessed(
        {
          isSpeech: speech(i) ? 0.95 : 0.05,
          notSpeech: speech(i) ? 0.05 : 0.95,
        },
        new Float32Array(512),
      );
  });
};
const commandCount = (command: string) =>
  calls.filter((call) => call.command === command).length;
/** Get the assistant talking: one answered turn whose reply is playing. */
const replyPlaying = async () => {
  await act(async () => voice.start());
  await speak();
  await emit("assistant-conversation-audio-begin", {
    ticket: { session: 1, turn: 1 },
    epoch: 7,
  });
  await emit("assistant-conversation-audio", {
    ticket: { session: 1, turn: 1 },
    epoch: 7,
    audio: "AAAAAA==",
  });
  // Let the queued decode run.
  await act(async () => {});
  expect(voice.phase).toBe("speaking");
};

describe("hands-free session lifecycle", () => {
  /**
   * The pace is a persisted setting, so the hook must read it rather than own
   * it: it used to live in local state and reset to Natural on every restart
   * and every panel reload, which meant a user who needed Patient re-set it
   * every session.
   */
  test("the saved pace decides the pause the VAD waits for", async () => {
    settingsPace = "patient";
    await act(async () => renderer.update(<Harness />));
    await act(async () => voice.start());
    expect(voice.pace).toBe("patient");
    expect(vadOptions.redemptionMs).toBe(TURN_PAUSE_MS.patient);
  });

  test("with nothing saved the pace falls back to Natural", async () => {
    await act(async () => voice.start());
    expect(voice.pace).toBe("natural");
    expect(vadOptions.redemptionMs).toBe(TURN_PAUSE_MS.natural);
  });

  test("changing the pace is handed to the caller to persist", async () => {
    await act(async () => voice.start());
    await act(async () => voice.setPace("quick"));
    expect(paceWrites).toEqual(["quick"]);
  });

  /** The voice-volume slider should be audible in the call it is moved in. */
  test("the saved volume applies to a call already in progress", async () => {
    settingsVolume = 0.5;
    await act(async () => renderer.update(<Harness />));
    await act(async () => voice.start());
    expect(gains.at(-1)?.gain.value).toBeCloseTo(0.5);
    settingsVolume = 0;
    await act(async () => renderer.update(<Harness />));
    expect(gains.at(-1)?.gain.value).toBe(0);
  });

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
  /**
   * The reported bug. On Normal the detector only starts a segment once a frame
   * scores 0.75, and a soft first word never gets there — so everything before
   * the frame that did was lost past a fixed 320 ms. The lead-in puts it back.
   */
  test("a quiet first word before the detector's decision reaches the turn", async () => {
    const { SPEECH_LEAD } = await import("./conversationPolicy");
    await act(async () => voice.start());
    const probability = async (count: number, isSpeech: number) => {
      await act(async () => {
        for (let i = 0; i < count; i++)
          vadOptions.onFrameProcessed(
            { isSpeech, notSpeech: 1 - isSpeech },
            new Float32Array(512).fill(0.01),
          );
      });
    };
    await probability(20, 0.02); // room tone
    await probability(25, 0.4); // "hey, can you" — under Normal's threshold
    await probability(1, 0.9); // the frame the detector starts on
    await act(async () => vadOptions.onSpeechStart());
    await act(async () => vadOptions.onSpeechRealStart());
    await act(async () => vadOptions.onSpeechEnd(new Float32Array(16_000)));
    const sent = calls.filter(
      (call) => call.command === "assistant_conversation_audio",
    );
    expect(sent).toHaveLength(1);
    const samples = (sent[0].args as ArrayBuffer).byteLength / 4;
    const margin = Math.round(SPEECH_LEAD.marginMs / VAD_FRAME_MS);
    // All 25 quiet frames, the margin before them, then the library's audio.
    expect(samples).toBe((25 + margin) * 512 + 16_000);
    expect(vadOptions.preSpeechPadMs).toBe(0);
  });
  test("a lead-in never repeats audio from the utterance before it", async () => {
    await act(async () => voice.start());
    await frames(30, () => true);
    await act(async () => vadOptions.onSpeechStart());
    await act(async () => vadOptions.onSpeechRealStart());
    await act(async () => vadOptions.onSpeechEnd(new Float32Array(16_000)));
    // Straight into the next utterance: nothing heard before it remains.
    await frames(1, () => true);
    await act(async () => vadOptions.onSpeechStart());
    await act(async () => vadOptions.onSpeechRealStart());
    await act(async () => vadOptions.onSpeechEnd(new Float32Array(16_000)));
    const sent = calls.filter(
      (call) => call.command === "assistant_conversation_audio",
    );
    expect((sent.at(-1)?.args as ArrayBuffer).byteLength / 4).toBe(16_000);
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
  /**
   * Mute used to mean both directions at once, so muting your microphone to
   * stop the assistant hearing the room also cut off the answer it was already
   * reading out. Mute is the input side alone; the speaker is the output side.
   */
  test("mute stops capture but the reply is still spoken", async () => {
    const pending = deferred<unknown>();
    audioTurn = pending.promise;
    await act(async () => voice.start());
    await speak();
    await act(async () => voice.toggleMute());
    expect(tracks.every((track) => track.stopped)).toBe(true);
    expect(voice.muted).toBe(true);
    expect(voice.speakerOff).toBe(false);
    // The turn in flight is untouched, so the phase still reports what the
    // assistant is doing rather than masking it with "muted".
    expect(voice.phase).toBe("responding");
    expect(
      calls.filter(
        (call) => call.command === "assistant_conversation_interrupt",
      ).length,
    ).toBe(1);
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
    expect(callbackBegins).toBe(1);
    expect(sourceStarts).toBe(1);
    // Speech that slips through while muted is still rejected.
    await speak();
    expect(
      calls.filter((call) => call.command === "assistant_conversation_audio")
        .length,
    ).toBe(1);
  });
  /**
   * The speaker switch is output only. It replaced a "sound off" switch that
   * also closed the microphone, which made the control you reach for in a
   * shared room stop the call from hearing you as well.
   */
  test("speaker off stops the reading but keeps the microphone and the reply", async () => {
    const pending = deferred<unknown>();
    audioTurn = pending.promise;
    await act(async () => voice.start());
    await speak();
    await act(async () => voice.toggleSpeaker());
    expect(voice.speakerOff).toBe(true);
    expect(
      calls.some(
        (call) =>
          call.command === "assistant_conversation_set_speaker" &&
          (call.args as { on: boolean }).on === false,
      ),
    ).toBe(true);
    // The reply is not cancelled: it carries on as text.
    expect(commandCount("assistant_conversation_interrupt")).toBe(1);
    expect(voice.phase).toBe("responding");
    // And nothing more is played.
    await emit("assistant-conversation-audio", {
      ticket: { session: 1, turn: 1 },
      epoch: 7,
      audio: "AAAAAA==",
    });
    await act(async () => pending.resolve(undefined));
    expect(sourceStarts).toBe(0);
    // The microphone never closed.
    expect(tracks.every((track) => !track.stopped)).toBe(true);
    await speak();
    expect(commandCount("assistant_conversation_audio")).toBe(2);
  });
  test("a call opened with spoken replies off starts with its speaker off", async () => {
    settingsSpeakerOn = false;
    await act(async () => renderer.update(<Harness />));
    await act(async () => voice.start());
    expect(voice.speakerOff).toBe(true);
    const push = calls.find(
      (call) => call.command === "assistant_conversation_set_speaker",
    );
    expect(push?.args).toEqual({ session: 1, on: false });
  });
  /**
   * The echo guard. The assistant's own voice leaks back through the speakers,
   * and it used to start a turn — which cut the answer off mid-sentence, over
   * and over, until the call felt like it never finished anything.
   */
  test("patchy speech over a playing reply is dropped, not answered", async () => {
    await replyPlaying();
    await act(async () => vadOptions.onSpeechRealStart());
    // Still talking: nothing has interrupted it yet.
    expect(voice.phase).toBe("speaking");
    expect(commandCount("assistant_conversation_interrupt")).toBe(1);
    await frames(40, (i) => i % 3 === 0);
    expect(commandCount("assistant_conversation_interrupt")).toBe(1);
    await act(async () => vadOptions.onSpeechEnd(new Float32Array(16_000)));
    expect(commandCount("assistant_conversation_audio")).toBe(1);
    expect(voice.phase).toBe("speaking");
  });
  test("sustained speech over a playing reply interrupts it", async () => {
    await replyPlaying();
    await act(async () => vadOptions.onSpeechRealStart());
    const window = Math.round(VAD_SENSITIVITY.normal.bargeInMs / VAD_FRAME_MS);
    await frames(window - 1, () => true);
    expect(commandCount("assistant_conversation_interrupt")).toBe(1);
    await frames(1, () => true);
    expect(commandCount("assistant_conversation_interrupt")).toBe(2);
    expect(voice.phase).toBe("hearing");
    await act(async () => vadOptions.onSpeechEnd(new Float32Array(16_000)));
    expect(commandCount("assistant_conversation_audio")).toBe(2);
  });
  test("speech into silence starts a turn without waiting", async () => {
    await act(async () => voice.start());
    await act(async () => vadOptions.onSpeechRealStart());
    expect(voice.phase).toBe("hearing");
    expect(commandCount("assistant_conversation_interrupt")).toBe(1);
  });
  test("the saved sensitivity decides how readily speech is detected", async () => {
    settingsSensitivity = "low";
    await act(async () => renderer.update(<Harness />));
    await act(async () => voice.start());
    expect(voice.sensitivity).toBe("low");
    expect(vadOptions.positiveSpeechThreshold).toBe(
      VAD_SENSITIVITY.low.positiveSpeechThreshold,
    );
    expect(vadOptions.minSpeechMs).toBe(VAD_SENSITIVITY.low.minSpeechMs);
  });
  test("a typed message is its own turn and cuts a spoken reply off", async () => {
    await replyPlaying();
    let sent!: boolean;
    await act(async () => {
      sent = await voice.sendText("  and in Nepali?  ");
    });
    expect(sent).toBe(true);
    expect(commandCount("assistant_conversation_interrupt")).toBe(2);
    const text = calls.find(
      (call) => call.command === "assistant_conversation_text",
    );
    expect(text?.args).toEqual({ session: 1, turn: 2, text: "and in Nepali?" });
    expect(voice.phase).toBe("listening");
  });
  test("speech in progress when a message is sent is not also sent", async () => {
    await act(async () => voice.start());
    await act(async () => vadOptions.onSpeechRealStart());
    await act(async () => {
      await voice.sendText("hello");
    });
    await act(async () => vadOptions.onSpeechEnd(new Float32Array(16_000)));
    expect(commandCount("assistant_conversation_audio")).toBe(0);
    expect(commandCount("assistant_conversation_text")).toBe(1);
  });
  test("keyboard noise while a message is written does not start a turn", async () => {
    await replyPlaying();
    await act(async () => voice.setComposing(true));
    await speak();
    expect(commandCount("assistant_conversation_interrupt")).toBe(1);
    expect(commandCount("assistant_conversation_audio")).toBe(1);
    expect(voice.phase).toBe("speaking");
  });
  test("opening a saved conversation keeps the call and drops what was in flight", async () => {
    await replyPlaying();
    let opened!: boolean;
    await act(async () => {
      opened = await voice.loadConversation(12);
    });
    expect(opened).toBe(true);
    expect(
      calls.find((call) => call.command === "assistant_conversation_load")
        ?.args,
    ).toEqual({ session: 1, id: 12 });
    expect(voice.open).toBe(true);
    expect(voice.phase).toBe("listening");
    expect(tracks.every((track) => !track.stopped)).toBe(true);
    // A reply from the old conversation cannot start playing afterwards.
    await emit("assistant-conversation-audio", {
      ticket: { session: 1, turn: 1 },
      epoch: 8,
      audio: "AAAAAA==",
    });
    expect(sourceStarts).toBe(1);
  });
  test("stopping the reply keeps the call listening", async () => {
    await replyPlaying();
    await act(async () => voice.stopReply());
    expect(commandCount("assistant_conversation_interrupt")).toBe(2);
    expect(voice.phase).toBe("listening");
    expect(voice.open).toBe(true);
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
  test("an unrelated panel event keeps the call listening", async () => {
    await act(async () => voice.start());
    await emit("assistant-settings-changed", null);
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
  test("hiding a failed call clears the error before the next quick ask", async () => {
    microphone = Promise.reject(
      new DOMException("Permission denied", "NotAllowedError"),
    );
    await act(async () => voice.start());
    expect(voice.phase).toBe("error");
    expect(voice.open).toBe(true);
    await emit("assistant-panel-hidden", null);
    expect(voice.open).toBe(false);
    expect(voice.error).toBe(null);
  });
  test("a quick ask cancels pending call setup and releases the late microphone", async () => {
    const mic = deferred<MediaStream>();
    microphone = mic.promise;
    let opening!: Promise<void>;
    await act(async () => {
      opening = voice.start();
    });
    await emit("assistant-quick-ask", null);
    await act(async () => {
      mic.resolve(stream());
      await opening;
    });
    expect(voice.open).toBe(false);
    expect(tracks.every((track) => track.stopped)).toBe(true);
    expect(
      calls.some((call) => call.command === "assistant_conversation_start"),
    ).toBe(false);
  });
  test("hiding a live call releases its microphone", async () => {
    await act(async () => voice.start());
    await emit("assistant-panel-hidden", null);
    expect(voice.open).toBe(false);
    expect(tracks.every((track) => track.stopped)).toBe(true);
  });
  test("a restart waits for a late backend start to be closed", async () => {
    const pending = deferred<unknown>();
    backendStart = pending.promise;
    let first!: Promise<void>;
    let second!: Promise<void>;
    await act(async () => {
      first = voice.start();
    });
    await act(async () => {
      second = voice.start();
    });
    expect(
      calls.filter((call) => call.command === "assistant_conversation_start"),
    ).toHaveLength(1);
    backendStart = null;
    await act(async () => {
      pending.resolve({ session: 42, turn: 0 });
      await first;
      await second;
    });
    expect(
      calls
        .filter((call) =>
          [
            "assistant_conversation_start",
            "assistant_conversation_end",
          ].includes(call.command),
        )
        .map((call) => call.command),
    ).toEqual([
      "assistant_conversation_start",
      "assistant_conversation_end",
      "assistant_conversation_start",
    ]);
    expect(voice.phase).toBe("listening");
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
  test("an ended event releases a session whose ticket has not arrived yet", async () => {
    // Rust treats a call as active from the moment it issues the ticket, which is
    // before the id reaches this hook. Something that hangs up in that window —
    // the call hotkey, the assistant shortcut taking the microphone back — emits
    // `ended` for a session this side cannot name yet. Ignoring it left the VAD
    // holding the microphone for a call that no longer existed.
    const pending = deferred<unknown>();
    backendStart = pending.promise;
    let opening!: Promise<void>;
    await act(async () => {
      opening = voice.start();
    });
    await emit("assistant-conversation-ended", 7);
    expect(voice.open).toBe(false);
    expect(voice.phase).toBe("off");
    // And the ticket that lands afterwards is closed rather than left running.
    await act(async () => {
      pending.resolve({ session: 7, turn: 0 });
      await opening;
    });
    expect(tracks.every((track) => track.stopped)).toBe(true);
    expect(
      calls.some(
        (call) =>
          call.command === "assistant_conversation_end" &&
          (call.args as { session: number }).session === 7,
      ),
    ).toBe(true);
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

/**
 * Dictating during a call used to hang it up, and hanging up threw the call's
 * conversation away. Now dictation holds the call: it stops listening while
 * the dictation records and carries on afterwards.
 */
describe("dictation beside a call", () => {
  const hold = async (held: boolean) => {
    await emit("assistant-conversation-dictation", held);
    // Let the VAD's pause or start settle.
    await act(async () => {});
  };

  test("a dictation pauses the call's microphone and the call carries on", async () => {
    await act(async () => voice.start());
    await hold(true);
    expect(voice.open).toBe(true);
    expect(voice.phase).toBe("held");
    expect(tracks.every((track) => track.stopped)).toBe(true);
    // What is dictated is not answered by the call.
    await speak();
    expect(commandCount("assistant_conversation_audio")).toBe(0);

    await hold(false);
    expect(voice.phase).toBe("listening");
    expect(tracks.at(-1)?.stopped).toBe(false);
    await speak();
    expect(commandCount("assistant_conversation_audio")).toBe(1);
    // Nothing hung the call up along the way.
    expect(commandCount("assistant_conversation_end")).toBe(0);
  });

  test("a reply already playing keeps playing while dictation holds the call", async () => {
    await replyPlaying();
    await hold(true);
    expect(voice.phase).toBe("speaking");
    expect(commandCount("assistant_conversation_interrupt")).toBe(1);
    expect(sourceStarts).toBe(1);
  });

  test("speech cut off by a dictation is not sent", async () => {
    await act(async () => voice.start());
    await act(async () => vadOptions.onSpeechRealStart());
    expect(voice.phase).toBe("hearing");
    await hold(true);
    await act(async () => vadOptions.onSpeechEnd(new Float32Array(16_000)));
    expect(commandCount("assistant_conversation_audio")).toBe(0);
    await hold(false);
    await speak();
    expect(commandCount("assistant_conversation_audio")).toBe(1);
  });

  test("a call started during a dictation begins held", async () => {
    dictationActive = true;
    await act(async () => voice.start());
    await act(async () => {});
    expect(voice.open).toBe(true);
    expect(voice.phase).toBe("held");
    expect(tracks.every((track) => track.stopped)).toBe(true);
    await hold(false);
    expect(voice.phase).toBe("listening");
    expect(tracks.at(-1)?.stopped).toBe(false);
  });

  test("the hold and mute are separate switches", async () => {
    await act(async () => voice.start());
    await act(async () => voice.toggleMute());
    await hold(true);
    await hold(false);
    // Still muted: the dictation ending does not unmute the call.
    expect(voice.muted).toBe(true);
    expect(voice.phase).toBe("muted");
    expect(tracks.every((track) => track.stopped)).toBe(true);
    await act(async () => voice.toggleMute());
    expect(voice.phase).toBe("listening");
    expect(tracks.at(-1)?.stopped).toBe(false);
  });

  test("unmuting during a dictation keeps the microphone closed until it ends", async () => {
    await act(async () => voice.start());
    await act(async () => voice.toggleMute());
    await hold(true);
    await act(async () => voice.toggleMute());
    expect(voice.muted).toBe(false);
    expect(voice.phase).toBe("held");
    expect(tracks.every((track) => track.stopped)).toBe(true);
    await hold(false);
    expect(voice.phase).toBe("listening");
    expect(tracks.at(-1)?.stopped).toBe(false);
  });
});
