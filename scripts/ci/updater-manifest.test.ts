import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  buildManifest,
  classifyDmg,
  classifyUpdaterAsset,
  tagMatchesVersion,
  verifyUpdaterSignature,
} from "./updater-manifest";

const conf = JSON.parse(
  readFileSync(
    join(import.meta.dir, "../../src-tauri/tauri.conf.json"),
    "utf8",
  ),
) as { plugins: { updater: { pubkey: string; endpoints: string[] } } };
const PUBKEY = conf.plugins.updater.pubkey;

// Produced once with `tauri signer sign` and the real SpeakoFlow updater key
// over the 16 bytes "hello speakoflow". A known-good vector ties the verifier
// to the exact format the Tauri CLI writes, not to our reading of the spec.
const PAYLOAD = new TextEncoder().encode("hello speakoflow");
const SIGNATURE =
  "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTQkEzaUgvLzNSYzJ0dVM1emFGdGtKaGEzUVpHRU9YY253VWtmR0dyUUtQUFc2eDhsQzZCRGs1czVvTXY3NnAxdlVVeTdVSUZDd3BzbldpM2RLYkhLbHVwQzlZRVJVdEF3PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkwNTkxMjU5CWZpbGU6c2Ytc2lnbi10ZXN0LmJpbgpnQlRzTm9HZGtFcjVvNEVKay9WUlJKVWpVSUxBSktZRklQMGRzeDJwSUhlWEFydUNGODJzRlVpYVkyNDYvT3BlcTQ1SEU0R0R3eUFpZ2RHMTJmbDNBQT09Cg==";

// Upstream Handy's key, which this app shipped with until 1.5.0 and whose
// private half we never had.
const HANDY_PUBKEY =
  "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEJBQjcyMDk1MjA2NjAxRjkKUldUNUFXWWdsU0MzdXRRZi8zYzhqV2FaNUVDbDd2Rk5VM1IvWWowVXdmRFNKQ1BrMXF5RFFsLy8K";

describe("verifyUpdaterSignature", () => {
  test("accepts a signature made by the SpeakoFlow key", () => {
    expect(verifyUpdaterSignature(PAYLOAD, SIGNATURE, PUBKEY)).toEqual({
      ok: true,
    });
  });

  test("rejects a file that changed after signing (the Linux repack bug)", () => {
    const tampered = new TextEncoder().encode("hello speakoflox");
    const result = verifyUpdaterSignature(tampered, SIGNATURE, PUBKEY);
    expect(result.ok).toBe(false);
    expect(result.reason).toContain("does not match");
  });

  test("rejects a signature checked against a different key", () => {
    const result = verifyUpdaterSignature(PAYLOAD, SIGNATURE, HANDY_PUBKEY);
    expect(result.ok).toBe(false);
    expect(result.reason).toContain("BAB72095206601F9");
  });

  test("rejects an edited trusted comment", () => {
    const text = Buffer.from(SIGNATURE, "base64")
      .toString("utf8")
      .replace("timestamp:1790591259", "timestamp:1790591260");
    const forged = Buffer.from(text, "utf8").toString("base64");
    const result = verifyUpdaterSignature(PAYLOAD, forged, PUBKEY);
    expect(result.ok).toBe(false);
    expect(result.reason).toContain("trusted comment");
  });

  test("rejects garbage without throwing", () => {
    expect(verifyUpdaterSignature(PAYLOAD, "not a signature", PUBKEY).ok).toBe(
      false,
    );
  });
});

describe("the shipped config", () => {
  test("no longer trusts upstream Handy's updater key", () => {
    expect(PUBKEY).not.toBe(HANDY_PUBKEY);
  });

  test("polls this repository's latest release", () => {
    expect(conf.plugins.updater.endpoints).toEqual([
      "https://github.com/AbhishekBarali/SpeakoFlow/releases/latest/download/latest.json",
    ]);
  });
});

