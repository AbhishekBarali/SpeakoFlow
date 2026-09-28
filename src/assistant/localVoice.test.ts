import { describe, expect, test } from "bun:test";
import { audioLooksBroken, parseKokoroDevice } from "./localVoice";

const RATE = 24_000;

/** A voice-like signal: a 180 Hz tone with a slow amplitude envelope, the
 *  kind of low-crossing, mid-level waveform real speech produces. */
const voiced = (seconds: number) =>
  Float32Array.from({ length: Math.round(RATE * seconds) }, (_, i) => {
    const t = i / RATE;
    const envelope = 0.5 + 0.5 * Math.sin(2 * Math.PI * 3 * t);
    return 0.12 * envelope * Math.sin(2 * Math.PI * 180 * t);
  });

/** Deterministic white noise (xorshift32, exact in 32-bit integer math). */
const noise = (seconds: number, amplitude = 0.3) => {
  let x = 2463534242;
  return Float32Array.from({ length: Math.round(RATE * seconds) }, () => {
    x ^= x << 13;
    x ^= x >>> 17;
    x ^= x << 5;
    return amplitude * (((x >>> 0) / 4294967296) * 2 - 1);
  });
};

describe("audioLooksBroken", () => {
  test("a clip shorter than half a second is not judged on its statistics", () => {
    expect(audioLooksBroken(noise(0.3), RATE)).toBeNull();
    expect(audioLooksBroken(voiced(0.2), RATE)).toBeNull();
    expect(audioLooksBroken(new Float32Array(RATE * 0.3), RATE)).toBeNull();
  });

  test("a clip too short to hold a word is never judged", () => {
    const blip = voiced(0.05);
    blip[10] = Number.NaN;
    expect(audioLooksBroken(blip, RATE)).toBeNull();
  });

  test("a voice-like clip passes", () => {
    expect(audioLooksBroken(voiced(1.5), RATE)).toBe(false);
  });

  test("quiet hiss between words does not read as noise", () => {
    const speech = voiced(1.5);
    const hiss = noise(1.5, 0.004);
    const clip = speech.map((value, i) => value + hiss[i]);
    expect(audioLooksBroken(clip, RATE)).toBe(false);
  });

  test("noise instead of speech is flagged", () => {
    expect(audioLooksBroken(noise(1.5), RATE)).toBe(true);
  });

  test("silence where a sentence should be is flagged", () => {
    expect(audioLooksBroken(new Float32Array(RATE), RATE)).toBe(true);
    // Barely audible, far below the quietest real speech (RMS 0.06).
    expect(
      audioLooksBroken(
        voiced(1).map((v) => v / 20),
        RATE,
      ),
    ).toBe(true);
  });

  test("non-finite samples are flagged", () => {
    const clip = voiced(1);
    clip[100] = Number.NaN;
    expect(audioLooksBroken(clip, RATE)).toBe(true);
    const short = voiced(0.3);
    short[100] = Number.POSITIVE_INFINITY;
    expect(audioLooksBroken(short, RATE)).toBe(true);
  });

  test("a clip pinned at full scale is flagged, even a short one", () => {
    const pin = (clip: Float32Array) =>
      clip.map((v) => (v >= 0 ? 1 : -1) * 0.995);
    expect(audioLooksBroken(pin(voiced(1)), RATE)).toBe(true);
    expect(audioLooksBroken(pin(voiced(0.3)), RATE)).toBe(true);
  });
});

describe("parseKokoroDevice", () => {
  test("known values pass through and anything else is Automatic", () => {
    expect(parseKokoroDevice("gpu")).toBe("gpu");
    expect(parseKokoroDevice("cpu")).toBe("cpu");
    expect(parseKokoroDevice("auto")).toBe("auto");
    expect(parseKokoroDevice(undefined)).toBe("auto");
    expect(parseKokoroDevice("q8-cpu")).toBe("auto");
  });
});
