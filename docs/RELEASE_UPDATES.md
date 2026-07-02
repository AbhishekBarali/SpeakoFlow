# SpeakoFlow — Signing & Auto-Update Release Guide

How a SpeakoFlow release becomes an in-app update, and the exact manual steps to
build signed updater artifacts and publish `latest.json`.

> Scope: this is the **release/updater** reference. General release scope and the
> progress tracker live in [`RELEASE_PLAN.md`](./RELEASE_PLAN.md) and
> [`RELEASE_CHECKLIST.md`](./RELEASE_CHECKLIST.md).

---

## 1. How the update flow actually works

SpeakoFlow uses the official [Tauri v2 updater plugin](https://v2.tauri.app/plugin/updater/).
The moving parts, all wired up in this repo:

| Piece           | Where                                                         | Current value                                                                       |
| --------------- | ------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Updater enabled | `src-tauri/tauri.conf.json` → `bundle.createUpdaterArtifacts` | `true`                                                                              |
| Public key      | `tauri.conf.json` → `plugins.updater.pubkey`                  | a minisign public key (base64)                                                      |
| Update feed URL | `tauri.conf.json` → `plugins.updater.endpoints`               | `https://github.com/AbhishekBarali/SpeakoFlow/releases/latest/download/latest.json` |
| Frontend UI     | `src/components/update-checker/UpdateChecker.tsx`             | "Check for updates" in the footer + tray                                            |
| Tray entry      | `src-tauri/src/tray.rs` + `lib.rs` (`check_updates`)          | emits `check-for-updates`                                                           |

**The runtime flow:**

1. The app calls `check()` (plugin-updater). This is triggered on launch (if
   "Check for updates" is enabled), from the footer button, or from the tray
   **Check for Updates...** item (which focuses the window and emits
   `check-for-updates`).
2. `check()` fetches `latest.json` from the endpoint. GitHub's
   `/releases/latest/download/<asset>` path always serves the asset from the
   release currently marked **Latest**.
3. The plugin compares `latest.json`'s `version` to the running app version
   (`0.8.3` today, from `Cargo.toml` / `tauri.conf.json` / `package.json`).
4. If newer, the matching platform entry's installer is downloaded, its
   `signature` is **verified against `pubkey`**, then installed; the app relaunches.

If verification fails, or the endpoint/`latest.json` is missing or malformed,
the install/check fails. The UI now surfaces this honestly (see §6).

### Two different "signings" — do not confuse them

- **Updater signature (minisign, REQUIRED).** Tauri signs each updater artifact
  with a **minisign private key**; the app verifies with the `pubkey` in
  `tauri.conf.json`. Without a matching private key, you cannot produce updates
  the app will accept. This is the key that gates auto-update.
- **OS code signing (Windows Authenticode / macOS notarization, SEPARATE).**
  This is about the OS trusting the installer (SmartScreen / Gatekeeper). On
  Windows this repo's `tauri.conf.json` sets:
  ```
  "signCommand": "trusted-signing-cli -e https://eus.codesigning.azure.net/ -a SpeakoFlow-Signing -c speakoflow-dev -d SpeakoFlow %1"
  ```
  That uses **Azure Trusted Signing** and requires Azure credentials + the
  `trusted-signing-cli` tool. It is unrelated to whether auto-update works — but
  the Windows build **will fail** if `signCommand` runs without those creds. See §7.

---

## 2. One-time setup: the updater signing key

Do this **once**. Keep the private key out of the repo forever.

```bash
# Generates a keypair. -w writes the private key file; you'll be asked for a password.
bun run tauri signer generate -w ~/.tauri/speakoflow_updater.key
```

This prints (and writes) two things:

- **Private key** (`~/.tauri/speakoflow_updater.key`) + its password — SECRET.
- **Public key** (base64) — goes in `tauri.conf.json` → `plugins.updater.pubkey`.

> IMPORTANT — key/pubkey must match. `tauri.conf.json` already contains a
> `pubkey`. Auto-update only works if you hold the **matching private key**. If
> that key is not in your possession (e.g. it came from a template or an earlier
> owner), regenerate the keypair with the command above and **replace** the
> `pubkey` value in `tauri.conf.json` with the new public key. Shipping a build
> whose `pubkey` has no known private key means you can never publish an
> installable update.

Store for later use as environment variables at build time:

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/speakoflow_updater.key)"   # or the file path
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="<the password you set>"
```

---

## 3. Cut a release (manual, per version)

There is currently **no CI workflow** in this repo (`.github/workflows/` does not
exist), so releases are produced manually. A CI recommendation is in §8.

### 3.1 Bump the version in all three files (keep them identical)

- `src-tauri/tauri.conf.json` → `version`
- `src-tauri/Cargo.toml` → `[package] version`
- `package.json` → `version`

(Today all three read `0.8.3`. Bump to e.g. `0.8.4`.)

### 3.2 Build signed artifacts

With the signing env vars from §2 exported:

```bash
bun install
bun run tauri build
```

Tauri produces the normal installers **plus** updater artifacts and a `.sig` file
for each (because `createUpdaterArtifacts: true`). Per platform, the updater cares
about:

| Platform | Updater artifact         | Signature file        |
| -------- | ------------------------ | --------------------- |
| Windows  | `*_x64-setup.exe` (NSIS) | `*_x64-setup.exe.sig` |
| macOS    | `*.app.tar.gz`           | `*.app.tar.gz.sig`    |
| Linux    | `*.AppImage`             | `*.AppImage.sig`      |

Artifacts land under `src-tauri/target/release/bundle/…`. Build on each OS you
intend to ship (cross-compiling desktop installers is not practical).

### 3.3 Assemble `latest.json`

`latest.json` is the update manifest the app reads. The `signature` value is the
**full text contents of the corresponding `.sig` file** (not a path). Example:

```json
{
  "version": "0.8.4",
  "notes": "Bug fixes and improvements.",
  "pub_date": "2026-07-01T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contents of SpeakoFlow_0.8.4_x64-setup.exe.sig>",
      "url": "https://github.com/AbhishekBarali/SpeakoFlow/releases/download/v0.8.4/SpeakoFlow_0.8.4_x64-setup.exe"
    },
    "darwin-aarch64": {
      "signature": "<contents of SpeakoFlow_0.8.4_aarch64.app.tar.gz.sig>",
      "url": "https://github.com/AbhishekBarali/SpeakoFlow/releases/download/v0.8.4/SpeakoFlow_0.8.4_aarch64.app.tar.gz"
    },
    "darwin-x86_64": {
      "signature": "<contents of SpeakoFlow_0.8.4_x64.app.tar.gz.sig>",
      "url": "https://github.com/AbhishekBarali/SpeakoFlow/releases/download/v0.8.4/SpeakoFlow_0.8.4_x64.app.tar.gz"
    },
    "linux-x86_64": {
      "signature": "<contents of SpeakoFlow_0.8.4_amd64.AppImage.sig>",
      "url": "https://github.com/AbhishekBarali/SpeakoFlow/releases/download/v0.8.4/SpeakoFlow_0.8.4_amd64.AppImage"
    }
  }
}
```

Notes:

- Only include the platforms you actually built and uploaded.
- Every `url` must point at the uploaded release asset for that exact version.
- `version` must be greater than the shipped version for clients to update.
- Platform keys are `<os>-<arch>`: `windows-x86_64`, `darwin-aarch64`,
  `darwin-x86_64`, `linux-x86_64`.

### 3.4 Publish the GitHub release

```bash
# Tag convention: v<version>
gh release create v0.8.4 \
  --title "SpeakoFlow 0.8.4" \
  --notes "Bug fixes and improvements." \
  "src-tauri/target/release/bundle/nsis/SpeakoFlow_0.8.4_x64-setup.exe" \
  "src-tauri/target/release/bundle/nsis/SpeakoFlow_0.8.4_x64-setup.exe.sig" \
  latest.json
