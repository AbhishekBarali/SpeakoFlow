import { describe, expect, test } from "bun:test";
import { screenStepFor } from "./screenScale";

describe("screenStepFor", () => {
  test("small laptop screens get the compact step", () => {
    expect(screenStepFor(1366, 768)).toBe("compact");
    expect(screenStepFor(1536, 845)).toBe("compact");
  });

  test("1080p stays on the default step", () => {
    expect(screenStepFor(1920, 1080)).toBe("default");
    expect(screenStepFor(1920, 1200)).toBe("default");
  });

  test("1440p at 100% and 4K at 150% share the large step", () => {
    expect(screenStepFor(2560, 1440)).toBe("large");
    expect(screenStepFor(3440, 1440)).toBe("large");
  });

  test("4K at 100% gets the huge step", () => {
    expect(screenStepFor(3840, 2160)).toBe("huge");
  });

  test("portrait monitors use the shorter edge", () => {
    expect(screenStepFor(1440, 2560)).toBe("large");
  });

  test("nonsense input falls back to default", () => {
    expect(screenStepFor(0, 0)).toBe("default");
    expect(screenStepFor(Number.NaN, 1080)).toBe("default");
  });
});
