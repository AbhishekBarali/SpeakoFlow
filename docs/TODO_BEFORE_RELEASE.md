# TODO Before Release — Manual Action Items

A living checklist of things a maintainer must do **by hand before shipping a
public build**. These are steps the code can't do for itself — uploading files,
flipping test-only flags, signing. Check each box as you complete it.

> See also `docs/KNOWN_ISSUES.md` for deferred defects and background.

---

## 1. Revert the testing-only onboarding override

- [x] In `src/App.tsx`, ensure the testing-only override cannot force
      onboarding in release builds.

**Release-safe as of 2026-07-13 (verified against current code).** `src/App.tsx`
still defines `const FORCE_ONBOARDING = import.meta.env.DEV;` and the guard at the
top of `checkOnboardingStatus`, but it is gated to **dev builds only**:
`import.meta.env.DEV` is `true` during `bun run tauri dev` (so the wizard shows
every launch for easy iteration) and **`false` in any compiled/release build**.
So a shipped build automatically falls back to the real first-run detection
(`hasAnyModelsAvailable()`) — onboarding shows only for genuinely new users, or
returning users missing permissions. **No manual action needed before release.**

> Correction: an earlier revision of this doc claimed the constant was deleted
> entirely (`grep` = 0 matches). That is inaccurate — it is retained but
> dev-gated, which is functionally equivalent for releases while preserving the
> dev-time convenience.

---

## 2. Publish reliable download mirrors for the bundled AI (LLM) models

**Why:** the large GGUF models download from Hugging Face, which is flaky for
big files (this caused the `error sending request for url …` failures). The
downloader now **auto-retries and resumes**, and will additionally **prefer a
reliable mirror** when one is configured. Uploading mirrors makes first-run
downloads rock-solid; where a mirror isn't possible, retry/resume still covers
it.

### How to add a mirror for ANY model (repeat per model)

1. **Get the exact file.** Download the model's `.gguf` from its Hugging Face
   URL (see `url` in `src-tauri/src/managers/model.rs`). Keep the **exact same
   filename** the app expects (the `filename` field).
2. **Upload it as a GitHub release asset** on the SpeakoFlow repo (e.g. a
   release tagged `models-v1`). GitHub serves these from a fast global CDN.
3. **Wire it up** in `src-tauri/src/managers/model.rs` → `mirror_url_for`: add
   (or uncomment) an arm returning the asset's `browser_download_url`:
   ```rust
   fn mirror_url_for(model_id: &str) -> Option<String> {
       match model_id {
           "gemma-3-1b" => Some(
               "https://github.com/AbhishekBarali/SpeakoFlow/releases/download/models-v1/gemma-3-1b-it-Q4_K_M.gguf".to_string(),
           ),
           // add more arms here, one per model you mirror …
           _ => None,
       }
   }
   ```
4. **Rebuild.** Downloads now try the mirror first and fall back to Hugging Face
   automatically.

### Constraints & gotchas (read before uploading)

- **GitHub asset limit is 2 GB per file.** Only models **under 2 GB** can be
  mirrored on GitHub Releases. Larger ones need another host (Cloudflare R2,
  Backblaze B2, your own CDN, …) or stay on Hugging Face with retry/resume.
- **Vision models have TWO files:** the main weights **and** a vision projector
  (`mmproj-*.gguf`). `mirror_url_for` currently mirrors **only the main
  weights**; the projector still downloads from Hugging Face. To fully mirror a
  vision model you'd also need to mirror its mmproj file **and** extend the code
  to use it (see "Future improvements" below — not done yet).
- Keep filenames identical to the `filename` field, or the app won't recognize
  the downloaded file.

### Per-model reference — tick off as you upload each mirror

Values pulled from `src-tauri/src/managers/model.rs` (confirm before uploading):

