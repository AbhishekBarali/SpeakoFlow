import { describe, expect, test } from "bun:test";
import { ConversationAudio } from "./conversationAudio";
import { sameVoiceTicket, VoiceTurnGate } from "./conversationPolicy";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
function audioContext(
  decode: () => Promise<{ duration: number }> = async () => ({ duration: 2 }),
) {
  const sources: {
    at?: number;
    stopped: boolean;
    onended: (() => void) | null;
    start: (at: number) => void;
    stop: () => void;
    connect: () => void;
    disconnect: () => void;
  }[] = [];
  const context = {
    currentTime: 5,
    destination: {},
    decodeAudioData: decode,
    createGain: () => ({ gain: { value: 1 }, connect() {} }),
    createBufferSource: () => {
      const source = {
        stopped: false,
        onended: null,
        start(at: number) {
          source.at = at;
        },
        stop() {
          source.stopped = true;
        },
        connect() {},
        disconnect() {},
      };
      sources.push(source);
      return source;
    },
  };
  return { context: context as unknown as AudioContext, sources };
}
describe("conversation playback cancellation", () => {
  test("a decode finishing after interruption cannot start audio", async () => {
    const pending = deferred<{ duration: number }>();
    const { context, sources } = audioContext(() => pending.promise);
    const player = new ConversationAudio(context, () => {});
    player.begin(1);
    const queued = player.enqueue(new ArrayBuffer(4), 1);
    await Promise.resolve();
    player.stop();
    pending.resolve({ duration: 2 });
    await queued;
    expect(sources.length).toBe(0);
  });
  test("chunks are scheduled gaplessly, and Stop clears every source", async () => {
    const { context, sources } = audioContext();
    const activity: boolean[] = [];
    const player = new ConversationAudio(context, (playing) =>
      activity.push(playing),
    );
    player.begin(4);
    await player.enqueue(new ArrayBuffer(4), 4);
    await player.enqueue(new ArrayBuffer(4), 4);
    expect(sources[1].at).toBe(sources[0].at! + 2);
    player.stop();
    expect(sources.every((source) => source.stopped)).toBe(true);
    expect(activity[activity.length - 1]).toBe(false);
    await player.enqueue(new ArrayBuffer(4), 4);
    expect(sources.length).toBe(2);
  });
  test("old decode cannot block a new reply", async () => {
    const pending = deferred<{ duration: number }>();
    let first = true;
    const { context, sources } = audioContext(() => {
      if (first) {
        first = false;
        return pending.promise;
      }
      return Promise.resolve({ duration: 1 });
    });
    const player = new ConversationAudio(context, () => {});
    player.begin(1);
    const old = player.enqueue(new ArrayBuffer(4), 1);
    await Promise.resolve();
    player.begin(2);
    await player.enqueue(new ArrayBuffer(4), 2);
    expect(sources.length).toBe(1);
    pending.resolve({ duration: 4 });
    await old;
    expect(sources.length).toBe(1);
  });
  test("session/turn identity rejects stale audio and STT", () => {
    expect(
      sameVoiceTicket({ session: 2, turn: 3 }, { session: 1, turn: 3 }),
    ).toBe(false);
    expect(
      sameVoiceTicket({ session: 2, turn: 3 }, { session: 2, turn: 2 }),
    ).toBe(false);
    expect(sameVoiceTicket(null, { session: 2, turn: 3 })).toBe(false);
    const gate = new VoiceTurnGate();
    const old = gate.next();
    const current = gate.next();
    expect(gate.accepts(old)).toBe(false);
    expect(gate.accepts(current)).toBe(true);
  });
});
