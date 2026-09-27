import { describe, expect, it } from "bun:test";
import {
  matchDeviceByName,
  SPEECH_LEAD,
  speechLeadFrames,
  VAD_FRAME_MS,
} from "./conversationPolicy";

const ms = (value: number) => Math.round(value / VAD_FRAME_MS);
const MIN = ms(SPEECH_LEAD.minMs);
const MAX = ms(SPEECH_LEAD.maxMs);
const MARGIN = ms(SPEECH_LEAD.marginMs);
const repeat = (count: number, value: number | null) =>
  Array<number | null>(count).fill(value);

describe("speechLeadFrames", () => {
  it("keeps the old fixed pre-roll when speech starts sharply", () => {
    const history = repeat(60, 0.02);
    expect(speechLeadFrames(history)).toBe(MIN);
  });

  /**
   * The reported bug: a soft first word that never reaches the start threshold
   * was cut off, because it began more than 320 ms before the frame that did.
   */
  it("reaches back to a quiet first word", () => {
    const history = [...repeat(20, 0.02), ...repeat(25, 0.4)];
    expect(speechLeadFrames(history)).toBe(25 + MARGIN);
  });

  it("steps over the pause between two words", () => {
    // "hey" … short pause … "can you" (still under the start threshold)
    const history = [
      ...repeat(15, 0.02),
      ...repeat(8, 0.5),
      ...repeat(6, 0.05),
      ...repeat(10, 0.45),
    ];
    expect(speechLeadFrames(history)).toBe(24 + MARGIN);
  });

  it("does not reach across a real silence", () => {
    const history = [
      ...repeat(10, 0.6),
      ...repeat(20, 0.02),
      ...repeat(12, 0.4),
    ];
    expect(speechLeadFrames(history)).toBe(12 + MARGIN);
  });

  it("is capped", () => {
    expect(speechLeadFrames(repeat(200, 0.5))).toBe(MAX);
  });

  /** A leaked reply scores as speech too; it must not open the user's turn. */
  it("stops at the assistant's own playback beyond the minimum", () => {
    const history = [...repeat(30, null), ...repeat(20, 0.5)];
    expect(speechLeadFrames(history)).toBe(20);
    // …but never gives less than the old pre-roll, which barge-in relied on.
    expect(speechLeadFrames([...repeat(30, null), 0.5])).toBe(MIN);
  });

  it("never asks for more history than exists", () => {
    expect(speechLeadFrames([])).toBe(0);
    expect(speechLeadFrames([0.5, 0.5])).toBe(2);
  });
});

const device = (
  kind: MediaDeviceKind,
  label: string,
  deviceId = label,
): MediaDeviceInfo =>
  ({ kind, label, deviceId, groupId: "" }) as MediaDeviceInfo;

describe("matchDeviceByName", () => {
  it("prefers an exact label", () => {
    const devices = [
      device("audioinput", "Yeti Stereo Microphone"),
      device("audioinput", "Default - Yeti Stereo Microphone"),
    ];
    expect(
      matchDeviceByName(devices, "audioinput", "Yeti Stereo Microphone")?.label,
    ).toBe("Yeti Stereo Microphone");
  });

  it("sees through the decoration Chromium adds to a label", () => {
    const devices = [device("audioinput", "Default - Yeti Stereo Microphone")];
    expect(
      matchDeviceByName(devices, "audioinput", "Yeti Stereo Microphone")?.label,
    ).toBe("Default - Yeti Stereo Microphone");
  });

  it("ignores devices of the wrong kind", () => {
    const devices = [device("audiooutput", "Yeti Stereo Microphone")];
    expect(
      matchDeviceByName(devices, "audioinput", "Yeti Stereo Microphone"),
    ).toBeNull();
  });

  /**
   * Labels are empty until microphone permission is granted, and an empty label
   * matches every substring test — so without this it would return the first
   * unnamed device and record from the wrong microphone.
   */
  it("never matches an unnamed device", () => {
    const devices = [
      device("audioinput", "", "id-1"),
      device("audioinput", "  ", "id-2"),
    ];
    expect(matchDeviceByName(devices, "audioinput", "Yeti")).toBeNull();
  });

  /**
   * The stored name comes from the native audio engine, so on Linux it is not a
   * browser label at all. Answering "use the default" is what keeps the call
   * startable; guessing between two candidates would silently record from the
   * wrong one.
   */
  it("gives up rather than guessing", () => {
    const devices = [
      device("audioinput", "Microphone (USB Audio)"),
      device("audioinput", "Microphone (Realtek)"),
    ];
    expect(matchDeviceByName(devices, "audioinput", "Microphone")).toBeNull();
    expect(
      matchDeviceByName(devices, "audioinput", "sysdefault:CARD=PCH"),
    ).toBeNull();
  });
});
