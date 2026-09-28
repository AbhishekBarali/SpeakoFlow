/**
 * Builds `latest.json`, the file every installed SpeakoFlow polls to learn
 * that a new version exists, and uploads it to the GitHub release.
 *
 * Why this exists instead of tauri-action's own `latest.json`:
 *
 *  1. The Linux AppImage and .deb are unpacked and repacked (to add the
 *     transcribe engine libraries) *after* tauri-action has uploaded them, so
 *     any signature tauri-action wrote into its manifest belongs to a file
 *     nobody downloads. The build re-signs the repacked files; this script
 *     reads the signatures that are actually on the release.
 *  2. Five matrix jobs writing one manifest race each other. One job at the
 *     end, reading what is attached, cannot.
 *  3. Every signature is verified against the public key compiled into the
 *     app, using the published bytes. A manifest whose signature would be
 *     rejected at install time fails here, in CI, instead of on a user's
 *     machine as "update failed".
 *
 * The manifest also carries a `speakoflow` block the updater ignores: direct
 * installer links per platform, which the app offers as "Download installer"
 * when an in-app install is not possible (portable build, a package manager
 * owns the install, or the install itself failed).
 *
 * Usage (CI):
 *   bun scripts/ci/updater-manifest.ts --release-id <id> \
 *     --conf src-tauri/tauri.conf.json --out latest.json --upload
 * Env: GITHUB_TOKEN, GITHUB_REPOSITORY (owner/name).
 */
import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

// ── Asset classification ──────────────────────────────────────────────────

export type UpdaterOs = "windows" | "darwin" | "linux";
export type UpdaterArch = "x86_64" | "aarch64" | "i686" | "armv7";
/** Installer names exactly as tauri-plugin-updater spells them. */
export type UpdaterBundle = "nsis" | "msi" | "app" | "appimage" | "deb" | "rpm";

export interface ClassifiedAsset {
  os: UpdaterOs;
  arch: UpdaterArch;
  bundle: UpdaterBundle;
}

export function normalizeArch(raw: string): UpdaterArch | null {
  switch (raw.toLowerCase()) {
    case "x64":
    case "amd64":
    case "x86_64":
      return "x86_64";
    case "arm64":
    case "aarch64":
      return "aarch64";
    case "x86":
    case "i386":
    case "i686":
      return "i686";
    case "armhf":
    case "armv7":
      return "armv7";
    default:
      return null;
  }
}

const UPDATER_PATTERNS: Array<[RegExp, UpdaterOs, UpdaterBundle]> = [
  // SpeakoFlow_1.5.0_x64-setup.exe
  [/_([a-z0-9]+)-setup\.exe$/i, "windows", "nsis"],
  // SpeakoFlow_1.5.0_x64_en-US.msi
  [/_([a-z0-9]+)_[a-z]{2,3}(?:-[a-z0-9]+)*\.msi$/i, "windows", "msi"],
  // SpeakoFlow_aarch64.app.tar.gz (tauri-action appends the arch)
  [/_([a-z0-9]+)\.app\.tar\.gz$/i, "darwin", "app"],
  // SpeakoFlow_1.5.0_amd64.AppImage
  [/_([a-z0-9]+)\.AppImage$/, "linux", "appimage"],
  // SpeakoFlow_1.5.0_amd64.deb
  [/_([a-z0-9]+)\.deb$/, "linux", "deb"],
  // SpeakoFlow-1.5.0-1.x86_64.rpm
  [/\.([a-z0-9_]+)\.rpm$/, "linux", "rpm"],
];

/** Which updater platform an asset serves, or null if it is not an updater bundle. */
export function classifyUpdaterAsset(name: string): ClassifiedAsset | null {
  for (const [pattern, os, bundle] of UPDATER_PATTERNS) {
    const match = pattern.exec(name);
    if (!match) continue;
    const arch = normalizeArch(match[1]);
    if (!arch) return null;
    return { os, arch, bundle };
  }
  return null;
}

/** macOS disk images are for people, not for the updater. */
export function classifyDmg(name: string): { arch: UpdaterArch } | null {
  const match = /_([a-z0-9]+)\.dmg$/i.exec(name);
  if (!match) return null;
  const arch = normalizeArch(match[1]);
  return arch ? { arch } : null;
}

/**
 * The bare `{os}-{arch}` key is what an install with no recorded bundle type
 * falls back to (a dev build, a from-source install). NSIS is the Windows
 * default because it is what nearly every Windows user installed; the MSI
 * still gets its own `windows-x86_64-msi` entry so MSI installs update as MSI.
 */
const PRIMARY_BUNDLE: Record<UpdaterOs, UpdaterBundle> = {
  windows: "nsis",
  darwin: "app",
  linux: "appimage",
};

