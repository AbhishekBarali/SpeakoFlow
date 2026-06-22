# SpeakoFlow — Feature Build Plan

Planning doc for the upstream-inspired features we want to bring into SpeakoFlow.
Goal of this doc: (1) describe **what needs to be done** for each feature, (2) say
**which can be built independently vs. which must be sequenced**, grounded in the
actual code, and (3) give a **recommended order of attack** at the end.

> Status: planning only. No code changed yet.

---

## The three features

| # | Feature | Upstream PR | Effort | Build independently? |
|---|---------|-------------|--------|----------------------|
| 1 | Deterministic text replacements (find/replace + magic commands) | #455 / #1533 | Medium | ✅ Yes — isolated hook |
| 2 | Secure API-key storage in OS keychain | #814 | Low-Medium | ✅ Yes — orthogonal to the pipeline |
| 3 | Eager / streaming transcription (transcribe while recording) | #1515 | High | ❌ No — rewrites the shared core |

**Bottom line:** #1 and #2 can each be built on their own branch, in parallel, merged in
any order (zero file overlap). #3 rewrites the recording→transcription core, so it must
come **last and alone**.

### Out of scope (considered and dropped)

- **Local audio-file transcription (#381)** — import/transcribe MP3/M4A/WAV files.
  **Decided against.** It's a batch-transcription use case that pushes the app toward a
  general-purpose transcription tool, which is counter to SpeakoFlow's focus (voice →
  dictation/assistant, paste into the active app). Not needed. Kept here only to record the
  decision so it doesn't get re-proposed.

---

## Architecture touchpoints (verified against the code)

These are the exact places each feature plugs into. Confirmed by reading the files.

- **Transcript output choke point** — `src-tauri/src/actions.rs::process_transcription_output()`
  (~line 393). Every dictation transcript flows through here: raw text →
  (optional Chinese conversion) → (optional LLM post-process) → `final_text` → pasted
  and written to history. Returns `ProcessedTranscription { final_text, post_processed_text, post_process_prompt }`.
- **Live transcribe entry** — `TranscriptionManager::transcribe(samples)`, called inside
  `TranscribeAction::stop()` (`actions.rs`, async task) after `rm.stop_recording()`.
  This is the batch API that #3 will restructure.
- **Recording/coordination** — `transcription_coordinator.rs` is a single-threaded state
  machine (`Idle → Recording → Processing`). Actions live in `actions.rs` via the
  `ShortcutAction` trait (`start`/`stop`) and `ACTION_MAP`.
- **API-key storage** — `settings.rs`:
  - `SecretMap` (line ~359) and `SecretString` (line ~387) are `#[serde(transparent)]`,
    so their values are **serialized in plaintext** into `settings_store.json`
    (`SETTINGS_STORE_PATH`). The custom `Debug` impls only redact **logs**, not disk.
  - `get_settings()` (line ~1347) reloads the whole `AppSettings` blob from the store on
    every call (**hot path** — called by every action). Writes go through the same store.
  - **Read sites (only 4):** `actions.rs` (`post_process_api_keys`), `assistant.rs`
    (`post_process_api_keys`), `tts.rs` (`assistant_tts_api_key`), `web_search.rs`
    (`web_search_api_keys`).
  - **Write sites (only 2):** `commands/assistant.rs` (`set_assistant_tts_api_key`,
    `web_search_api_keys.insert`), `shortcut/mod.rs` (`post_process_api_keys.insert`).
- **History** — `managers/history.rs` (large). Stores transcripts + WAV path. #1 may
  optionally store the pre/post-replacement text.

---

## Feature 1 — Deterministic text replacements (#455 / #1533)

A rule-based find/replace pass over the transcript: literal or regex rules, plus
"magic commands" (`[uppercase]`, `[capitalize]`, `[date]`, `[time]`, and `[run]"cmd {text}"`).
Instant, offline, deterministic — complements (does not duplicate) the existing LLM
post-processing. Today `custom_words` is only a single-word dictionary, so this is a real gap.

### What needs to be done
- **Settings (`settings.rs`)**
  - Add `replacements_enabled: bool` and `text_replacements: Vec<Replacement>`.
  - `Replacement { search, replace, is_regex, enabled, trim_*, capitalization }` with
    `#[serde(default)]` + a `default_text_replacements()`.
- **Transform module (new: `audio_toolkit/replacements.rs` or extend `audio_toolkit/text.rs`)**
  - `apply_replacements(text, &[Replacement]) -> String`: literal + regex (use `regex` crate),
    trim handling, then magic-command expansion.
  - Magic commands as a small, extensible map (`[date]`, `[time]`, `[upper/lower/capitalize]`,
    `[nospace]`). `[run]` executes a shell command via `std::process::Command`.
- **Pipeline hook (`actions.rs::process_transcription_output`)**
  - Apply replacements to `final_text`. Decide order vs. LLM post-process — default:
    replacements run **after** LLM post-processing (deterministic fix-ups win). Make it a
    single, well-commented insertion so it's easy to reorder later.
- **Bindings + store + frontend**
  - Regenerate `bindings.ts`; add updater in `settingsStore.ts`; add a
    `change_text_replacements` command in `shortcut/mod.rs`.
  - New `src/components/settings/TextReplacements.tsx` (two fields by default, advanced
    options behind a disclosure; import/export JSON). i18n keys in `en/translation.json`.
- **Security**
  - `[run]` is arbitrary command execution. Gate it behind an **explicit opt-in toggle**
    (default off), show a clear warning, and never enable it from imported rule files
    without confirmation.

### Independence
**Fully independent.** Touches `actions.rs` (one function), `settings.rs`, a new transform
module, and new UI. Does **not** touch the recording pipeline, the transcribe API, or API
keys. Safe to build in parallel with everything else.