describe("classifyUpdaterAsset", () => {
  test.each([
    ["SpeakoFlow_1.5.0_x64-setup.exe", "windows", "x86_64", "nsis"],
    ["SpeakoFlow_1.5.0_arm64-setup.exe", "windows", "aarch64", "nsis"],
    ["SpeakoFlow_1.5.0_x64_en-US.msi", "windows", "x86_64", "msi"],
    ["SpeakoFlow_aarch64.app.tar.gz", "darwin", "aarch64", "app"],
    ["SpeakoFlow_x64.app.tar.gz", "darwin", "x86_64", "app"],
    ["SpeakoFlow_1.5.0_amd64.AppImage", "linux", "x86_64", "appimage"],
    ["SpeakoFlow_1.5.0_aarch64.AppImage", "linux", "aarch64", "appimage"],
    ["SpeakoFlow_1.5.0_amd64.deb", "linux", "x86_64", "deb"],
    ["SpeakoFlow_1.5.0_arm64.deb", "linux", "aarch64", "deb"],
    ["SpeakoFlow-1.5.0-1.x86_64.rpm", "linux", "x86_64", "rpm"],
  ])("%s → %s-%s-%s", (name, os, arch, bundle) => {
    expect(classifyUpdaterAsset(name)).toEqual({ os, arch, bundle } as never);
  });

  test.each([
    "SpeakoFlow_1.5.0_x64-setup.exe.sig",
    "SpeakoFlow_1.5.0_aarch64.dmg",
    "latest.json",
    "SpeakoFlow_1.5.0_amd64.AppImage.tar.gz",
  ])("%s is not an updater bundle", (name) => {
    expect(classifyUpdaterAsset(name)).toBeNull();
  });

  test("dmg is a download, not an update", () => {
    expect(classifyDmg("SpeakoFlow_1.5.0_aarch64.dmg")).toEqual({
      arch: "aarch64",
    });
    expect(classifyDmg("SpeakoFlow_1.5.0_x64.dmg")).toEqual({ arch: "x86_64" });
  });
});

// What a 1.5.0 release carries: the v1.4.0 asset list plus the .sig files a
// signed build now uploads beside each updater bundle.
const V140_ASSETS = [
  "SpeakoFlow_1.5.0_aarch64.AppImage",
  "SpeakoFlow_1.5.0_aarch64.AppImage.sig",
  "SpeakoFlow_1.5.0_aarch64.dmg",
  "SpeakoFlow_1.5.0_amd64.AppImage",
  "SpeakoFlow_1.5.0_amd64.AppImage.sig",
  "SpeakoFlow_1.5.0_amd64.deb",
  "SpeakoFlow_1.5.0_amd64.deb.sig",
  "SpeakoFlow_1.5.0_arm64.deb",
  "SpeakoFlow_1.5.0_arm64.deb.sig",
  "SpeakoFlow_1.5.0_x64-setup.exe",
  "SpeakoFlow_1.5.0_x64-setup.exe.sig",
  "SpeakoFlow_1.5.0_x64.dmg",
  "SpeakoFlow_1.5.0_x64_en-US.msi",
  "SpeakoFlow_1.5.0_x64_en-US.msi.sig",
  "SpeakoFlow_aarch64.app.tar.gz",
  "SpeakoFlow_aarch64.app.tar.gz.sig",
  "SpeakoFlow_x64.app.tar.gz",
  "SpeakoFlow_x64.app.tar.gz.sig",
];

function signaturesFor(names: string[]): Map<string, string> {
  return new Map(
    names
      .filter((name) => name.endsWith(".sig"))
      .map((sig) => [sig.slice(0, -4), `sig-of-${sig.slice(0, -4)}\n`]),
  );
}