// ── Manifest ──────────────────────────────────────────────────────────────

export interface ManifestPlatform {
  signature: string;
  url: string;
}

export interface UpdaterManifest {
  version: string;
  notes: string;
  pub_date: string;
  platforms: Record<string, ManifestPlatform>;
  speakoflow: {
    release_url: string;
    /** Full installers for a manual install, keyed like `platforms`. */
    downloads: Record<string, string>;
  };
}

export interface BuildManifestInput {
  version: string;
  notes: string;
  pubDate: string;
  repo: string;
  tag: string;
  assetNames: string[];
  /** Updater asset name → contents of its `.sig` file. */
  signatures: Map<string, string>;
}

/** Release notes are shown in a small card; a runaway body is cut, not dropped. */
export const MAX_NOTES_CHARS = 8000;

export function assetUrl(repo: string, tag: string, name: string): string {
  return `https://github.com/${repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(name)}`;
}

export function buildManifest(input: BuildManifestInput): UpdaterManifest {
  const platforms: Record<string, ManifestPlatform> = {};
  const downloads: Record<string, string> = {};

  const sorted = [...input.assetNames].sort();
  for (const name of sorted) {
    const dmg = classifyDmg(name);
    if (dmg) {
      const url = assetUrl(input.repo, input.tag, name);
      downloads[`darwin-${dmg.arch}-dmg`] = url;
      downloads[`darwin-${dmg.arch}`] = url;
      continue;
    }

    const classified = classifyUpdaterAsset(name);
    if (!classified) continue;
    const { os, arch, bundle } = classified;
    const url = assetUrl(input.repo, input.tag, name);

    // Manual-install links. The macOS updater bundle is a tarball of the
    // .app, which is not something to hand a person, so darwin links come
    // from the .dmg above instead.
    if (os !== "darwin") {
      downloads[`${os}-${arch}-${bundle}`] = url;
      if (bundle === PRIMARY_BUNDLE[os]) downloads[`${os}-${arch}`] = url;
    }

    const signature = input.signatures.get(name)?.trim();
    if (!signature) continue;
    const entry = { signature, url };
    platforms[`${os}-${arch}-${bundle}`] = entry;
    if (bundle === PRIMARY_BUNDLE[os]) platforms[`${os}-${arch}`] = entry;
  }

  // A Windows release with only an MSI still needs a primary entry. Linux is
  // deliberately not given one from a .deb: the bare key is what an install
  // with no recorded bundle type falls back to, and a .deb is never right for
  // a machine that did not install one.
  for (const key of Object.keys(platforms)) {
    const [os, arch] = key.split("-");
    const primary = `${os}-${arch}`;
    if (os === "windows" && !platforms[primary])
      platforms[primary] = platforms[key];
  }

  const notes =
    input.notes.length > MAX_NOTES_CHARS
      ? `${input.notes.slice(0, MAX_NOTES_CHARS).trimEnd()}\n\n…`
      : input.notes;

  return {
    version: input.version,
    notes,
    pub_date: input.pubDate,
    platforms: sortKeys(platforms),
    speakoflow: {
      release_url: `https://github.com/${input.repo}/releases/tag/${encodeURIComponent(input.tag)}`,
      downloads: sortKeys(downloads),
    },
  };
}

function sortKeys<T>(record: Record<string, T>): Record<string, T> {
  return Object.fromEntries(
    Object.entries(record).sort(([a], [b]) => a.localeCompare(b)),
  );
}

/**
 * The release tag must name the version being published, either exactly
 * (v1.5.0) or as a prerelease of it (v1.5.0-beta.1). Anything else means the
 * checkout and the release disagree, and publishing would advertise the wrong
 * version to every install.
 */
export function tagMatchesVersion(tag: string, version: string): boolean {
  return tag === `v${version}` || tag.startsWith(`v${version}-`);
}

// ── Minisign verification (what the updater does at install time) ─────────

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

function decodeBase64Text(value: string): string {
  return Buffer.from(value.trim(), "base64").toString("utf8");
}

