# TODO Before Release — Manual Action Items

A living checklist of things a maintainer must do **by hand before shipping a
public build**. These are steps the code can't do for itself — uploading files,
flipping test-only flags, signing. Check each box as you complete it.

> See also `docs/KNOWN_ISSUES.md` for deferred defects and background.

---

## 1. Revert the testing-only onboarding override

- [x] In `src/App.tsx`, set `FORCE_ONBOARDING = false` (or delete the constant
      and the guard at the top of `checkOnboardingStatus`).

**Resolved by the Simplicity Overhaul (S1), verified by S6 on 2026-07-13.** The
`FORCE_ONBOARDING` constant and its guard were **deleted entirely** from
`src/App.tsx` — `grep FORCE_ONBOARDING src/` now returns 0 matches. Onboarding
shows only for genuinely new users (no models installed) or returning users
missing permissions, via the normal `hasAnyModelsAvailable()` path. Nothing to
flip before release.

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

---

## Future improvements (nice-to-have, not release blockers)

- [ ] Extend auto-retry + mirror fallback to the **vision projector (mmproj)**
      download. It currently goes through `download_companion` in
      `model.rs`, which is a single-shot `reqwest::Client::new()` request with
      no retry — so vision-model projector downloads don't yet benefit from the
      reliability work. Route it through `attempt_download` (or a shared retry
      helper) and add an mmproj entry to the mirror lookup.
