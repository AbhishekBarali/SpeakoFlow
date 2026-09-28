import { describe, expect, test } from "bun:test";
import {
  MAX_MESSAGE_CHARS,
  canSend,
  describeSystem,
  looksLikeEmail,
  parseDraft,
} from "./feedbackStore";

describe("canSend", () => {
  test("needs a few words, and no more than the limit", () => {
    expect(canSend("  ok ", "")).toBe(false);
    expect(canSend("the overlay vanished", "")).toBe(true);
    expect(canSend("x".repeat(MAX_MESSAGE_CHARS + 1), "")).toBe(false);
  });

  test("counts characters, not UTF-16 units", () => {
    // 5,000 emoji are 10,000 UTF-16 units but 5,000 characters.
    expect(canSend("🎙".repeat(MAX_MESSAGE_CHARS), "")).toBe(true);
  });

  test("an email is optional but must look like one when given", () => {
    expect(canSend("dark mode please", "   ")).toBe(true);
    expect(canSend("dark mode please", "me@example.com")).toBe(true);
    expect(canSend("dark mode please", "me@example")).toBe(false);
  });
});

describe("looksLikeEmail", () => {
  test("matches the backend's rules", () => {
    for (const ok of ["a@b.co", "first.last+tag@mail.example.com"]) {
      expect(looksLikeEmail(ok)).toBe(true);
    }
    for (const bad of [
      "a@b",
      "a@@b.co",
      "@b.co",
      "a b@c.co",
      "a@.co",
      "a@b.co.",
      "a@b..co",
    ]) {
      expect(looksLikeEmail(bad)).toBe(false);
    }
  });
});

describe("parseDraft", () => {
  test("restores a saved draft", () => {
    expect(parseDraft('{"kind":"idea","message":"hi"}')).toEqual({
      kind: "idea",
      message: "hi",
    });
  });

  test("survives anything else in storage", () => {
    expect(parseDraft(null)).toEqual({ kind: "bug", message: "" });
    expect(parseDraft("not json")).toEqual({ kind: "bug", message: "" });
    expect(parseDraft('{"kind":"rant","message":7}')).toEqual({
      kind: "bug",
      message: "",
    });
  });
});

test("describeSystem is one readable line", () => {
  expect(
    describeSystem({
      app_version: "1.5.0",
      os: "Windows 10.0.26100",
      arch: "x86_64",
      install: "nsis",
    }),
  ).toBe("SpeakoFlow 1.5.0 · Windows 10.0.26100 · x86_64 · nsis");
});