function nonCommentLines(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

export interface VerifyResult {
  ok: boolean;
  reason?: string;
}

/**
 * Verify a Tauri updater signature exactly the way tauri-plugin-updater does
 * (minisign-verify with the global signature checked): both the detached
 * signature over the file and the signature over the trusted comment must
 * hold, and the key ids must match.
 *
 * `signatureB64` is the contents of a `.sig` file; `pubkeyB64` is the
 * `plugins.updater.pubkey` string from tauri.conf.json.
 */
export function verifyUpdaterSignature(
  data: Uint8Array,
  signatureB64: string,
  pubkeyB64: string,
): VerifyResult {
  let pkLine: string;
  let sigLines: string[];
  try {
    const pkText = nonCommentLines(decodeBase64Text(pubkeyB64));
    pkLine =
      pkText.find((line) => !line.startsWith("untrusted comment:")) ?? "";
    sigLines = nonCommentLines(decodeBase64Text(signatureB64));
  } catch (error) {
    return { ok: false, reason: `not base64: ${(error as Error).message}` };
  }

  const pk = Buffer.from(pkLine, "base64");
  if (pk.length !== 42 || pk.subarray(0, 2).toString("latin1") !== "Ed") {
    return { ok: false, reason: "public key is not a minisign Ed25519 key" };
  }
  const pkKeyId = pk.subarray(2, 10);
  const publicKey = createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, pk.subarray(10, 42)]),
    format: "der",
    type: "spki",
  });

  // Lines: untrusted comment, signature, trusted comment, global signature.
  const body = sigLines.filter(
    (line) => !line.startsWith("untrusted comment:"),
  );
  const [sigLine, trustedLine, globalLine] = body;
  if (!sigLine || !trustedLine || !globalLine) {
    return { ok: false, reason: "signature file is incomplete" };
  }
  if (!trustedLine.startsWith("trusted comment: ")) {
    return { ok: false, reason: "signature file has no trusted comment" };
  }

  const sig = Buffer.from(sigLine, "base64");
  if (sig.length !== 74) {
    return { ok: false, reason: "signature has the wrong length" };
  }
  const algorithm = sig.subarray(0, 2).toString("latin1");
  if (algorithm !== "Ed" && algorithm !== "ED") {
    return { ok: false, reason: `unknown signature algorithm ${algorithm}` };
  }
  if (!sig.subarray(2, 10).equals(pkKeyId)) {
    return {
      ok: false,
      reason: `signed with key ${keyIdHex(sig.subarray(2, 10))}, app trusts ${keyIdHex(pkKeyId)}`,
    };
  }
  const signature = sig.subarray(10, 74);

  // "ED" is minisign's prehashed mode (BLAKE2b-512 of the file); "Ed" signs
  // the raw bytes. The Tauri CLI produces "ED".
  const message =
    algorithm === "ED"
      ? createHash("blake2b512").update(data).digest()
      : Buffer.from(data);
  if (!verify(null, message, publicKey, signature)) {
    return { ok: false, reason: "signature does not match the file" };
  }

  const trustedComment = Buffer.from(
    trustedLine.slice("trusted comment: ".length),
    "utf8",
  );
  const globalSignature = Buffer.from(globalLine, "base64");
  if (
    globalSignature.length !== 64 ||
    !verify(
      null,
      Buffer.concat([signature, trustedComment]),
      publicKey,
      globalSignature,
    )
  ) {
    return { ok: false, reason: "trusted comment signature is invalid" };
  }

  return { ok: true };
}

/** minisign key ids are little-endian; print them the way `minisign` does. */
function keyIdHex(bytes: Uint8Array): string {
  return Buffer.from(bytes).reverse().toString("hex").toUpperCase();
}

// ── CLI ───────────────────────────────────────────────────────────────────

interface ReleaseAsset {
  id: number;
  name: string;
  size: number;
}

interface Release {
  id: number;
  tag_name: string;
  body: string | null;
  draft: boolean;
  prerelease: boolean;
  upload_url: string;
  assets: ReleaseAsset[];
}

function arg(name: string): string | undefined {
  const index = process.argv.indexOf(`--${name}`);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is not set`);
  return value;
}

async function github(
  path: string,
  init: RequestInit & { accept?: string } = {},
): Promise<Response> {
  const url = path.startsWith("https://")
    ? path
    : `https://api.github.com${path}`;
  const response = await fetch(url, {
    ...init,
    headers: {
      Authorization: `Bearer ${requireEnv("GITHUB_TOKEN")}`,
      Accept: init.accept ?? "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
      "User-Agent": "speakoflow-updater-manifest",
      ...(init.headers ?? {}),
    },
  });
  if (!response.ok) {
    const detail = await response.text().catch(() => "");
    throw new Error(
      `${init.method ?? "GET"} ${url} → ${response.status} ${detail.slice(0, 300)}`,
    );
  }
  return response;
}

/**
 * Asset bytes through the API rather than `browser_download_url`: the latter
 * 404s while a release is still a draft, which is exactly when this runs.
 */
async function downloadAsset(
  repo: string,
  asset: ReleaseAsset,
): Promise<Uint8Array> {
  const response = await github(`/repos/${repo}/releases/assets/${asset.id}`, {
    accept: "application/octet-stream",
  });
  return new Uint8Array(await response.arrayBuffer());
}