---

## Feature 2 — Secure API-key storage in OS keychain (#814)

Move provider secrets out of the plaintext `settings_store.json` into the OS keychain
(Windows Credential Manager / macOS Keychain / Linux Secret Service) via the `keyring` crate.
We now hold many keys (OpenAI, Anthropic, Groq, OpenRouter, Z.AI, Cerebras, Bedrock, Azure,
ElevenLabs, Brave, Firecrawl), so this matters — but it can be kept simple.

### Can it be done independently? — YES, and kept simple
It is **orthogonal to the transcription pipeline** and to #1/#3. It only touches
`settings.rs` plus the small, already-enumerated set of read/write sites. Recommended
minimal-blast-radius design that **keeps all 4 read sites unchanged**:

1. **New module `secret_store.rs`** wrapping `keyring`: `get(account) -> Option<String>`,
   `set(account, value)`, `delete(account)`. Service name = app id; account = e.g.
   `post_process:<provider_id>`, `web_search:<provider_id>`, `assistant_tts`.
2. **Stop persisting secrets to JSON** — mark the secret fields `#[serde(skip)]` so they
   never hit `settings_store.json`.
3. **Hydrate on load** — in `get_settings()`, after deserializing, fill `SecretMap`/
   `SecretString` from the keychain. Read sites keep doing
   `settings.post_process_api_keys.get(&provider.id)` unchanged.
4. **Write on set** — the 2 write sites (`commands/assistant.rs`, `shortcut/mod.rs`) write to
   the keychain instead of the store.
5. **One-time migration** — on first load, if old plaintext keys exist in the JSON, move them
   into the keychain and strip them from the JSON.

### Keep it simple (scope guardrails)
- ✅ Use the OS keychain primitive directly via `keyring`. That's the whole feature.
- ⚠️ **Hot-path caveat:** `get_settings()` runs on every action. Do **not** hit the keychain
  on every call — hydrate once into an in-memory cache (`OnceCell`/`Mutex`) and update the
  cache on write. This is the one correctness detail that keeps it both simple *and* fast.
- ❌ Don't over-build: no custom encryption envelopes, no master-password vault, no rotation
  system. The OS keychain is the trust boundary.
- 🔻 Linux fallback: Secret Service may be absent on headless/minimal setups — fall back to
  the current store with a logged warning so the app still runs.

### Independence
**Fully independent.** No overlap with the pipeline or #1. Build it whenever; it pairs
naturally alongside #1.

---

## Feature 3 — Eager / streaming transcription (#1515)

Transcribe segments **while** recording so text appears almost immediately. Biggest
perceived-latency win, but it restructures the core.

### What needs to be done
- **Audio recorder (`audio_toolkit/audio/recorder.rs`)** — emit rolling audio chunks during
  capture (not just one buffer at stop).
- **Transcription manager (`managers/transcription.rs`)** — add incremental/segment
  transcription (transcribe a segment, keep partial state). This changes the `transcribe`
  surface.
- **Coordinator/actions** — the `Recording` stage must now also transcribe in-flight;
  reconcile partial → final on stop. Re-entrancy and cancel handling get more complex.
- **Overlay (`src/overlay/`)** — render streaming partial text.
- **Paste/finalize** — only paste the finalized text; handle correction of earlier partials.
- **Interaction with #1** — text replacements must run on the **finalized** text, not on
  partials. Keep the replacement pass in `process_transcription_output` (the finalize path).

### Independence
**Not independent.** It rewrites the recorder + transcription manager + coordinator + actions
+ overlay — the shared core. High risk. Must be done **last and alone**, after #1/#2 are
merged and stable.

---

## Dependency map

```
#1 text replacements ──┐ (independent)
#2 keychain ───────────┘ ⟶ build in parallel, merge in any order (zero file overlap)
                          │
#3 streaming transcription ⟶ rewrites the shared core; do LAST, ALONE
```

- **No conflict:** #1 ↔ #2 (entirely different files).
- **Hard sequence:** #3 last — it's a core rewrite that touches the recorder, transcription
  manager, coordinator, actions, and overlay. Re-validate #1's replacement hook still runs on
  finalized text afterward.

---

## How to go about the update (recommended sequencing)

Build in small, independently mergeable branches. Each phase ships on its own and is
verifiable (build + targeted test) before the next.

**Phase A — quick, independent wins (parallel-safe)**
1. **#2 Secure API-key storage** first. It's contained, low-risk, high-trust, and gets the
   security upgrade in before more secrets accumulate. Land the in-memory cache + migration.
2. **#1 Text replacements** alongside it. Different files, zero conflict. Ship literal +
   regex + magic commands; gate `[run]` behind an off-by-default toggle.

> A is a clean first PR pair: one security branch, one feature branch, no shared files.

**Phase B — the core rewrite (last, alone)**
3. **#3 Streaming transcription.** Only after Phase A is merged and stable. Expect to touch
   the recorder, transcription manager, coordinator, actions, and overlay. Confirm #1's
   replacement pass still runs on the finalized text.

**Per-phase checklist**
- New `settings.rs` fields get `#[serde(default)]` + a default fn (back-compat with existing stores).
- Regenerate `bindings.ts` (tauri-specta) and add `settingsStore.ts` updaters for any new setting.
- All user-facing strings go through i18n (ESLint enforces it).
- `bun run lint` + `cargo fmt`/`clippy`; build with `bun run tauri dev` before opening a PR.
- Update `docs/PROGRESS.md` per phase.

**Why this order:** it front-loads the two zero-conflict items (security + replacements),
then quarantines the one invasive core rewrite (streaming) to the end where it can't
repeatedly break the others.
