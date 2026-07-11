# Known Issues & Deferred Defects

Tracked problems we've diagnosed but intentionally deferred, plus operational
follow-ups for shipped fixes. Update the status when one is picked up.

---

## 1. Windows: "Windows protected your PC" / **Unknown publisher** (SmartScreen)

**Status:** Deferred (distribution/signing task, not a code bug)
**Severity:** High for adoption — new users may abandon install, believing the
app is malware.

### Symptom

Running `SpeakoFlow_x.y.z_x64-setup.exe` shows the blue
**"Windows protected your PC"** dialog: _"Microsoft Defender SmartScreen
prevented an unrecognized app from starting… Publisher: **Unknown publisher**."_

### Root cause

The tested installer was **not code-signed**. `src-tauri/tauri.conf.json` already
declares a Windows `signCommand` (Azure Trusted Signing via `trusted-signing-cli`),
but the build that was tested clearly ran without it (hence "Unknown publisher").
Two separate things are at play:

- **Signing** proves _who_ published the binary. Without it → guaranteed
  "Unknown publisher" + SmartScreen every launch.
- **Reputation** is separate and accrues over time. Even a correctly signed app
  from a _new_ identity keeps showing SmartScreen until it earns download
  reputation. Signing is what lets reputation accumulate across versions instead
  of resetting each release.

### Options (researched)

| Option                             | Cost         | Instant trust?                                                                                           | Notes                                                                                               |
| ---------------------------------- | ------------ | -------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| **Azure Trusted Signing**          | ~$10/mo      | No — starts at zero reputation                                                                           | Already configured in `tauri.conf.json`. Cheapest real signing. Requires a verified Azure identity. |
| **EV code-signing cert**           | ~$400–700/yr | Historically yes, but **no longer reliable** — recent reports show EV-signed apps still warned initially | Requires hardware token / cloud HSM. Not worth it right now.                                        |
| **winget submission**              | Free         | Yes for winget installs                                                                                  | Apps installed via `winget` bypass the SmartScreen popup. Realistic for an OSS app.                 |
| **Microsoft Store**                | Free         | Yes (Store install)                                                                                      | Larger submission/packaging process.                                                                |
| **Build reputation**               | Free         | No                                                                                                       | Unavoidable baseline; signing makes it stick across versions.                                       |
| **Submit to Microsoft for review** | Free         | Sometimes                                                                                                | Can help; not guaranteed.                                                                           |

### Recommended path (when picked up)

1. **Verify the release pipeline actually signs** the artifact (check the built
   `.exe` → Properties → Digital Signatures shows the SpeakoFlow publisher). The
   config is present; confirm Azure Trusted Signing credentials are available at
   build time and that only the _local_ test build was unsigned.
2. **Also publish via winget** (free, and winget installs skip the popup).
3. Accept a short reputation ramp; do **not** buy an EV cert for now.

There is **no free way to make the warning vanish instantly.**

---

## 2. Model download mirrors (operational follow-up for the download-reliability fix)

**Status:** Mechanism shipped; mirrors not yet populated.

The downloader (`src-tauri/src/managers/model.rs`) now:

- **Auto-retries** each source up to 4× with exponential backoff (1s/2s/4s),
  **resuming** from the `.partial` file rather than restarting.
- Supports an **ordered source list** per model: a reliable mirror first, then
  the canonical Hugging Face URL as a fallback (`download_candidates` /
  `mirror_url_for`).

This is what fixes the intermittent `error sending request for url …` failures
that hit the large LLM (GGUF) downloads while the STT models — served from a
single reliable host — were fine.

### To activate mirrors (manual, per model)

The agent cannot upload release assets; a maintainer must do this. The full
step-by-step, constraints (GitHub's 2 GB limit, the vision-model `mmproj`
caveat), and a per-model checklist live in
**[`docs/TODO_BEFORE_RELEASE.md`](./TODO_BEFORE_RELEASE.md) §2**.

Until a mirror is wired up, `mirror_url_for` returns `None` and the canonical
Hugging Face URL is used directly (with retry/resume).

---

## 3. AI Correction (dictation post-processing)

**Status:** Promoted out of Experimental — a first-class, opt-in feature. Off by
default; the user enables it from the toggle at the top of the Post Process
settings page.
**Severity:** Low — off by default and only ever invoked by its own hotkey; core
dictation/assistant are unaffected.

### What it is

An optional second pass that sends a finished dictation transcript to an LLM to
clean it up (punctuation, filler words) and optionally adjust its tone
(Formal / Casual / Professional / Friendly / Concise), then pastes the cleaned
text instead of the raw transcript. It runs on its own hotkey
(`Ctrl+Shift+Space`), reuses the assistant's provider/model by default, has a
configurable timeout, and always falls back to the raw transcription on any
failure or timeout.

### Resolved (this pass)

- **No longer gated behind Experimental.** The **Post Process** section is
  always visible in the sidebar; the feature is toggled on/off from an enable
  switch at the top of that page (moved out of Advanced → Experimental). The
  hotkey registers whenever `post_process_enabled` is true — the old
  `experimental_enabled` requirement is gone (`shortcut/mod.rs`,
  `shortcut/handy_keys.rs`, `shortcut/tauri_impl.rs`).
- **Built-in local model picker.** The built-in (Local) provider now hides the
  API-key field and lists the user's downloaded LLM models in a dropdown
  (mirroring the Assistant picker) instead of showing an empty "Type a model
  name" field.
- **Azure / editable Base URL.** The Base URL field now renders for any provider
  that allows editing it (Custom, Local, **Azure OpenAI**) — not just Custom —
  so Azure post-processing is configurable. The base URL is stored on the shared
  provider object, so it's the same value the Assistant page edits.
- **Tone now takes effect.** Tone directives are appended as an explicit,
  highest-priority override (`actions.rs::append_tone_directive`) so they win
  over a cleanup prompt that insists on "don't paraphrase / output exactly" —
  previously that conflict made tone appear to do nothing.
- **Default prompt cleaned up.** The shipped "Improve Transcriptions" prompt is
  self-contained: no `{{agentName}}`/app-specific placeholders, keeps the
  `<transcript>` prompt-injection defense and the never-answer-the-content rule.

### Remaining follow-ups

1. Real-world quality validation of the cleanup + tone prompts across models
   (especially small local GGUFs).
2. Optional hint that picking a _different_ built-in local model than the
   assistant causes a GGUF reload when switching between chat and dictation.

Implementation touchpoints: `src-tauri/src/actions.rs`
(`post_process_transcription`, `resolve_post_process_provider_and_model`,
timeout in `process_transcription_output`), `src-tauri/src/settings.rs`
(`PostProcessTone`, `post_process_tone`, `post_process_timeout_secs`), and the
`src/components/settings/post-processing/` + `PostProcessingToggle` /
`PostProcessTimeout` UI.

---

## Note: temporary onboarding override (remove before release)

`src/App.tsx` has `FORCE_ONBOARDING = true`, which shows the full onboarding on
**every** launch for testing. It must be reverted before shipping — tracked in
[`docs/TODO_BEFORE_RELEASE.md`](./TODO_BEFORE_RELEASE.md) §1.