describe("buildManifest", () => {
  const manifest = buildManifest({
    version: "1.5.0",
    notes: "What's new",
    pubDate: "2026-09-28T00:00:00.000Z",
    repo: "AbhishekBarali/SpeakoFlow",
    tag: "v1.5.0",
    assetNames: V140_ASSETS,
    signatures: signaturesFor(V140_ASSETS),
  });

  test("offers every platform the release builds, under the keys the updater looks up", () => {
    expect(Object.keys(manifest.platforms)).toEqual([
      "darwin-aarch64",
      "darwin-aarch64-app",
      "darwin-x86_64",
      "darwin-x86_64-app",
      "linux-aarch64",
      "linux-aarch64-appimage",
      "linux-aarch64-deb",
      "linux-x86_64",
      "linux-x86_64-appimage",
      "linux-x86_64-deb",
      "windows-x86_64",
      "windows-x86_64-msi",
      "windows-x86_64-nsis",
    ]);
  });

  test("an NSIS install and an MSI install each update with their own installer", () => {
    expect(manifest.platforms["windows-x86_64-nsis"].url).toEndWith(
      "SpeakoFlow_1.5.0_x64-setup.exe",
    );
    expect(manifest.platforms["windows-x86_64-msi"].url).toEndWith(
      "SpeakoFlow_1.5.0_x64_en-US.msi",
    );
    // An install with no recorded bundle type gets the NSIS installer.
    expect(manifest.platforms["windows-x86_64"]).toEqual(
      manifest.platforms["windows-x86_64-nsis"],
    );
  });

  test("links pin the tag, so a draft's untagged URL never leaks in", () => {
    expect(manifest.platforms["linux-x86_64-deb"]).toEqual({
      url: "https://github.com/AbhishekBarali/SpeakoFlow/releases/download/v1.5.0/SpeakoFlow_1.5.0_amd64.deb",
      signature: "sig-of-SpeakoFlow_1.5.0_amd64.deb",
    });
  });

  test("manual downloads point macOS at the .dmg, not the updater tarball", () => {
    expect(manifest.speakoflow.downloads["darwin-aarch64"]).toEndWith(
      "SpeakoFlow_1.5.0_aarch64.dmg",
    );
    expect(manifest.speakoflow.downloads["windows-x86_64"]).toEndWith(
      "SpeakoFlow_1.5.0_x64-setup.exe",
    );
    expect(manifest.speakoflow.downloads["linux-x86_64-deb"]).toEndWith(
      "SpeakoFlow_1.5.0_amd64.deb",
    );
    expect(manifest.speakoflow.release_url).toBe(
      "https://github.com/AbhishekBarali/SpeakoFlow/releases/tag/v1.5.0",
    );
  });

  test("an unsigned bundle is downloadable but never offered as an update", () => {
    const names = V140_ASSETS.filter(
      (name) => name !== "SpeakoFlow_1.5.0_amd64.deb.sig",
    );
    const partial = buildManifest({
      version: "1.5.0",
      notes: "",
      pubDate: "2026-09-28T00:00:00.000Z",
      repo: "AbhishekBarali/SpeakoFlow",
      tag: "v1.5.0",
      assetNames: names,
      signatures: signaturesFor(names),
    });
    expect(partial.platforms["linux-x86_64-deb"]).toBeUndefined();
    expect(partial.speakoflow.downloads["linux-x86_64-deb"]).toBeDefined();
  });

  test("an MSI-only Windows release still has a primary entry, a deb-only Linux one does not", () => {
    const names = [
      "SpeakoFlow_1.5.0_x64_en-US.msi",
      "SpeakoFlow_1.5.0_amd64.deb",
    ];
    const partial = buildManifest({
      version: "1.5.0",
      notes: "",
      pubDate: "2026-09-28T00:00:00.000Z",
      repo: "AbhishekBarali/SpeakoFlow",
      tag: "v1.5.0",
      assetNames: names,
      signatures: new Map(names.map((name) => [name, "s"])),
    });
    expect(partial.platforms["windows-x86_64"]).toEqual(
      partial.platforms["windows-x86_64-msi"],
    );
    expect(partial.platforms["linux-x86_64"]).toBeUndefined();
  });

  test("runaway release notes are cut rather than dropped", () => {
    const long = buildManifest({
      version: "1.5.0",
      notes: "x".repeat(20000),
      pubDate: "2026-09-28T00:00:00.000Z",
      repo: "AbhishekBarali/SpeakoFlow",
      tag: "v1.5.0",
      assetNames: [],
      signatures: new Map(),
    });
    expect(long.notes.length).toBeLessThan(8100);
    expect(long.notes).toEndWith("…");
  });
});

describe("tagMatchesVersion", () => {
  test.each([
    ["v1.5.0", "1.5.0", true],
    ["v1.5.0-beta.1", "1.5.0", true],
    ["v1.5.01", "1.5.0", false],
    ["v1.4.0", "1.5.0", false],
    ["1.5.0", "1.5.0", false],
  ])("%s vs %s → %s", (tag, version, expected) => {
    expect(tagMatchesVersion(tag, version)).toBe(expected);
  });
});