# ...add macOS/Linux artifacts if you built them, then:
```

Then, critically:

- The release must be marked **Latest** (not draft, not pre-release). The
  endpoint resolves `/releases/latest/download/latest.json`, so only the
  "Latest" release's `latest.json` is served.
- `latest.json` must be uploaded as a release **asset** with exactly that name.

---

## 4. Verify the update end to end

1. Install the previous version (e.g. `0.8.3`).
2. Publish `0.8.4` per §3 (marked Latest, `latest.json` attached).
3. In the old build, click **Check for updates** (footer) or tray **Check for
   Updates...**. It should report an update, download, verify, install, relaunch.
4. Confirm the relaunched app shows the new version in the footer and About page.

To sanity-check the feed without installing:

```bash
curl -L https://github.com/AbhishekBarali/SpeakoFlow/releases/latest/download/latest.json
```

You should get the JSON with the new `version`.

---

## 5. Portable builds

Portable installs cannot self-update (there is no installer to run). The app
detects this (`commands.isPortable()`) and shows a manual-update dialog pointing
at GitHub Releases. Nothing to publish differently — just be aware portable users
update by downloading the new installer manually.

---

## 6. What the in-app UX now does (honest states)

`UpdateChecker.tsx` renders one of:

- **Check for updates** — idle, clickable to check.
- **Checking for updates...** — in progress.
- **Up to date** — manual check found nothing (auto-hides).
- **Update available** — clickable to download/install.
- **Downloading… N% / Preparing… / Installing…** — during install.
- **Update check failed** / **Update failed** — shown in red, clickable to
  retry (tooltip: "Click to try again"). Only surfaced for user-initiated
  checks; silent background checks stay quiet.
- **Update Checking Disabled** — when the setting is off.

This removes the previous behavior where a failed check silently reverted to
"Check for updates" and looked like nothing happened.

---

## 7. Windows code signing (`signCommand`)

`tauri.conf.json` runs `trusted-signing-cli` (Azure Trusted Signing) on Windows.
This is independent of the updater. Two paths:

- **You have Azure Trusted Signing:** install `trusted-signing-cli`
  (`cargo install trusted-signing-cli`) and provide Azure creds
  (`AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`) so the account
  `speakoflow-dev` / profile `SpeakoFlow-Signing` resolves. Installers are then
  Authenticode-signed and avoid most SmartScreen warnings.
- **You do NOT have it yet:** the Windows build will fail when `signCommand`
  runs. To ship unsigned for now, temporarily remove the `windows.signCommand`
  line from `tauri.conf.json` and document that users will see a SmartScreen
  "unknown publisher" prompt. Auto-update still works unsigned — OS signing and
  updater signing are separate.

macOS notarization/Gatekeeper is not configured here; document the Gatekeeper
prompt or add notarization before shipping mac builds.

---

## 8. Optional: automate with GitHub Actions (recommended later)

A `tauri-apps/tauri-action` workflow triggered on `v*` tags can build the three
platforms, sign updater artifacts, generate `latest.json`, and publish the
release automatically. Store as repo secrets:

- `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- (Windows) `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`
- (macOS, if notarizing) Apple ID / API key secrets

`tauri-action` can emit `latest.json` (a.k.a. `updater.json`) for you, avoiding
the manual assembly in §3.3.

---

## 9. What the maintainer must provide (keys & secrets)

Auto-update is **code-complete and points at the correct repo**, but it cannot
be fully exercised without these, which only the project owner can supply:

1. **Updater minisign private key + password** matching the `pubkey` in
   `tauri.conf.json`. If the matching private key does not exist, regenerate the
   keypair (§2) and replace the `pubkey`. **Required for any working update.**
2. **The first published GitHub release** with installers, `.sig` files, and
   `latest.json`, marked **Latest**. Until one exists, "Check for updates" will
   correctly report failure/no update.
3. **(Windows) Azure Trusted Signing credentials** for the configured
   `signCommand`, or a decision to ship unsigned (§7).
4. **(macOS, if shipping)** Apple notarization credentials.

Once (1) and (2) exist, the flow in §4 can be verified end to end.
