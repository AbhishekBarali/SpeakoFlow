import { describe, expect, test } from "bun:test";
import { formatBytes, formatModelSize } from "./format";

describe("formatBytes", () => {
  test("megabytes below a gigabyte, gigabytes above", () => {
    expect(formatBytes(512 * 1024 * 1024)).toBe("512 MB");
    expect(formatBytes(24.5 * 1024 * 1024)).toBe("24.5 MB");
    expect(formatBytes(1.5 * 1024 * 1024 * 1024)).toBe("1.5 GB");
  });

  test("nothing, or nonsense, is zero rather than a broken label", () => {
    expect(formatBytes(0)).toBe("0 MB");
    expect(formatBytes(Number.NaN)).toBe("0 MB");
  });
});

describe("formatModelSize", () => {
  test("reads a catalog size in megabytes", () => {
    expect(formatModelSize(697)).toBe("697 MB");
    expect(formatModelSize(2048)).toBe("2.0 GB");
  });
});
