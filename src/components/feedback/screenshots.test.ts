import { describe, expect, test } from "bun:test";
import {
  MAX_EDGE,
  TARGET_BYTES,
  chooseEncoding,
  fitWithin,
  formatBytes,
  type Encoder,
} from "./screenshots";

/**
 * A fake encoder whose output size is proportional to pixels × quality, like
 * a real lossy codec roughly is. `bytesPerPixel` sets how "busy" the image is.
 */
function fakeEncoder(
  bytesPerPixel: number,
  { webp = true }: { webp?: boolean } = {},
) {
  const calls: Array<[number, number, string, number]> = [];
  const encode: Encoder = async (width, height, type, quality) => {
    calls.push([width, height, type, quality]);
    const actual = type === "image/webp" && !webp ? "image/png" : type;
    const bytes = Math.round(width * height * bytesPerPixel * quality);
    return { type: actual, bytes, blob: new Blob([]) };
  };
  return { encode, calls };
}

describe("fitWithin", () => {
  test("leaves small images alone", () => {
    expect(fitWithin(1280, 720, MAX_EDGE)).toEqual({
      width: 1280,
      height: 720,
    });
  });

  test("caps the long edge and keeps the aspect ratio", () => {
    expect(fitWithin(3840, 2160, MAX_EDGE)).toEqual({
      width: 1920,
      height: 1080,
    });
    expect(fitWithin(1000, 4000, MAX_EDGE)).toEqual({
      width: 480,
      height: 1920,
    });
  });
});

describe("chooseEncoding", () => {
  test("a simple screenshot is done in one encode, at full quality", async () => {
    const { encode, calls } = fakeEncoder(0.05);
    const chosen = await chooseEncoding(1920, 1080, encode);
    expect(calls).toEqual([[1920, 1080, "image/webp", 0.85]]);
    expect(chosen.encoded.bytes).toBeLessThanOrEqual(TARGET_BYTES);
  });

  test("a 4K screen is halved before anything else", async () => {
    const { encode, calls } = fakeEncoder(0.05);
    const chosen = await chooseEncoding(3840, 2160, encode);
    expect(calls[0].slice(0, 2)).toEqual([1920, 1080]);
    expect([chosen.width, chosen.height]).toEqual([1920, 1080]);
  });

  test("a busy image lowers quality first, then size", async () => {
    // 1920×1080×0.35 is over target even at 0.65; 1536×864 at 0.85 is under.
    const { encode, calls } = fakeEncoder(0.35);
    const chosen = await chooseEncoding(1920, 1080, encode);
    expect(calls.slice(0, 3).map((c) => c[3])).toEqual([0.85, 0.75, 0.65]);
    expect(calls[3].slice(0, 2)).toEqual([1536, 864]);
    expect(chosen.encoded.bytes).toBeLessThanOrEqual(TARGET_BYTES);
  });

  test("never shrinks below legibility; returns the smallest attempt", async () => {
    const { encode, calls } = fakeEncoder(10);
    const chosen = await chooseEncoding(1920, 1080, encode);
    for (const [w, h] of calls) {
      expect(Math.max(w, h)).toBeGreaterThanOrEqual(640);
    }
    const smallest = Math.min(
      ...calls.map(([w, h, , q]) => Math.round(w * h * 10 * q)),
    );
    expect(chosen.encoded.bytes).toBe(smallest);
  });

  test("falls back to JPEG where the WebView cannot write WebP", async () => {
    const { encode, calls } = fakeEncoder(0.05, { webp: false });
    const chosen = await chooseEncoding(1600, 900, encode);
    expect(calls.map((c) => c[2])).toEqual(["image/webp", "image/jpeg"]);
    expect(chosen.encoded.type).toBe("image/jpeg");
    // The PNG it got back instead is never what gets sent.
    expect(calls[1][3]).toBe(0.85);
  });
});

test("formatBytes", () => {
  expect(formatBytes(900)).toBe("900 B");
  expect(formatBytes(184 * 1024)).toBe("184 KB");
  expect(formatBytes(1.25 * 1024 * 1024)).toBe("1.3 MB");
});