| Model id     | Name                              | File (`filename`)             | Size     | Fits GitHub 2 GB? | Vision projector (extra file)     | Mirror uploaded? |
| ------------ | --------------------------------- | ----------------------------- | -------- | ----------------- | --------------------------------- | ---------------- |
| `gemma-3-1b` | Gemma 3 1B                        | `gemma-3-1b-it-Q4_K_M.gguf`   | 806 MB   | ✅ yes            | none (text only)                  | [ ]              |
| `qwen3.5-2b` | Qwen3.5 2B (Vision)               | `Qwen_Qwen3.5-2B-Q4_K_M.gguf` | ~2.35 GB | ❌ over 2 GB      | `mmproj-Qwen_Qwen3.5-2B-f16.gguf` | [ ]              |
| `qwen3.5-4b` | Qwen3.5 4B (Vision) — **default** | `Qwen_Qwen3.5-4B-Q4_K_M.gguf` | ~3.9 GB  | ❌ over 2 GB      | `mmproj-Qwen_Qwen3.5-4B-f16.gguf` | [ ]              |
| `gemma-3-4b` | Gemma 3 4B (Vision)               | `gemma-3-4b-it-Q4_K_M.gguf`   | ~3.35 GB | ❌ over 2 GB      | `mmproj-model-f16.gguf`           | [ ]              |

> Practically: **Gemma 3 1B** is the only one that fits GitHub Releases today.
> For the 2–4 GB vision models, either use a non-GitHub host or rely on the
> retry/resume from Hugging Face (which already fixes the original failure).

---

## 3. Windows code signing / SmartScreen "Unknown publisher"

- [ ] See `docs/KNOWN_ISSUES.md` §1. Ensure release builds are **actually
      signed** with the configured Azure Trusted Signing, and consider a
      **winget** submission (free, and winget installs skip the popup).

### ⚠️ Skipping code signing for TEST builds — safe, but know the caveats

Building **unsigned** — via `--config tauri.local-build.json` (which swaps the
Azure `signCommand` for a no-op) or a CI test build with `sign-binaries: false`
— is **completely safe for testing** and has **no effect on the code or on a
later signed build**. The compiled app is byte-for-byte the same program;
signing only wraps a cryptographic seal around the installer at packaging time.
There is no residue — a future signed release "just works".

What unsigned builds DO change (distribution only — so don't ship them to real
users):

- **Windows:** SmartScreen shows "Windows protected your PC — unknown
  publisher". Users must click _More info → Run anyway_. Not broken, just scary.
- **macOS:** Gatekeeper is stricter — "app can't be opened / unidentified
  developer" (or "damaged"). Users must right-click → Open, or run
  `xattr -cr /Applications/SpeakoFlow.app`.
- **Auto-updater is OFF:** `tauri.local-build.json` sets
  `createUpdaterArtifacts: false`, and the Tauri updater needs the
  `TAURI_SIGNING_PRIVATE_KEY` signature regardless — so test builds cannot
  auto-update. Fine for testing; required for a real release.
- **Linux:** no OS-level code signing exists, so nothing changes there.

**Bottom line:** unsigned = perfect for testing, not for public distribution.

### How to obtain the signing certificates (when you're ready to ship)

- **Windows — Azure Trusted Signing** (now "Azure Artifact Signing"; this is
  what the repo's `signCommand` already targets): ~**$9.99/month** (Basic, up to
  5,000 signatures). Needs an Azure subscription + Microsoft **identity
  validation** (individuals are now eligible, not just orgs). Create a Trusted
  Signing account + certificate profile, then set repo secrets
  `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_TENANT_ID`. The account/
  profile names in `tauri.conf.json`'s `signCommand` must match your setup.
- **macOS — Apple Developer Program**: **$99/year**. Create a **Developer ID
  Application** certificate (for distribution outside the App Store), export it
  as `.p12`, and set repo secrets `APPLE_CERTIFICATE`,
  `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD` (an app-specific
  password), `APPLE_TEAM_ID`, `KEYCHAIN_PASSWORD`. Notarization is free and runs
  in CI. A Mac is not required — GitHub's macOS runners do the signing.
- **Updater signature** (both platforms, for auto-update): generate with
  `bun tauri signer generate`; set `TAURI_SIGNING_PRIVATE_KEY` and
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The public key is already in
  `tauri.conf.json`.
- **Linux:** no certificate required.

---

## Future improvements (nice-to-have, not release blockers)

- [ ] Extend auto-retry + mirror fallback to the **vision projector (mmproj)**
      download. It currently goes through `download_companion` in
      `model.rs`, which is a single-shot `reqwest::Client::new()` request with
      no retry — so vision-model projector downloads don't yet benefit from the
      reliability work. Route it through `attempt_download` (or a shared retry
      helper) and add an mmproj entry to the mirror lookup.
