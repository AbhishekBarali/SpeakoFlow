import { describe, expect, test } from "bun:test";
import { classifyCheckError, percentOf, safeExternalHref } from "./updateLogic";

describe("classifyCheckError", () => {
  test("a release with no manifest means there is nothing newer", () => {
    // The exact text tauri-plugin-updater 2.10 produces for a 404.
    expect(
      classifyCheckError(
        "Could not fetch a valid release JSON from the remote",
      ),
    ).toBe("noUpdate");
  });

  test("a release with no build for this machine is reported as such", () => {
    expect(
      classifyCheckError(
        new Error(
          'None of the fallback platforms `["linux-aarch64-deb", "linux-aarch64"]` were found in the response `platforms` object',
        ),
      ),
    ).toBe("noBuild");
  });

  test("anything else is a real failure", () => {
    expect(classifyCheckError("error sending request for url")).toBe("failed");
    expect(classifyCheckError(undefined)).toBe("failed");
  });
});

describe("percentOf", () => {
  test("is bounded and whole", () => {
    expect(percentOf(50, 200)).toBe(25);
    expect(percentOf(199, 200)).toBe(99);
    expect(percentOf(300, 200)).toBe(100);
  });

  test("is unknown without a size", () => {
    expect(percentOf(10, null)).toBeNull();
    expect(percentOf(10, 0)).toBeNull();
  });
});

describe("safeExternalHref", () => {
  test("only https leaves the app", () => {
    expect(safeExternalHref("https://github.com/x/y/pull/1")).toBe(
      "https://github.com/x/y/pull/1",
    );
    expect(safeExternalHref("javascript:alert(1)")).toBeNull();
    expect(safeExternalHref("http://example.com")).toBeNull();
    expect(safeExternalHref("/relative")).toBeNull();
    expect(safeExternalHref(undefined)).toBeNull();
  });
});
