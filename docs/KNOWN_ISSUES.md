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

## 3. AI Correction (dictation post-processing) — experimental, in development

**Status:** Intentionally shelved behind Experimental Features. Off by default.
**Severity:** Low — feature is hidden and non-default; core dictation/assistant
are unaffected.

### What it is

An optional second pass that sends a finished dictation transcript to an LLM to
clean it up (punctuation, filler words) and optionally adjust its tone
(Formal / Casual / Professional / Friendly / Concise), then pastes the cleaned
text instead of the raw transcript. It runs on its own hotkey
(`Ctrl+Shift+Space`), reuses the assistant's provider/model by default, has a
configurable timeout, and always falls back to the raw transcription on any
failure or timeout.

### Why it's deferred

The plumbing works, but the UX isn't polished enough to be a main feature yet:

- **Built-in local model picker is incomplete.** In Post Process → Model, the
  built-in (local) provider shows an empty "Type a model name" field and does
  not list the user's downloaded local models the way the Assistant model picker
  does. The runtime falls back to the assistant's model, so it still functions,
  but the empty field makes it look broken and there's no clear way to choose a
  dedicated small model (e.g. Gemma 1B) from the UI.
- Needs prompt/tone tuning and real-world testing before it's trustworthy as a
  default correction step.

### Current gating (as shipped)

- The **AI Correction** toggle lives inside **Advanced → Experimental Features**
  (only visible when Experimental is enabled), and its own toggle must then be
  turned on — a deliberate two-step gate.
- The **Post Process** settings section (provider / model / tone / prompts) only
  appears when _both_ `experimental_enabled` **and** `post_process_enabled` are
  true (`src/components/Sidebar.tsx`).
- The long explanation lives behind the row's (i) info hint; the inline
  description is kept short and flags it as "in development."

### Future work (when picked up)

1. Give the built-in provider a proper **downloaded-local-model dropdown** in the
   Post Process model field (mirror the Assistant/Models picker), and a hint that
   picking a _different_ local model than the assistant causes a GGUF reload when
   switching between chat and dictation.
2. Tune the cleanup + tone prompts; validate output quality across models.
3. Decide whether it graduates out of Experimental and, if so, revisit defaults.

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