async function listAllAssets(
  repo: string,
  releaseId: number,
): Promise<ReleaseAsset[]> {
  const assets: ReleaseAsset[] = [];
  for (let page = 1; page < 20; page++) {
    const response = await github(
      `/repos/${repo}/releases/${releaseId}/assets?per_page=100&page=${page}`,
    );
    const batch = (await response.json()) as ReleaseAsset[];
    assets.push(...batch);
    if (batch.length < 100) break;
  }
  return assets;
}

async function uploadManifest(
  repo: string,
  release: Release,
  assets: ReleaseAsset[],
  body: string,
): Promise<void> {
  const existing = assets.find((asset) => asset.name === "latest.json");
  if (existing) {
    await github(`/repos/${repo}/releases/assets/${existing.id}`, {
      method: "DELETE",
    });
  }
  const uploadBase = release.upload_url.replace(/\{.*\}$/, "");
  await github(`${uploadBase}?name=latest.json`, {
    method: "POST",
    body,
    headers: { "Content-Type": "application/json" },
  });
}

async function main(): Promise<void> {
  const repo = requireEnv("GITHUB_REPOSITORY");
  const releaseId = Number(arg("release-id") ?? process.env.RELEASE_ID);
  if (!Number.isFinite(releaseId) || releaseId <= 0) {
    throw new Error("--release-id (or RELEASE_ID) is required");
  }
  const confPath = arg("conf") ?? "src-tauri/tauri.conf.json";
  const outPath = arg("out") ?? "latest.json";
  const upload = process.argv.includes("--upload");

  const conf = JSON.parse(readFileSync(confPath, "utf8")) as {
    version: string;
    plugins?: { updater?: { pubkey?: string } };
  };
  const pubkey = conf.plugins?.updater?.pubkey;
  if (!pubkey) throw new Error(`${confPath} has no plugins.updater.pubkey`);

  const release = (await (
    await github(`/repos/${repo}/releases/${releaseId}`)
  ).json()) as Release;
  if (!tagMatchesVersion(release.tag_name, conf.version)) {
    throw new Error(
      `release ${releaseId} is tagged ${release.tag_name} but the checkout is version ${conf.version}`,
    );
  }
  const assets = await listAllAssets(repo, releaseId);
  const byName = new Map(assets.map((asset) => [asset.name, asset]));

  console.log(
    `Release ${release.tag_name} (${release.draft ? "draft" : "published"}${release.prerelease ? ", prerelease" : ""}), ${assets.length} assets`,
  );

  const signatures = new Map<string, string>();
  const failures: string[] = [];
  for (const asset of assets) {
    if (!classifyUpdaterAsset(asset.name)) continue;
    const sigAsset = byName.get(`${asset.name}.sig`);
    if (!sigAsset) {
      console.log(
        `  - ${asset.name}: no .sig on the release, not offered as an update`,
      );
      continue;
    }
    const signature = new TextDecoder()
      .decode(await downloadAsset(repo, sigAsset))
      .trim();
    const bytes = await downloadAsset(repo, asset);
    const result = verifyUpdaterSignature(bytes, signature, pubkey);
    if (!result.ok) {
      failures.push(`${asset.name}: ${result.reason}`);
      continue;
    }
    console.log(
      `  ✓ ${asset.name} (${(bytes.length / 1e6).toFixed(1)} MB) signature verified`,
    );
    signatures.set(asset.name, signature);
  }

  if (failures.length > 0) {
    for (const failure of failures) console.error(`::error::${failure}`);
    throw new Error(
      `${failures.length} updater signature(s) would be rejected by the app. Refusing to publish a manifest that fails at install time.`,
    );
  }
  if (signatures.size === 0) {
    throw new Error(
      "No signed updater bundles on this release. Was TAURI_SIGNING_PRIVATE_KEY available to the build?",
    );
  }

  const manifest = buildManifest({
    version: conf.version,
    notes: release.body ?? "",
    pubDate: new Date().toISOString(),
    repo,
    tag: release.tag_name,
    assetNames: assets.map((asset) => asset.name),
    signatures,
  });

  const json = `${JSON.stringify(manifest, null, 2)}\n`;
  writeFileSync(outPath, json);
  console.log(`\nWrote ${outPath} for ${manifest.version}:`);
  for (const key of Object.keys(manifest.platforms))
    console.log(`  update   ${key}`);
  for (const key of Object.keys(manifest.speakoflow.downloads))
    console.log(`  download ${key}`);

  if (upload) {
    await uploadManifest(repo, release, assets, json);
    console.log(`Uploaded latest.json to ${release.tag_name}`);
  }
}

if (import.meta.main) {
  main().catch((error: unknown) => {
    console.error(`::error::${(error as Error).message}`);
    process.exit(1);
  });
}
