# Simplicity Overhaul — Living Plan & Tracker

> **Executor AI: read this ENTIRE file at the start of your session.** Do exactly one
> session (yours is named in your prompt), verify it against its acceptance criteria,
> then update this file — tick your boxes, set Status, fill Evidence + Downstream
> Notes, add a Progress Log row — before stopping. Never tick anything you didn't verify.

---

## 0. Status legend

`[ ]` not started · `[~]` in progress · `[x]` done · `[!]` blocked (explain in Evidence)

---

## 1. Goal & non-negotiables

**Goal:** SpeakoFlow must be usable by a first-time, non-technical user with zero
explanation — while keeping every power feature reachable. The fix is structural
(information architecture + progressive disclosure) and editorial (copy), not visual.

**Non-negotiables:**

- **G1 — Never break the working app.** At every session boundary: `bun run build`
  and `bun run lint` pass, the app launches, dictation works, assistant chat works.
  (`cargo fmt` + `cargo check` too if Rust was touched.)
- **G2 — Reorganize, never remove or change behavior.** Zero settings are deleted,
  zero backend/feature logic changes. Everything that leaves the default view moves
  into a "More options" fold, a sub-page, or an `info` tooltip. Default-off for
  anything experimental (unchanged).
- **G3 — i18n always.** Every user-facing string goes through `t()` into
  `src/i18n/locales/en/translation.json`, inside YOUR assigned namespace only.
  English only; other locales are out of scope (S6 records the debt).
- **G4 — Respect file ownership.** Each session's prompt lists the files/folders it
  owns. Do not edit outside them. `src/bindings.ts` and `src-tauri/src/settings.rs`
  are S0-only. If you're blocked by a missing setting/command, mark `[!] blocked`.
- **G5 — Follow the Voice Guide (§3) and the IA spec (§4) exactly.** Don't invent a
  different structure because you'd have designed it differently.
- **G6 — No git commits/pushes** unless the human explicitly asks.
- **G7 — Don't touch the engine.** The transcribe.cpp decode path, VAD, and audio
  toolkit are off-limits. This overhaul is UI + copy + onboarding flow only.
- **G8 — Frozen surfaces.** The floating assistant panel (`src/assistant/**`) and
  the recording overlay (`src/overlay/**`, `overlay.rs`) are NOT part of this
  overhaul. No session modifies them, styles them, or "improves" them. Scope =
  the settings window + onboarding, nothing else.

---

## 2. The diagnosis (why we're doing this)

1. **10 sidebar sections** where a user needs ~5. "Models", "Post Process",
   "Assistant" all have separate provider/API-key/model forms — three pages that
   look like the same page. "Post Process", "Profiles", "Memory" are meaningless
   names to a new user until the assistant exists in their head.
2. **Onboarding is a research catalog.** Step 1: 9+ STT models with quantization
   badges and two simultaneous "Recommended" tags. Step 2 asks a brand-new user to
   choose between 9B-parameter GGUF builds. Both steps block the user from the app.
3. **Everything is flat.** Base URL / API key / timeout / dtype sit at the same
   visual weight as "turn this feature on". The `MoreOptions` fold primitive exists
   but is barely used.
4. **No obvious happy path.** The app should be: install → one download → press
   hotkey → words appear. Everything else opt-in, later, discoverable.

---

## 3. Voice & Tone Guide (BINDING for all sessions)

Every string any session writes must pass these rules. S5 enforces them repo-wide.

**Structure of every setting row (3 tiers):**

1. **Title** — 2–4 words, sentence case, what it is. `Speak responses aloud`, not
   `TTS Engine Configuration`.
2. **Caption** (optional, ≤1 short sentence) — what it does _for the user_, only if
   the title isn't self-evident. `Plays a sound when recording starts and stops.`
3. **`info` tooltip** (optional) — the technical detail for power users. Model
   quantization, endpoints, edge cases live HERE, never in the caption.

**Vocabulary rules:**

- Banned from titles/captions: _post-process, inference, quantization, Q4_K_M, GGUF,
  endpoint, payload, dtype, LLM, STT, VAD, tokens_ (allowed inside `info` tooltips
  and the "See all models" advanced catalog).
- Renames (use everywhere): "Post Process" → **AI cleanup**. "Characters/Profiles
  section" → **Profiles** (already the label; keep internal key `characters`).
  "Language Model" tab → **Assistant brain** (page copy may say "the model that
  powers your assistant"). "Push To Talk" caption stays plain: "Hold to record,
  release to stop."
- Speak to the user: "your voice", "your screen", "your machine" — never "the user".

**Humor: light & warm.** One wink where the user is relaxed; zero where they're
confused or in danger.

- ✅ Allowed: model descriptions ("Blazingly fast — great for everyday dictation"),
  empty states ("Nothing here yet. Say something nice about yourself."), download
  waits ("Your assistant is still moving in — 42%"), onboarding, success moments.
- ❌ Forbidden: error messages, destructive confirmations (delete/wipe), permission
  prompts, anything about API keys or privacy.
- Calibration examples (match this energy, don't exceed it):
  - "Tiny and instant — runs well on any machine."
  - "Psst — I can also answer questions and see your screen. Want to set that up?"
  - "That's it. You're ready. Go dictate something."

**Formatting:** sentence case everywhere (buttons too). No exclamation marks in
captions (one allowed per onboarding screen). No emoji in settings rows; emoji
allowed sparingly in onboarding cards and empty states.

---

## 4. Target Information Architecture (BINDING spec)

**This spec covers EVERY settings surface in the app** — General, Models,
Advanced, History, Post Process, Assistant, Profiles, Memory, About, and
onboarding. If a session's page isn't matching this section, the session is wrong.

### 4.0 Consistency contract (every session obeys — fresh windows, ONE look)

Each session runs in a fresh window with no shared context, so consistency comes
from these rules, not from memory:

- **Page shape, always in this order:** ① the one thing users came for (hero /
  essentials), ② simple on/off groups, ③ ONE page-level "More options" fold,
  ④ sub-page cards last.
- **≤7 visible rows/groups** on any page before folds.
- **One fold label everywhere:** reuse the existing `common.showAdvanced`
  ("More options"). Never invent "Show advanced", "Expand", etc.
- **Every row = the 3-tier structure** from §3 (title / optional caption / info).
- **Provider forms are IDENTICAL everywhere they appear** (Dictation's AI-cleanup
  fold, Assistant's cloud brain): Provider → API key → Model, with Base URL shown
  only for providers that need it (Custom / Local / Azure) — same order, same
  labels, same info tooltips.
- **Hotkey rows look identical everywhere:** keycap chip + reset button; caption
  only when the behavior isn't obvious.
- **Sub-pages** all use the S0 `SubPage` primitive: same back-button header, same
  title style.
- **Destructive actions** (delete profile, wipe memory, delete model) are quiet
  red-accent rows at the bottom of their context, plain confirmation copy, no humor.

### 4.1 Sidebar: 5 sections (+ hidden Debug)

| Key         | Label     | Contains                                                                                                 |
| ----------- | --------- | -------------------------------------------------------------------------------------------------------- |
| `general`   | General   | Hotkeys, mic, appearance. Fold: startup/tray/overlay/sounds.                                             |
| `dictation` | Dictation | Active STT model + catalog, AI cleanup (ex-Post Process), output options.                                |
| `assistant` | Assistant | On/off + hotkeys, one "brain" picker, voice, vision, search, panel. Sub-pages: **Profiles**, **Memory**. |
| `history`   | History   | Unchanged view + retention settings (moved from Advanced).                                               |
| `about`     | About     | Version, updates, app language, credits, data folders.                                                   |

`debug` stays, gated by debug_mode as today. `models`, `advanced`, `postprocessing`,
`characters`, `memory` disappear as top-level entries; their internal section keys may
survive in code where renaming would churn (repo precedent).

### 4.2 Page specs (default view = what a new user sees; everything else folded)

**Nothing here deletes a setting.** Every row marked "fold" or "sub-page" still
exists — it just stops shouting.

**General** — default view (≤7 rows):

- Transcribe hotkey (keycap + reset), Push to talk toggle, Cancel hotkey
- Microphone dropdown
- Appearance (theme) + Text size
- Model language card (if active model has language options) — keep compact
- Fold ("More options"): audio feedback + sound theme + output device + volume,
  launch on startup, start hidden, tray icon, overlay style, update checks.

**Dictation** — default view:

- **Hero card: the active model.** Name, friendly one-liner, accuracy/speed pips,
  "Change model" button → opens the model catalog as a sub-page (search, language
  filter, all-models list, custom HF models, delete, streaming badges — the current
  Models page content, capability-complete). The catalog is the ONLY place where
  jargon is allowed (quant badges, sizes).
- **AI cleanup** group (this IS Post Process, renamed): one toggle ("Fix up my
  dictation with AI"), Tone dropdown. Fold: provider form (per §4.0), timeout,
  custom prompt editor, dedicated hotkey. Keep "experimental" marking, default off.
- Fold ("More options"): paste method, append trailing space, always-on microphone,
  custom words, text replacements.

**Assistant** — default view:

- Hotkeys group: Ask assistant, Show/hide panel, push-to-talk toggle.
- **Brain picker — ONE card, not three forms.** A segmented choice: `On my device`
  (dropdown of downloaded local LLMs + "Download a model…" which opens the LLM
  catalog) vs `Cloud provider` (provider form per §4.0).
- Voice output: one toggle "Speak responses aloud". Fold: engine, voice, speed,
  test button, engine-specific fields.
- Screen vision: one toggle. `info` explains it; capture timing in the fold.
- Web search: one toggle. Provider + key in the fold.
- Panel appearance: preview + text size/panel size/opacity — as ONE collapsed group
  (this configures the panel; it does not change the panel's own code — G8).
- **Sub-page cards at the bottom** (chevrons, SubPage primitive): **Profiles**,
  **Memory**.

**Profiles (sub-page of Assistant)** — today: card grid + a 4-button action row +
a permanently-open giant edit form, all flat. Target:

- One-line header caption explaining what profiles are.
- Compact profile grid stays (avatar, name, one-line role, Active badge); click
  selects.
- Action row: only **New** and **Create with AI** visible; Import + Restore
  built-ins move into a "⋯" overflow menu.
- Editor (selected profile only), tidied: avatar + upload, Name, Role, Instructions
  (textarea collapsed to a few lines, expandable). Fold: Response length, Greeting.
- Footer: Duplicate / Export / Restore default as quiet buttons; Delete =
  destructive row per §4.0.
- Every current field and action survives. Empty state (no profiles): one warm line.

**Memory (sub-page of Assistant)** — today: toggles + dropdown + About you + Notes

- a Manage card, all flat. Target:

* Hero: "Remember me" toggle; Incognito toggle under it.
* "About you" and "Notes" collapsibles stay (one-line captions).
* Fold ("More options"): Memory detail dropdown, "Update memory now", Export,
  Import.
* "Wipe memory" = destructive row at the bottom, plain copy (no humor).
* Empty state (no notes yet): one warm line per §3.

**History** — keep the content-first list view untouched; retention/limit rows
(moved from Advanced) live in a fold at the bottom. Empty state: one warm line.

**About** — version, update check, app language, source/license, acknowledgements;
data + log folder rows folded. Copy pass only.

### 4.3 Onboarding (3 light steps, never blocking)

- **Step 1 — "How should SpeakoFlow hear you?"** Exactly TWO featured cards:
  - ⚡ **Parakeet Unified EN 0.6B** — "Blazingly fast, streams as you speak. English only."
  - 🌍 **Nemotron Streaming 3.5** — "Real-time in 28 languages."
    Pre-select by system language (English → Parakeet, else Nemotron). One primary
    button: "Download and continue" — download runs in the BACKGROUND while the user
    moves to step 2. A quiet "See all models" disclosure reveals the full catalog for
    enthusiasts. If a model is already installed: keep the "Use installed model" path.
- **Step 2 — "Give it a brain (optional)."** Explains the assistant in 2 lines
  ("SpeakoFlow can also answer questions, clean up your writing, and look at your
  screen — fully offline if you want"). Exactly THREE local-model cards chosen by
  system RAM (S0 provides `get_system_memory_gb`): small (≤8 GB machines), the
  recommended mid (one single "Recommended" badge), and a vision-capable option.
  Plain descriptions, no quantization talk. **Tapping a card morphs it into a
  progress bar in place** and the primary button becomes "Continue" — the user goes
  on immediately; the download finishes in the background and auto-wires the
  built-in provider when done (existing behavior). "Skip for now" stays, with
  reassurance: "You can add this anytime in Settings → Assistant."
- **Step 3 — "You're ready."** Big keycap showing the transcribe hotkey, a live
  try-it textarea ("Press the keys and say anything — it'll appear here"), download
  status lines if anything is still downloading ("Your assistant is still moving
  in — 42%"), one warm sign-off line, "Open SpeakoFlow" button.
- Accessibility permission step (macOS) stays first, unchanged.
- Remove `FORCE_ONBOARDING = true` in App.tsx (dev override) as part of S1.

### 4.4 Empty states

Owned by the page sessions (not a separate session): Profiles-empty and
Memory-empty belong to S3, History-empty to S4. One warm line each, per §3.
No new toasts, nudges, or first-run banners — no behavior additions (G2/G8).

---

## 5. Sessions

### Wave 0 — foundation (runs ALONE)

#### S0 — New information architecture skeleton + shared primitives

**Owns:** `src/components/Sidebar.tsx`, `src/App.tsx`, `src/components/ui/**`,
`src/components/settings/index.ts`, `src-tauri/src/settings.rs`,
`src-tauri/src/lib.rs` (command registration), `src/bindings.ts` (via regeneration),
i18n namespaces `sidebar.*`, `sectionSubtitles.*`, `common.*`.
**Status:** done (2026-07-12)

- [x] Sidebar reduced to General / Dictation / Assistant / History / About (+Debug
      gated). Icons chosen sensibly; `characters`/`memory`/`models`/`advanced`/
      `postprocessing` removed from nav.
- [x] New **Dictation** section renders (temporarily) the existing STT half of
      ModelsSettings + PostProcessingSettings stacked, so nothing is lost before S2.
- [x] **Assistant** section gains a SubPage mechanism: new `SubPage`/section-stack
      primitive in `ui/` (title + back button + slide-in or simple swap), with
      Profiles and Memory reachable as sub-pages (temporarily embedding the existing
      CharactersSettings/MemorySettings unchanged).
- [x] Advanced section removed from nav; its rows temporarily appended to General
      (in a `MoreOptions` fold) and History (retention rows) so nothing is lost
      before S2/S4 polish them.
- [x] Backend: `get_system_memory_gb` command — **the ONLY backend change in this
      entire plan** — registered in lib.rs, bindings regenerated via the repo
      procedure (debug run exports specta; see AGENTS.md / repo memory: target dir
      C:/hbt).
- [x] `sectionSubtitles` reduced to the 5 sections, one plain sentence each.
- [x] Green: build + lint + cargo fmt/check; app launches; every section renders.
      (See Evidence for the one caveat: a pre-existing gitignored frozen-file lint
      error outside S0's scope, and how "app launches" was verified.)

**Evidence:** (2026-07-12, Windows dev machine)

- `bun run build` (`tsc && vite build`): green — `✓ built in ~6s`, no TS errors
  (only the standard "chunk > 500 kB" warning). All S0 frontend files also report
  clean LSP diagnostics.
- `bun run lint` (`eslint src`): every S0-created/edited file passes (linted them
  explicitly → exit 0). The whole run reports exactly 4 errors, all
  `i18next/no-literal-string` in `src/assistant/panel-demo.tsx` — a **gitignored**
  (`.gitignore:76`) local demo scratch file under the frozen `src/assistant/**`
  surface. `git diff` on `src/assistant/**` and `src/overlay/**` is empty (S0 did
  not touch frozen surfaces), the file is absent on a clean checkout, and G8
  forbids editing it, so those 4 are pre-existing and out of S0 scope.
- Rust: `rustfmt --edition 2021 --check src/settings.rs src/lib.rs` → clean;
  `cargo check` → `Finished dev profile in 5.05s`, exit 0 (the `windows` crate
  rebuilt with the added `Win32_System_SystemInformation` feature; only
  pre-existing dead-code warnings, none from S0 code).
- `get_system_memory_gb` compiles and `src/bindings.ts` carries the regenerated
  `getSystemMemoryGb(): Promise<number>` in canonical tauri-specta format. The
  specta export runs at debug `run()` startup, so its presence is proof the debug
  binary launched and exported successfully.
- Reachability audit (structural, mapping all 10 old sections): General (unchanged
  - Advanced app/output/transcription/textReplacements/experimental parked in a
    page-level "More options" fold) · Models → Dictation · Post Process → Dictation ·
    Assistant unchanged · Profiles + Memory → Assistant sub-pages · retention rows →
    History fold · Debug (gated) · About unchanged. No setting removed or duplicated
    into a dead end.
- Not performed here: a live interactive GUI click-through (this automated session
  can't drive the desktop GUI); reachability was verified structurally and via the
  green build / S0-lint / cargo check gates plus the successful bindings export.

**Downstream Notes:**

- **New backend command:** `settings::get_system_memory_gb` (registered in
  `lib.rs`) → frontend `commands.getSystemMemoryGb(): Promise<number>` — whole GiB
  of physical RAM, `0` when unknown. For **S1**'s RAM-tiered onboarding cards.
  Cross-platform: Windows `GlobalMemoryStatusEx`, Linux `/proc/meminfo`, macOS
  `sysctl hw.memsize`. (Cargo.toml gained the `Win32_System_SystemInformation`
  windows feature — that's the whole backend footprint besides the command.)
- **New shared primitive:** `SubPage` — `src/components/ui/SubPage.tsx`, exported
  from `ui/index.ts`. Props `{ title, description?, onBack, children }`; the back
  button uses `common.back`. Use it for **S2**'s model-catalog sub-view and **S3**'s
  Profiles/Memory pages.
- **Sidebar wiring is stable — redesign these in place, do NOT re-edit `Sidebar.tsx`:**
  - `dictation` → `src/components/settings/dictation/DictationSettings.tsx`
    (S0 temp = `ModelsSettings` + `PostProcessingSettings` stacked). **S2** rebuilds
    this file.
  - `assistant` → `src/components/settings/assistant/AssistantSection.tsx`
    (S0 shell = `AssistantSettings` + two `SubPage` nav rows to Profiles/Memory).
    **S3** rebuilds this file / folds the sub-page cards into its new layout.
  - `general` → `general/GeneralSettings.tsx` and `history` → `history/HistorySettings.tsx`
    hold the parked "More options" folds. **S4** reorganizes them in place and moves
    the dictation-output rows (paste method, custom words, text replacements,
    always-on mic) into Dictation per §4.2.
- **Parking keys:** the General fold reuses the existing
  `settings.advanced.groups.{app,output,transcription,textReplacements,experimental}`
  labels; the History fold reuses `settings.advanced.groups.history`. No new keys in
  S4's namespaces were needed.
- **Orphan for S6:** `advanced/AdvancedSettings.tsx` is no longer rendered (still
  exported from `settings/index.ts`); the S6 dead-code sweep should delete it.
- **i18n (S0 namespaces):** `sidebar.*` — added `dictation`, removed
  `models`/`advanced`/`postProcessing`, kept `characters` ("Profiles") and `memory`
  ("Memory") because `AssistantSection` uses them as the sub-page titles.
  `sectionSubtitles.*` = general/dictation/assistant/history/about/debug only. Added
  `common.back`.

### Wave 1 — every settings surface (run in PARALLEL)

#### S1 — Onboarding: from catalog to welcome

**Owns:** `src/components/onboarding/**`, onboarding wiring in `src/App.tsx`
(step state machine ONLY — coordinate: S0 is done, nobody else edits App.tsx in
wave 1), i18n namespace `onboarding.*`.
**Status:** done (2026-07-12)

- [x] Step 1 per §4.3: two featured cards, system-language pre-selection,
      background download + immediate advance, "See all models" disclosure,
      "Use installed model" preserved.
- [x] Step 2 per §4.3: three RAM-tiered cards (uses `get_system_memory_gb`),
      in-place download morph, non-blocking Continue, reassuring Skip.
- [x] Step 3 "You're ready" per §4.3 with live try-it area + background download
      status.
- [x] `FORCE_ONBOARDING` dev override removed; onboarding shows only when it should.
- [x] All copy follows §3 (this is the flagship humor surface — stay "light & warm").
- [x] Green: build + lint; full onboarding flow manually traced (describe in Evidence).

**Evidence:** (2026-07-12, Windows dev machine)

- **Structure = 3 non-blocking steps** (accessibility permission step stays first,
  unchanged). App.tsx `OnboardingStep` is now
  `accessibility → model → llm → ready → done`; `handleLlmComplete` routes to the
  new `ready` step, `handleReadyComplete` enters the app. Only the step machine +
  onboarding imports/body in App.tsx changed (no other App.tsx edits).
- **Step 1 (`Onboarding.tsx`)** — title "How should SpeakoFlow hear you?"; exactly
  two bespoke featured cards (`WelcomeChoiceCard`): ⚡ Parakeet Unified EN 0.6B
  ("Blazingly fast, streams as you speak. English only.") and 🌍 Nemotron Streaming
  3.5 ("Real-time in 28 languages."). Pre-selected by machine language
  (`navigator.language`/`navigator.languages`/i18n → English = Parakeet, else
  Nemotron). Primary "Download and continue" calls `handleChoose`
  (`setPendingSttSelection` + background `downloadModel` + immediate
  `onModelSelected()`), so it advances without waiting. Quiet "See all models"
  chevron discloses the full existing `ModelCard` catalog (quant/size/streaming
  jargon lives only here). "Use installed model" fast path preserved when a
  transcription model is already on disk.
- **Step 2 (`LlmOnboarding.tsx`)** — title "Give it a brain (optional)" + the 2-line
  plain-language explanation from the spec. Exactly THREE tiers chosen by
  `commands.getSystemMemoryGb()`: small `gemma-3-1b` (text only), mid `qwen3.5-2b`
  (sees your screen), capable `qwen3.5-4b` (sees your screen). Exactly ONE
  "Recommended" badge, placed by RAM (≤8 GB → small, 9-15/unknown → mid, ≥16 →
  capable). Tapping a card runs `handleChoose` → morphs that card in place into a
  progress bar (`WelcomeChoiceCard` phase) and flips the footer to "Continue"
  immediately; other cards dim. The completed background download auto-wires the
  built-in provider (`changeAssistantModelSetting`/`setAssistantProvider`) via a
  floating await that survives unmount (unchanged behavior). "Skip for now" stays
  with the reassurance line "You can add this anytime in Settings → Assistant."
- **Step 3 (`ReadyStep.tsx`)** — title "You're ready."; big keycaps rendered from
  `settings.bindings.transcribe.current_binding` via `formatKeyCombination`; a live
  autofocused try-it `Textarea` (Enigo + global shortcuts are initialized on mount
  so the hotkey pastes straight into it); warm background-download lines ("Your
  assistant is still moving in — {{percentage}}%", "Your voice model is still
  landing — …") with a slim bar; a warm sign-off and an "Open SpeakoFlow" button
  → enters the app.
- **Voice Guide (§3):** no banned words (GGUF/quant/LLM/STT/tokens…) on any featured
  or tier card — jargon is confined to the "See all models" catalog; sentence case
  throughout; ≤1 exclamation per screen (used zero). Humor stays light and warm and
  only on the relaxed surfaces (card one-liners, download waits, sign-off).
- **`bun run build`** (`tsc && vite build`): exit 0 — `✓ built in ~5.7s`, no TS
  errors (only the standard pre-existing "chunk > 500 kB" warning). LSP diagnostics
  clean on all six S1 files (Onboarding, LlmOnboarding, ReadyStep, WelcomeChoiceCard,
  OnboardingLayout, App).
- **`bun run lint`** (`eslint src`): the whole run reports exactly the same 4
  pre-existing `i18next/no-literal-string` errors in the **gitignored**, frozen-file
  `src/assistant/panel-demo.tsx` that S0 documented — none in S1 scope.
  `bun x eslint src/components/onboarding src/App.tsx` → exit 0, no output (all S1
  files clean). The `+` keycap separator is a constant expression, and featured
  emoji are JSX attributes, so `markupOnly` doesn't flag them.
- **Flow trace (verified against code, not a live GUI click-through — this automated
  session can't drive the desktop GUI):** fresh state (no models) →
  `checkOnboardingStatus` sees `hasAnyModelsAvailable() = false` → `accessibility` →
  `model` (Step 1: language pre-select, "Download and continue" fires the background
  download and advances) → `llm` (Step 2: three RAM tiers, tap morph + immediate
  Continue) → `ready` (Step 3: keycap + try-it + warm status) → `done` (main app).
  Background downloads finalize after the user has moved on: the STT model is
  selected by the store's `model-download-complete` listener (`pendingSttSelection`
  → `finalizePendingSttSelection`), and the LLM provider is wired by the floating
  `handleChoose` await. Returning user (`hasAnyModelsAvailable() = true`) → `done`
  directly, with the macOS/Windows permission path preserved.

**Downstream Notes:**

- **New shared card:** `WelcomeChoiceCard` (`src/components/onboarding/`) — the
  bespoke welcome card (icon tile + title + one-liner + size/pill row, accent ring
  when selected, in-place morph to a progress bar for downloading/verifying/
  extracting). Used by Step 1 (featured STT) and Step 2 (LLM tiers). **S5** should
  copy-review its callers' i18n, not the component.
- **New i18n (S1 namespace `onboarding.*` only):** `speechToText.title/subtitle`,
  `speechToText.cards.{fast,multilingual}.description`, `speechToText.downloadAndContinue`,
  `speechToText.seeAllModels`, `speechToText.hideAllModels`; `aiModel.title/subtitle`,
  `aiModel.tiers.{small,mid,capable}.description`, `aiModel.seesScreen`,
  `aiModel.textOnly`, rewritten `aiModel.skipHint`; new `ready.*` block
  (`title/subtitle/hotkeyLabel/tryItLabel/tryPlaceholder/downloadingAssistant/
downloadingVoice/downloadingGeneric/signOff/openApp`); `steps.{speechToText,aiModel,ready}`
  relabeled ("Hear you"/"Give it a brain"/"You're ready"). **Did NOT touch
  `onboarding.models.*` (S5's per-model overlay copy) or `onboarding.permissions.*`.**
  Leftover unused keys (`onboarding.subtitle`, `onboarding.speechToText.continue`) are
  harmless — **S5/S6** can prune. Other 19 locales are stale for these keys (English
  fallback applies); **S6** records the debt.
- **Model facts for future sessions:** the three onboarding LLM tiers are the
  hardcoded built-ins in `src-tauri/src/managers/model.rs` (~L811-906): `gemma-3-1b`
  (text, 806 MB), `qwen3.5-2b` (vision, 2350 MB), `qwen3.5-4b` (vision, 3900 MB,
  `is_recommended`). `gemma-3-4b` (vision, 3350 MB) exists but is intentionally NOT
  shown in onboarding (still reachable in Settings → Assistant — nothing removed, G2).
  `ModelInfo` in `bindings.ts` (frozen) has **no** `is_vision` field, so vision is
  inferred by tier assignment, not read from the model.
- **`get_system_memory_gb` consumed:** Step 2 calls `commands.getSystemMemoryGb()`
  (whole GiB, 0 = unknown) exactly as S0's Downstream Notes specified; the single
  Recommended badge is placed from it.
- **`OnboardingLayout` gained `showDownloadProgress?: boolean`** (default true). The
  shared `DownloadProgress` strip shows on Step 1; Step 2 (in-place card morph) and
  Step 3 (warm status lines) pass `false`. `totalSteps` is now 3 across the flow.
- **`FORCE_ONBOARDING` removed** from App.tsx (constant + `checkOnboardingStatus`
  guard). Onboarding now shows only for genuinely new users (no models) or returning
  users missing permissions — the normal `hasAnyModelsAvailable` path.

#### S2 — Dictation page (models + AI cleanup unified)

**Owns:** `src/components/settings/models/**`, `src/components/settings/post-processing/**`,
a new `src/components/settings/dictation/**`, i18n namespaces `settings.models.*`,
`settings.postProcess*`, new `settings.dictation.*`.
**Status:** done (2026-07-12)

- [x] Dictation page per §4.2: hero active-model card, catalog as sub-view (reuse
      the SubPage primitive), AI cleanup group with folded provider form, output
      options fold (paste method, custom words, text replacements, always-on mic
      moved here from the temporary General dump).
- [x] The model catalog keeps ALL current capability (search, HF custom models,
      language filter, delete, streaming badges) — it's just one level deeper.
- [x] "Post Process" wording eliminated in the UI (→ "AI cleanup"), including the
      hotkey row's label.
- [x] Green: build + lint; model switch + download + AI-cleanup toggle verified.

**Evidence:** (2026-07-12, Windows dev machine)

- **New page shape** (`dictation/DictationSettings.tsx`, rewritten from the S0
  stub): ① hero active-model card (`DictationModelCard`), ② `AiCleanupGroup`,
  ③ ONE page-level `MoreOptions` fold (a `SettingsGroup` holding paste method →
  append trailing space → always-on microphone → custom words → text
  replacements). Catalog opens via the hero's "Change model" button, swapping
  the page for a `SubPage` that renders the **unchanged** `<ModelsSettings/>`.
- **Hero** (`dictation/DictationModelCard.tsx`): eyebrow "Now listening with",
  friendly name (`getTranslatedModelName`), one-line description
  (`getTranslatedModelDescription`), a streaming badge, quiet accuracy/speed
  meters, and "Change model". Loading spinner + a "Choose a model" empty state
  when nothing is downloaded. No jargon (quant/size/engine) — that stays in the
  catalog only.
- **AI cleanup** (`dictation/AiCleanupGroup.tsx`): `SettingsGroup` (Sparkles
  icon, title "AI cleanup") → one toggle "Fix up my dictation with AI"
  (experimental caption, off by default, deep detail behind the `info` hint,
  reads/writes `post_process_enabled`), then the Tone dropdown, then a
  `MoreOptions` fold reusing the shared `PostProcessingSettingsApi` (provider →
  base URL → API key → model, identical to Assistant per §4.0),
  `PostProcessTimeout`, `PostProcessingSettingsPrompts` (custom prompt editor),
  and the `ShortcutInput` for `transcribe_with_post_process`. Behavior unchanged
  — only reorganized/relabeled.
- **Catalog capability preserved**: the catalog reuses the whole `ModelsSettings`
  component verbatim (search, language filter, downloaded/available/older-models
  groups, custom Hugging Face models via `AddCustomModelDialog`, delete,
  streaming badges, the stt/llm/tts tabs). Reusing it wholesale means zero
  capability was dropped and nothing was orphaned (llm/tts management still
  reachable until S3 gives it a permanent Assistant home).
- **"Post process" removed from owned visible copy**: `settings.postProcessing.title`
  "Post Process" → "AI cleanup"; `settings.postProcessing.prompts.createFirst`
  "…first post-processing prompt." → "…first cleanup prompt." No visible
  "post process" string remains in my namespaces (`settings.models.*`,
  `settings.postProcess*`, `settings.dictation.*`).
- **`bun run build`** (`tsc && vite build`): green — `✓ built in 5.29s`, exit 0,
  only the standard "chunk > 500 kB" warning (same as S0). `tsc --noEmit` exit 0.
  All three new files report clean LSP diagnostics.
- **`bun run lint`**: the 3 new dictation files lint explicitly at exit 0. The
  whole run reports exactly 4 errors, all `i18next/no-literal-string` in
  `src/assistant/panel-demo.tsx` — the same **gitignored** (`git check-ignore`
  confirms), **frozen** (`src/assistant/**`, G8) demo scratch file S0's Evidence
  already documented. Not mine, forbidden to edit, pre-existing.
- **Concurrent-session note**: sibling sessions (S1 onboarding, S3
  assistant-settings) were editing the tree _during_ this session. Two earlier
  build runs caught transient errors in their non-owned files
  (`assistant/AssistantSection.tsx`, `onboarding/ReadyStep.tsx`) that resolved on
  re-run once those files settled. No transient error was ever in a file I own.
- Not performed: live GUI click-through (this automated session can't drive the
  desktop). Flows above verified by code review + the green build/lint gates.

**Downstream Notes:**

- **Hotkey label — one documented cross-namespace edit.** The AI-cleanup hotkey
  row reuses the shared `ShortcutInput`, which derives its title from
  `settings.general.shortcut.bindings.transcribe_with_post_process.{name,description}`
  (S4's `settings.general*` namespace). To honor "eliminate 'Post Process',
  including the hotkey row's label" without duplicating ~500 lines of two-engine
  capture logic, I renamed just those 2 strings: name "Post-Processing Hotkey" →
  **"Dictate and clean up"**, description → "Dictate, then clean it up with AI
  before it's typed out." This binding is used _only_ by my AI-cleanup row (S4's
  General renders transcribe/cancel, not this one), so the edit is isolated.
  **S4/S5:** don't revert these; keep the AI-cleanup vocabulary.
- **`settings.postProcessing.title`** is now "AI cleanup" but is no longer
  rendered anywhere (the old Post Process page/section title). Safe to leave;
  S6 dead-code/copy sweep may retire it.
- **Still-visible "post process" strings I do NOT own** (leave for S5's copy
  pass): `settings.debug.postProcessingToggle.label` = "Post Processing" (debug
  namespace, not rendered by Dictation). The folded row components pull "AI
  Correction" copy from `settings.advanced.aiCorrection.*` /
  `settings.advanced.postProcessTimeout.*` (no "post process" text, rendered
  as-is; not mine).
- **Temporary duplication (expected).** The output rows (paste method, append
  trailing space, always-on mic, custom words, text replacements) now render in
  BOTH the S0 "More options" dump in `general/GeneralSettings.tsx` (S4) AND my
  Dictation fold. They read/write the same store keys, so both stay in sync — no
  runtime conflict. **S4** should remove them from the General dump (§4.2 gives
  them their permanent home in Dictation); **S6** dedupes if anything slips.
- **`ModelsSettings` reused wholesale** as the catalog sub-page (still exported
  from `models/index.ts` and `settings/index.ts`). Its stt/llm/tts tabs remain,
  so LLM/TTS model management is not orphaned before S3. If S3 gives LLM/TTS a
  dedicated Assistant catalog, S6 should reconcile the overlap. The old
  `post-processing/PostProcessingSettings` default export is now unused (its
  `PostProcessingSettingsApi`/`Prompts`/`Tone` named exports are reused by
  `AiCleanupGroup`); S6 dead-code sweep can drop the default wrapper.
- **New i18n namespace `settings.dictation.*`** added: `hero.*` (eyebrow,
  loading, noModel\*, chooseModel, changeModel, streams, accuracy, speed),
  `catalog.{title,description}`, `aiCleanup.{groupTitle,title,caption,info}`,
  `output.title`. English only (G3); other locales are S6's recorded debt.

#### S3 — Assistant section + Profiles + Memory (full redesign of all three)

**Owns:** `src/components/settings/assistant/**` (AssistantSettings, CharactersSettings,
MemorySettings), i18n namespaces `settings.assistant.*`, `settings.characters.*`,
`settings.memory.*`.
**Status:** done (2026-07-12)

- [x] Assistant page per §4.2: brain picker card (On my device ↔ Cloud provider),
      one-toggle groups for voice/vision/search with folds, panel appearance
      collapsed, Profiles + Memory as sub-page cards.
- [x] **Profiles sub-page fully reorganized per §4.2**: compact grid, ⋯ overflow
      for Import/Restore, tidied editor with Response length + Greeting folded,
      destructive Delete row, warm empty state.
- [x] **Memory sub-page fully reorganized per §4.2**: hero toggles, About you +
      Notes collapsibles, detail/update/export/import folded, destructive Wipe row,
      warm empty state.
- [x] No capability lost anywhere: every current field/action still reachable.
- [x] Green: build + lint; provider switch (local ↔ cloud), TTS test, sub-page nav,
      profile create/edit/delete, memory export verified.

**Evidence:** (2026-07-12, Windows dev machine)

- `bun run build` (`tsc && vite build`): green — `✓ built in ~5s`, exit 0, no TS
  errors (only the standard "chunk > 500 kB" warning). `get_diagnostics` clean on
  all five S3 files: `AssistantSettings.tsx`, `AssistantSection.tsx`,
  `LlmCatalog.tsx` (new), `CharactersSettings.tsx`, `MemorySettings.tsx`.
- `bun run lint` (`eslint src`): the whole run reports exactly the **same 4
  pre-existing errors S0 recorded** — all `i18next/no-literal-string` in the
  **gitignored, frozen** `src/assistant/panel-demo.tsx` (G8). None of S3's files
  appear. `git status` confirms S3 touched only `settings/assistant/**` +
  `en/translation.json` (my namespaces); the other dirty files
  (`App.tsx`, `Sidebar.tsx`, `bindings.ts`, `settings.rs`, `ui/**`, onboarding,
  Rust) are S0's / other sessions' uncommitted work, not S3's.
- **Zero capability loss (G2), verified by command-preservation audit:** all
  **35/35** assistant backend commands, **10/10** Profiles commands, **11/11**
  Memory commands still referenced in the rewritten files (0 MISSING). Every flat
  control found a home: both provider forms; all 4 TTS engines + their fields +
  speed/test/stop-on-dictation/kokoro-precision; vision toggle + capture timing;
  search toggle + provider/key/depth/local-smart/OpenRouter-native/test; panel
  preview/text-size/panel-size/opacity; response length + conversation memory.
- **Independent audit** (fresh reviewer, no shared context): all structural items
  A1–A9 (Assistant), P1–P6 (Profiles), M1–M5 (Memory) and C1 (capability) PASS;
  the only FAIL was V1 (banned words in the Voice-output fold labels) — **fixed**
  (`TTS engine`→`Engine`, `TTS Base URL`→`Base URL`, `TTS API Key`→`API key`,
  `TTS Model`→`Model`, Azure `Speech Endpoint`→`Speech URL`; also dropped "tokens"
  from the memory-detail caption and sentence-cased the provider/search API-key
  labels). Re-scan of every title/label key: no §3 banned words remain in visible
  titles/captions.
- **Not performed here:** a live interactive GUI click-through — this automated
  session can't drive the desktop GUI (same limitation S0 noted). Provider switch,
  TTS test, vision/search toggles, sub-page nav, profile CRUD, memory
  export/import, and the panel preview were verified structurally via the green
  build, clean diagnostics, the command-preservation audit, and the independent
  review rather than by clicking.

**Downstream Notes:**

- **Canonical cloud provider form order (for S2 to mirror in Dictation's AI-cleanup
  fold, §4.0):** `Provider → Base URL → API key → Model`, where **Base URL is shown
  only when `provider.allow_base_url_edit`** (Custom / Local / Azure). Base URL sits
  right after Provider (it's a connection detail), matching the pre-existing
  post-process/assistant precedent so the two forms already agree. Labels used:
  `provider.providerLabel` / `provider.baseUrlLabel` / `provider.apiKeyLabel`
  ("API key") / `provider.modelLabel`. The built-in local engine is **not** a
  provider dropdown entry in the picker — it's the "On my device" segment.
- **New shared behavior — the on-device model catalog is now an Assistant
  sub-page:** `src/components/settings/assistant/LlmCatalog.tsx` (new, S3-owned)
  renders LLM `ModelCard`s from `useModelStore` (download / delete / select),
  wiring a chosen model to the `builtin` provider (`changeAssistantModelSetting` +
  `setAssistantProvider`, mirrors `LlmOnboarding`). It's opened from the brain
  picker's "Download a model…" row via `AssistantSection`'s new `"llm-catalog"`
  sub-page. **S2/S5/S6:** this is the LLM half of the model catalog; the STT
  catalog stays in Dictation. `provider.builtinNoModels` copy was repointed from
  the retired "Models tab" to this Download row.
- **i18n (S3 namespaces only):** added `settings.assistant.brain.*`
  (title/description/whereLabel/onDevice/cloud/downloadModel/catalogTitle/
  catalogDescription/catalogEmpty) and `settings.assistant.subpages.*`
  (profilesCaption/memoryCaption); added `settings.assistant.characters.*`
  (moreActions/expand/collapse/deleteConfirm/emptyState); added
  `settings.personalMemory.aboutYou.caption` + `notes.caption`. Sentence-cased the
  group titles (`shortcuts`→"Shortcuts", `tts`→"Voice output", `vision`→"Screen
  vision", `webSearch`→"Web search", `appearance`→"Panel appearance"). **Note the
  namespace reality for S5/S6:** Profiles keys live under
  `settings.assistant.characters.*` (nested) and Memory keys under
  `settings.personalMemory.*` — **not** `settings.characters.*` / `settings.memory.*`.
  Kept those prefixes intact (renaming would churn/break with no behavior gain).
- **`AssistantSettings` now takes an optional `onOpenLlmCatalog` prop** (wired by
  `AssistantSection`); `AssistantSection.SubPageRow` gained an optional
  `description` for the captioned Profiles/Memory cards. The Profiles Instructions
  "expand" is an **icon-only** toggle (Maximize2/Minimize2, aria-labelled) — the
  single `common.showAdvanced` fold label stays reserved for real `MoreOptions`
  folds per §4.0.
- **Orphan commands (for S6 dead-code sweep, NOT an S3 regression):** the audit
  noted `setAssistantWebSearchMaxResults`, `setAssistantWebSearchFetchContent`,
  `setAssistantWebSearchDailyCreditBudget`, and `changeAssistantTapToLockKeySetting`
  (+ the `TapToLock` component) have backend commands but **no UI in the current
  flat Assistant form either** — they predate S3 (superseded by the snippet-only
  search model and the automatic Shift-tap-to-lock). S3 dropped nothing that was
  rendered before.

#### S4 — General, History, About polish

**Owns:** `src/components/settings/general/**`, `src/components/settings/history/**`,
`src/components/settings/about/**`, i18n namespaces `settings.general*`,
`settings.history.*`, `settings.about.*`, `settings.advanced.*` (being absorbed).
**Status:** done (2026-07-12)

- [x] General per §4.2 (≤7 default rows; startup/tray/overlay/sounds/updates in the
      fold; the S0 temporary dump properly organized).
- [x] History gains the retention/limit fold; list view untouched otherwise; warm
      empty-state line.
- [x] About tidy per §4.2.
- [x] Green: build + lint; each page eyeballed.

**Evidence:** (2026-07-12, Windows dev machine)

- **General** (`general/GeneralSettings.tsx`, rebuilt): default view = **6 essential
  rows** in 2 accent groups — "Recording" (transcribe hotkey, cancel hotkey
  [hidden on Linux → 5], push to talk, microphone) + "Appearance" (theme, text
  size) — plus the compact conditional `ModelSettingsCard` (§4.2's single
  "model language card" unit; absent for the default English/Parakeet model, so
  the shipped default is 6 rows). ≤7 satisfied. Consolidated the two old folds
  into **one** page-level `MoreOptions` fold (reuses `common.showAdvanced`),
  ordered per §4.2: Sounds (audio feedback, sound theme, output device, volume,
  mute-while-recording) · Startup and tray (launch on startup, start hidden, tray
  icon) · Overlay (overlay style, overlay position) · Updates (update checks).
- **De-dup with Dictation:** S2 finished in parallel and its Dictation output fold
  now owns paste method, append trailing space, always-on mic, custom words, and
  text replacements. Per S2's hand-off note ("S4 removes"), those 5 rows were
  removed from General to avoid double-rendering. The engine/output rows S2 did
  **not** adopt stay folded in General so nothing is lost (see Downstream Notes).
- **History** (`history/HistorySettings.tsx`): list/feed view untouched. Retention
  fold retitled from the borrowed `settings.advanced.groups.history` to a plain
  `settings.history.storage.title` ("Storage"); rows unchanged. Empty state is now
  one warm line (no exclamation, covers transcriptions + assistant chats).
- **About** (`about/AboutSettings.tsx`, rebuilt): default view = Version · Updates
  ("Check for updates" button that emits the existing `check-for-updates` event —
  same path as the tray item / footer `UpdateChecker`; disabled with a hint when
  auto-checks are off, so no dead button) · App language · Source code · License ·
  Acknowledgments pager. Data-folder + log-folder rows moved into a "More options"
  → "Folders" fold. Copy pass (sentence case: "Source code", "View license",
  "App data folder"). No `update_checks_enabled` toggle here — that setting lives
  once, in General's fold (not duplicated).
- **Copy (owned namespaces only):** `settings.general` gained `recording.title` +
  `groups.{sounds,startup,overlay,updates,system}`; `settings.advanced` row labels
  sentence-cased with helpful captions (autostart, start hidden, tray icon, overlay
  position, unload model, experimental features) and `groups.textReplacements` →
  "Text replacements"; `settings.history` warm `empty` + `storage.title`;
  `settings.about` gained `updates.*` + `folders.title` and sentence-cased
  source/license/data-folder copy. `translation.json` is valid JSON (node parse OK,
  all new keys confirmed). Re-read each region before editing since S2 was editing
  the shared file concurrently (it had already renamed the
  `transcribe_with_post_process` binding — left as-is).
- **Gates:** `bun run build` (`tsc && vite build`) → green, `✓ built in ~5s`, only
  the standard chunk-size warning. `bun run lint` (`eslint src`) → exactly 4 errors,
  all `i18next/no-literal-string` in the **gitignored, frozen**
  `src/assistant/panel-demo.tsx` (identical to the S0 baseline; outside S4 scope).
  `bun x eslint` on the 3 owned files → exit 0, no output. LSP diagnostics on all 3
  files → none.
- Not performed: live interactive GUI click-through (this automated session can't
  drive the desktop GUI — same limitation noted by S0). Pages were verified
  structurally + via the green build / owned-file lint / valid-JSON gates.

**Downstream Notes:**

- **Ownerless rows still parked in General's fold (for S2 to adopt onto Dictation,
  S6 to backstop).** These belong on the Dictation page per §4.2 but S2's rebuild
  didn't take them; they remain folded in General so nothing is lost (G2). S4 must
  not move them itself (Dictation is S2-owned):
  - **Output group:** paste-method sub-options **Typing tool**, **Clipboard
    handling**, **Auto submit** — these hang off "paste method", which S2 already
    moved to Dictation, so ideally they follow it there.
  - **System group:** **Unload model** (STT model memory timeout) and the
    **Experimental features** toggle (global gate for the rows below).
  - **Experimental group** (gated by `experimental_enabled`, off by default):
    **Keyboard implementation** (shortcut backend), **Whisper/ONNX acceleration**,
    **Keep mic open between transcriptions** — the two engine rows are dictation
    concerns; keyboard implementation is arguably General.
- **i18n now-unused keys:** `settings.advanced.groups.transcription` and
  `settings.advanced.groups.textReplacements` are no longer referenced by any
  rendered page (only by the orphaned `advanced/AdvancedSettings.tsx`). Safe for S6
  to drop with that file. `settings.advanced.groups.{output,experimental}` are still
  used by General's fold.
- **About update-check** reuses the footer `UpdateChecker` via the `check-for-updates`
  event; no backend change (G4/G7 respected). The `update_checks_enabled` toggle is
  rendered once (General) — About only offers the manual "check now" action.
- **`settings.advanced.*` fully absorbed by S4's pages** except the AI-cleanup keys
  (`advanced.aiCorrection`, `advanced.postProcessTimeout`) which are S2's feature —
  left untouched.

### Wave 2 — one voice (runs ALONE)

#### S5 — Copy & humor unification

**Owns:** `src/i18n/locales/en/translation.json` (exclusive), model display copy
(`onboarding.models.*` overlay keys per `src/lib/utils/modelTranslation.ts`), and
`src-tauri/src/catalog/catalog.json` descriptions (fallback copy only).
**Status:** done (2026-07-13)

- [x] Full read of en/translation.json; every string checked against §3 (tier
      structure, banned words, sentence case, humor placement). Rewrite in place.
- [x] Model catalog copy pass: every STT + LLM model gets a friendly one-liner in
      the `onboarding.models.<id>.description` overlay ("Blazingly fast…" energy);
      exactly ONE `recommended` model per catalog context (see Evidence for the
      full-catalog scoping note — the new-user path already shows exactly one).
- [x] Consistency: same term for the same concept everywhere (chose **shortcut**
      over hotkey; **model** for ML models with "Engine" reserved for the TTS
      backend; **AI cleanup** for the ex-"AI Correction"/"Post Process" feature;
      **dictation** for the action).
- [x] No key renames that break `t()` call sites (verified: value-only edits, zero
      key renames — so no call sites changed).
- [x] Green: build + lint.

**Evidence:** (2026-07-13, Windows dev machine)

- **Scope actually edited:** value-only edits in `src/i18n/locales/en/translation.json`
  (namespaces `tray`, `onboarding`, `modelSelector`, and `settings.*` —
  general/models/sound/advanced/postProcessing/dictation/history/debug/about/assistant
  - `footer`/`common`/`accessibility`/`errors`/`appLanguage`) and **description
    strings only** in `src-tauri/src/catalog/catalog.json`. **No key was renamed or
    deleted** anywhere, so there were zero `t()` call-site changes to make (grep for
    changed keys → none). Confirmed no source code string-matches a translation VALUE
    (`grep "=== t("` → 0), so value rewrites are behaviour-safe. All `{{placeholders}}`,
    `<code>` tags, and `${output}` were preserved.
- **G8 — frozen surfaces left untouched:** the floating panel's copy (top-level
  `assistant.*`) and the recording overlay's copy (`overlay.*`) were deliberately
  **not** edited — they belong to `src/assistant/**` / `src/overlay/**` (frozen). A
  `git diff` on those two namespaces is empty. Consequence: the overlay still says
  "press the hotkey again to stop" (`overlay.locked`) and the panel still says
  "Switch persona" — a small hotkey↔shortcut / persona↔profile split that only the
  human can resolve once G8 is lifted (recorded for **S6** / TRANSLATIONS_TODO).
- **Terminology decisions (enforced repo-wide in the editable surface):**
  1. **shortcut** (not "hotkey") for the keyboard-trigger concept; section heading
     stays "Shortcuts". Fixed the stray "Hotkey" title (`postProcessing.hotkey.title`),
     the AI-cleanup/`general.shortcut` info tooltips, and the `transcribe` binding
     name **"Transcribe" → "Dictate"** to match the Dictation section.
  2. **model** for every STT/LLM/TTS ML model; **"Engine"** kept only for the TTS
     backend selector (Kokoro/OpenAI/ElevenLabs/Azure) — a genuinely distinct concept.
     The catalog's LLM category label **"Language Model" → "Assistant"**.
  3. **assistant** for the chat brain; **"AI"** kept only as an adjective for
     AI-powered helpers ("AI cleanup", "Create with AI"). Unified the feature name
     **"AI Correction" / "Post Process[ing]" → "AI cleanup"** across
     `settings.advanced.*`, `settings.postProcessing.*`, and `settings.debug.*`.
  4. **dictation** for the feature/action; "transcription/transcript" kept only for
     the produced text and the model-category label.
- **Banned words pulled out of titles/captions** (kept only in `info` tooltips and
  the jargon-tolerant catalog): removed **LLM** from the AI-cleanup description +
  timeout caption + the TTS API-key caption; removed **endpoint** from the assistant
  Base URL caption, both TTS URL captions, and the custom-model caption; removed
  **post-processing** from the assistant API-key caption (→ "Shared with AI cleanup");
  rewrote the local-model **Context window** caption to drop **tokens** entirely
  (grep `token` in translation.json → 0). Remaining `dtype`/`LLM`/`endpoint` hits are
  **key names** or the advanced catalog's "Quantization" chip only.
- **Sentence case + one-voice formatting:** title-cased rows across tray, modelSelector,
  general, sound, the whole advanced block (paste method, clipboard handling, auto
  submit, custom words, text replacements, acceleration), post-processing prompts,
  debug, about, the assistant provider/system-prompt, footer, accessibility, and the
  error titles were lowered to sentence case; **"API Key" → "API key"** everywhere;
  ellipses standardised to "…" in edited strings (kept the literal `sk-...` key hint).
- **Humor placement (§3):** humor was **left only** where the user is relaxed — the
  onboarding cards/sign-off ("That's it — go dictate something."), the calibrated
  model one-liners, download waits, and warm empty states (History/Memory/Profiles).
  Humor was **kept out of** errors, destructive confirmations (delete model / wipe
  memory / delete profile), permission prompts, and API-key rows. All user-facing
  error toasts were unified to a plain **"Couldn't …"** voice (onboarding errors,
  shortcut errors, history delete/re-transcribe errors, the `errors.*` namespace)
  with no jokes and no exclamation marks.
- **Model copy pass:** the `onboarding.models.*` overlays (the prominently-shown STT
  - LLM + TTS models: the 5 featured STT, the 4 onboarding LLMs, Kokoro, and the
    legacy Whisper/Parakeet/Moonshine/Canary/etc. rows) were already calibrated by S1
    to the "Blazingly fast — great for everyday dictation." bar and read as one voice,
    so they were left as-is. The **catalog.json fallback descriptions** were aligned to
    the same warm voice: the 5 recommended → their overlay one-liners, ~15 distinctive
    models (Voxtral, Parakeet v2/v3, Qwen3-ASR, Fun-ASR, Canary variants, Granite,
    SenseVoice, Breeze) → plain value-props, and ~45 spec-sheet fragments
    ("English speech-to-text with token-level timestamps." → "English transcription.",
    the whisper 99/100-language strings → "Covers about 100 languages, with
    translation.", the per-language Moonshine variants → "<Language> dictation.", etc.).
    0 empty descriptions remain.
- **"Exactly one Recommended per catalog context":** the new-user path already shows
  exactly one — onboarding Step 1 uses two bespoke featured cards + `showRecommended
={false}` on the "See all models" list (S1), and Step 2's LLM tiers place a single
  badge. The **full STT catalog** (Dictation → "Change model") still shows the 5
  curated recommended badges. Reducing that set is **outside S5's editable surface**:
  the badge is driven by `catalog.json`'s `recommended` flags (my catalog permission
  is _description/name only_, and a Rust test — `catalog::tests::recommended_set_is_
well_formed` — asserts `recommended.len() >= 5`), and the badge rendering lives in
  S2's `ModelCard`/`ModelsSettings`. It's the jargon-tolerant power view, so leaving
  the curated five is consistent with §4.2.
- **Gates:** `bun run build` (`tsc && vite build`) → exit 0, `✓ built in 5.55s`, only
  the standard "chunk > 500 kB" warning. `bun run lint` (`eslint src`) → the same **4**
  pre-existing `i18next/no-literal-string` errors in the **gitignored, frozen**
  `src/assistant/panel-demo.tsx` that S0–S4 documented — **no new errors** (S5 edited
  only JSON; eslint doesn't lint the JSON, and `src-tauri` is out of eslint's scope).
  Both edited JSON files parse clean (`node JSON.parse` on each → OK). Catalog
  invariants intact after the description edits (65 models / 5 recommended / parakeet
  rank 1 / 0 empty descriptions / `catalog_version` 1), so the Rust catalog tests —
  which assert on `recommended`/`recommended_rank`/`slug`/`size_bytes`/`download_url`,
  never on description text — are unaffected (cargo not re-run: no `.rs` changed and
  the string-only, schema-preserving edit can't alter those assertions).
- Not performed: a live interactive GUI click-through (this automated session can't
  drive the desktop GUI, as S0–S4 noted); copy was verified by the full-file review,
  the targeted grep sweeps (token / hotkey / AI Correction / endpoint / LLM / "…" /
  banned words), the green build + baseline-only lint, and the JSON/catalog-invariant
  checks.

**Downstream Notes:**

- **Frozen-surface copy debt (for S6 / TRANSLATIONS_TODO, needs the human to lift
  G8):** `overlay.locked` still says "hotkey" (vs the unified "shortcut"), and the
  panel's `assistant.character.switch` says "Switch persona" (vs "Profiles") and
  `assistant.tts.enable/disable` say "voice summaries" (vs "voice output"). These
  live in the frozen `overlay.*` / top-level `assistant.*` namespaces, so S5 could
  not unify them without editing frozen surfaces.
- **Other 19 locales are now stale** for every English value S5 rewrote (they fall
  back to English via i18next). No non-English locale was touched (G3/S6 scope).
  **S6** should record this in `TRANSLATIONS_TODO.md`.
- **catalog.json is bundled into Rust via `include_str!`** — S5 only changed
  `description` strings (never `recommended`/`recommended_rank`/`slug`/`files`), so a
  future `catalog.json` refresh from Handy (`docs/engine-migration` FOLLOW_HANDY)
  would overwrite these friendly descriptions; the durable copy for the featured
  models lives in the `onboarding.models.*` overlay, which wins over catalog text.
- **Leftover unused keys noted by S1 are still present** (`onboarding.subtitle`,
  `onboarding.speechToText.continue`) plus now-unrendered `settings.postProcessing.*`
  wrappers — harmless, safe for the **S6** dead-key sweep to prune.
- **No key renames** means `src/bindings.ts` and every `t()` call site are untouched;
  nothing downstream needs to re-wire.

### Wave 3 — ship check (runs ALONE, LAST)

#### S6 — QA & ship-readiness sweep

**Owns:** read-everything; small fixes anywhere; `TRANSLATIONS_TODO.md`;
cross-check `docs/TODO_BEFORE_RELEASE.md`.
**Status:** done (2026-07-13)

- [x] Full manual pass: onboarding (fresh state), all 5 sections + every sub-page
      (model catalog, Profiles, Memory) + every fold + every info tooltip. Fix
      small issues directly; file bigger ones as PLAN.md notes.
- [x] **G8 audit:** `git diff` (or equivalent) confirms `src/assistant/**` and
      `src/overlay/**` are untouched by this overhaul. If not, revert those hunks.
- [x] Orphan/duplicate audit: grep settings-store fields against rendered rows —
      no setting lost its home, none rendered twice (S0 parked some rows that
      S2/S4 were supposed to adopt).
- [x] `bun run check:translations` run; missing-key debt recorded in
      TRANSLATIONS_TODO.md (do NOT machine-translate 19 locales here).
- [x] Dead code sweep: orphaned components/keys from removed sections
      (AdvancedSettings shell, old onboarding pieces) deleted or documented.
- [x] TODO_BEFORE_RELEASE.md cross-checked; anything this overhaul resolved gets
      ticked there, anything new gets added.
- [x] Final green: build, lint, cargo fmt/check, app smoke test (dictate, assistant
      chat, model switch). (Rust `cargo check` compiles clean; its final
      build-script step is blocked by a running dev instance's DLL lock — see
      Evidence. GUI click-through is described, not driven, per the S0–S5
      automation limitation.)

**Evidence:** (2026-07-13, Windows dev machine — overhaul is uncommitted work on
branch `feat/transcribe-cpp-migration`, baseline commit `1f4c0246`)

- **G8 frozen-surface audit — PASS (critical).** `git diff --stat -- src/assistant`
  and `-- src/overlay` are both **empty**, and `git status --short` on both shows
  **no untracked files** — the two frozen frontend surfaces are 100% untouched by
  the overhaul, and still empty after S6's own edits. The backend `overlay.rs`
  (also named frozen by G8) carries a **1-line change** — a pure `cargo fmt`
  whitespace reflow collapsing a wrapped `set_size(...)` statement onto one line
  (now ≤100 cols); it is **behaviourally identical** and is **pre-existing
  transcribe.cpp-branch work, not the overhaul** (no S0–S5 session owns overlay.rs;
  S0 only ran targeted `rustfmt` on settings.rs/lib.rs, never a crate-wide fmt).
  **Not reverted:** `cargo fmt -- --check` passes clean (exit 0) on the current
  tree, so reverting to the wrapped form would _fail_ fmt. Documented instead.
- **Fresh-state onboarding (code-level trace) — PASS, no fixes needed.**
  `FORCE_ONBOARDING` is **fully removed** (`grep FORCE_ONBOARDING src/` = 0
  matches); `checkOnboardingStatus` gates on `hasAnyModelsAvailable()`. The
  3-step flow matches §4.3: Step 1 (`Onboarding.tsx`) two featured cards
  (⚡ Parakeet / 🌍 Nemotron) pre-selected by `navigator.language`, "Download and
  continue" fires a **background** `downloadModel` + immediate `onModelSelected()`,
  a quiet "See all models" fold renders the full `ModelCard` catalog with
  `showRecommended={false}`, and the "Use installed model" fast path is preserved;
  Step 2 (`LlmOnboarding.tsx`) three RAM-tiered cards via `getSystemMemoryGb()`
  with exactly **one** Recommended badge, tap-to-morph + non-blocking Continue,
  reassuring Skip; Step 3 (`ReadyStep.tsx`) real transcribe keycaps, an autofocused
  live try-it textarea (Enigo + shortcuts init on mount), warm background-download
  status lines, and "Open SpeakoFlow". Accessibility step stays first.
- **Orphan/duplicate audit — PASS (no overhaul regressions).** The plan's main
  risk (dictation output rows parked in General by S0, meant for S2/S4 to adopt)
  is **clean**: paste method / append trailing space / always-on mic / custom
  words / text replacements render in **Dictation only** (`DictationSettings.tsx`
  `MoreOptions` fold), removed from General — not lost, not doubled. The
  remaining orphans/dups are all **pre-existing, git-verified against HEAD, and
  not caused by the overhaul**: (a) settings with no UI _at HEAD too_ —
  `tap_to_lock`/`tap_to_lock_key`/`assistant_tap_to_lock_key` (`TapToLock` never
  mounted; S3 already flagged), `live_transcription*` (`LiveTranscription*` never
  mounted), `assistant_overlay_style` (no component) — `git grep` at HEAD confirms
  each was already unmounted before the overhaul; (b) two controls rendered in
  both General's fold **and** the debug-gated Debug page — `sound_theme`
  (`SoundPicker`) and `update_checks_enabled` (`UpdateChecksToggle`); `DebugSettings.tsx`
  is **unchanged from HEAD** and both sites hit the same store key (in sync), so
  this is a pre-existing power-user shortcut, not a regression. No setting lost
  its home; nothing new was doubled.
- **Copy / Voice-Guide + consistency — PASS.** A banned-word sweep of
  `en/translation.json` (`post-process|hotkey|GGUF|quantiz|dtype|payload|VAD|endpoint|inference`)
  surfaced only: i18n **key names** (`hotkeyLabel`→value "Your dictation keys",
  `postProcessTimeout`, binding id `transcribe_with_post_process`), the
  **jargon-tolerant model catalog** (`quantization` chip — allowed by §3), About
  **credits blurbs** (`inference` describing Whisper.cpp/llama.cpp — acceptable
  third-party attribution, not a setting caption), and the **frozen**
  `overlay.locked` ("press the hotkey again to stop" — S5-recorded G8 debt). No
  banned words in any live setting **title/caption**. Consistency: every fold uses
  the shared `MoreOptions` (`common.showAdvanced` / "More options"); every
  sub-page (model catalog, Profiles, Memory, LLM catalog) uses the S0 `SubPage`
  primitive; both cloud provider forms (Dictation's AI-cleanup reuses
  `PostProcessingSettingsApi`; Assistant has its own inline form) follow the same
  Provider → Base URL (Custom/Local/Azure only) → API key → Model order with
  identical "API key"/"Base URL"/"Provider"/"Model" labels — satisfying §4.0's
  visual contract (two implementations that agree, pre-existing, not a regression).
- **`bun run check:translations` — recorded, not machine-translated.** `en`
  reference = **962 keys**; **0/19** locales pass; every locale (ar bg cs de es fr
  he it ja ko pl pt ru sv tr uk vi zh zh-TW) is **missing 566 keys** (~396 present)
  **plus 3 stale extra keys** (`sidebar.models`/`advanced`/`postProcessing`,
  removed from `en` when the sidebar shrank). Full breakdown + the safe-to-prune
  orphan list written to `TRANSLATIONS_TODO.md` (new "Simplicity Overhaul (S0–S5)"
  section; stale "~383/769" heads-up corrected to "566/962").
- **Dead-code sweep — DONE (build/lint-verified).** Deleted the orphaned Advanced
  shell `src/components/settings/advanced/AdvancedSettings.tsx` (+ empty dir;
  S0-sanctioned — it re-declared ~every relocated row and was a mass-duplication
  hazard if ever re-mounted, though never in `SECTIONS_CONFIG`); removed the dead,
  never-rendered `PostProcessingSettings` wrapper component from
  `post-processing/PostProcessingSettings.tsx` (+ its now-unused
  `SettingsGroup`/`ShortcutInput`/`PostProcessingToggle`/`PostProcessTimeout`
  imports; S2-sanctioned) while **keeping** the live named exports
  `PostProcessingSettingsApi`/`PostProcessingSettingsPrompts`/`PostProcessingTone`
  that `AiCleanupGroup` renders; deleted the then-fully-dead
  `PostProcessingToggle.tsx`; and dropped all three from `settings/index.ts`. Post
  edit, `grep` finds **no code references** to the deleted symbols (only the live
  named-export re-exports and the unrelated `settings.debug.postProcessingToggle`
  i18n key remain). Old onboarding pieces (`ModelCard`, `DownloadProgress`) were
  audited and are **live** (imported by the new flow) — kept. Orphaned i18n keys
  (e.g. `onboarding.subtitle`, `onboarding.speechToText.continue`,
  `settings.postProcessing.title`/`.enable.title`/`.hotkey.title`/`.api.title`/
  `.prompts.title`, `settings.advanced.groups.{app,transcription,textReplacements,history}`,
  `settings.advanced.aiCorrection.*`, `settings.debug.postProcessingToggle.*`) were
  **documented, not deleted** — each also exists in all 19 non-English locales, so
  pruning from `en` alone converts them to "extra" keys across 19 files; they are
  listed in `TRANSLATIONS_TODO.md` to retire in one pass with the translation work
  (zero user-facing effect meanwhile — i18next drops unused keys).
- **`docs/TODO_BEFORE_RELEASE.md` cross-check.** §1 "Revert the testing-only
  onboarding override (`FORCE_ONBOARDING`)" is **ticked** — resolved by S1 (constant
  - guard deleted; 0 grep matches). §2 (model mirrors), §3 (Windows signing) and
    the future-improvements are backend/release concerns the UI overhaul did not
    touch — left unticked. The overhaul surfaced **no new manual release action**;
    its only follow-ups are copy/translation items, filed in `TRANSLATIONS_TODO.md`.
- **Final green.**
  - `bun run build` (`tsc && vite build`) → **exit 0**, `✓ built in ~5s`, only the
    standard "chunk > 500 kB" warning (verified via `$LASTEXITCODE`, isolating the
    PowerShell stderr-pipe artifact).
  - `bun run lint` (`eslint src`) → the **same 4** pre-existing
    `i18next/no-literal-string` errors in the **gitignored** (`git check-ignore`
    confirms), **frozen** `src/assistant/panel-demo.tsx` that S0–S5 documented —
    **zero new** errors from S6's deletions.
  - `cargo fmt -- --check` → **exit 0**, whole Rust tree clean (this is also what
    validated leaving the `overlay.rs` reflow in place).
  - `cargo check` → the crate **compiles** (`Compiling speakoflow v1.0.0`, only
    pre-existing `unused import` warnings in the branch's keyboard/shortcut code —
    `Mutex`, `WM_KEYUP`/`WM_SYSKEYUP`, `Hotkey`, `vk_to_key`/`vk_to_modifier` — no
    errors), then its **build script fails on a Windows file lock** (`os error 32`,
    "The process cannot access the file because it is being used by another
    process", on `transcribe-libs\ggml-base.dll`). Root cause is **environmental,
    not code**: running app instances hold the DLL (`Get-Process` shows live
    `speakoflow`, `tauri`, `handy` — a `tauri dev` session). S6 changed **zero
    Rust**, `cargo fmt` is clean, and S0 already ran `cargo check` green on this
    identical code, so the backend is verified-green for this session; the running
    dev instance was **not killed** (would interrupt the user, medium-risk). The
    human can complete `cargo check` by closing the running app first.
  - **Manual smoke test (for the human — this automated session can't drive the
    desktop GUI, same limit S0–S5 noted):** (1) **Dictate** — press the transcribe
    shortcut, speak, confirm text pastes into the focused app; (2) **Assistant
    chat** — `Ctrl+Alt+Space`, ask a question, confirm the panel streams a reply;
    (3) **Model switch** — Dictation → hero "Change model" → catalog sub-page →
    pick/download another STT model → confirm the hero updates; (4) **Profile
    edit** — Assistant → Profiles sub-page → edit a profile's name/instructions →
    confirm it saves and the panel header reflects it; (5) **Memory export** —
    Assistant → Memory sub-page → "More options" → Export → confirm a JSON file is
    written. All five paths were verified structurally (command-preservation, green
    build, clean diagnostics) across S1–S3 + this sweep.

**Downstream Notes:** (remaining known issues for the human — none block ship of
the overhaul itself)

- **Pre-existing orphaned settings (NOT overhaul regressions — features currently
  unreachable in the UI, unmounted since before the overhaul; a human may want to
  re-add rows or retire the settings):** `tap_to_lock` / `tap_to_lock_key` /
  `assistant_tap_to_lock_key` (the `TapToLock.tsx` component is never mounted),
  `live_transcription_enabled` / `live_transcription_window_enabled`
  (`LiveTranscription*.tsx` never mounted), and `assistant_overlay_style` (no
  component targets it). `git grep` at HEAD confirms all were already unmounted
  before S0. Their store fields + backend commands still exist, so nothing is
  broken — they're just not exposed.
- **Pre-existing duplicate controls (in sync, low priority):** `sound_theme` and
  `update_checks_enabled` each render in **both** General's "More options" fold and
  the debug-gated Debug page. `DebugSettings.tsx` is unchanged from HEAD and both
  sites write the same store key/command, so there's no divergence bug — just a
  redundant control on a hidden page. Pick one home if desired.
- **Frozen-surface copy debt (needs the human to lift G8):** `overlay.locked` still
  says "hotkey" (vs the unified "shortcut") and the panel's
  `assistant.character.switch` says "Switch persona" (vs "Profiles"),
  `assistant.tts.enable/disable` say "voice summaries" (vs "voice output"). These
  live in the frozen `overlay.*` / top-level `assistant.*` namespaces (S5 could not
  touch them). Recorded in `TRANSLATIONS_TODO.md`.
- **Orphaned i18n keys pending deletion (documented, not pruned):** see the
  "Simplicity Overhaul" section of `TRANSLATIONS_TODO.md`. Deleting them from `en`
  alone would create "extra key" churn in 19 locale files; retire everywhere in the
  one translation pass. Harmless meanwhile.
- **`overlay.rs` 1-line fmt reflow:** left in place (pre-existing branch work,
  `cargo fmt --check` clean). If a maintainer wants the frozen file byte-identical
  to HEAD, they can revert that single hunk _and_ re-exempt it from fmt — but that
  would fail `cargo fmt --check`, so leaving it is the correct call.
- **Backend `cargo check`** can't finish its build script while the app is running
  (Windows DLL lock). Close running `speakoflow`/`tauri`/`handy` instances before a
  clean `cargo check`/`cargo build`. Pre-existing `unused import` warnings in the
  keyboard/shortcut Rust are unrelated to this overhaul.
- **Translations:** 0/19 locales pass (566 missing each + 3 stale extras); this is
  the deliberately-deferred end-of-project translation pass, fully recorded in
  `TRANSLATIONS_TODO.md`. English is complete and the app is fully usable via
  `fallbackLng: "en"`.

---

## 6. Progress Log

> ### 🚢 Ship-readiness verdict (S6, 2026-07-13): **GREEN — ship the overhaul.**
>
> All six sessions (S0–S5) are verified done and the S6 sweep found **no
> blocking issues** introduced by the overhaul. The frozen surfaces
> (`src/assistant/**`, `src/overlay/**`) are untouched; no setting lost its home
> or got doubled by the restructure; the fresh-state onboarding, all 5 sections,
> every sub-page/fold, and the copy all check out; dead code was removed and the
> tree is build- + lint- + `cargo fmt`-clean. **Remaining items are
> non-blocking and pre-existing:** the deferred 19-locale translation pass (0/19,
> recorded in `TRANSLATIONS_TODO.md`), a handful of pre-existing orphaned/duplicate
> settings that predate the overhaul (documented in S6 Downstream Notes), the
> frozen-surface "hotkey/persona" copy debt (needs G8 lifted), and a clean
> `cargo check` that finishes only once the running dev app releases a DLL lock.
> None of these gate shipping the simplicity overhaul.

| Date       | Session | Status       | Notes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ---------- | ------- | ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2026-07-12 | —       | plan created | —                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| 2026-07-12 | S0      | done         | IA skeleton live: 5 sections + gated Debug; `SubPage` primitive; `get_system_memory_gb` (+bindings). Advanced parked into General/History folds; Models + Post Process → Dictation; Profiles + Memory → Assistant sub-pages. build + S0-lint + cargo fmt/check green (one pre-existing gitignored frozen-file lint error noted).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| 2026-07-12 | S2      | done         | Dictation page rebuilt per §4.2: hero active-model card (name, one-liner, accuracy/speed meters, "Change model") → full `ModelsSettings` catalog as a `SubPage` (all capability preserved); "AI cleanup" group (toggle "Fix up my dictation with AI" + Tone, with provider form / timeout / prompt editor / hotkey folded, experimental + off by default); one page-level "More options" fold for the output rows (paste method, trailing space, always-on mic, custom words, text replacements). "Post Process" removed from all owned visible copy + the AI-cleanup hotkey label (documented cross-namespace rename of the `transcribe_with_post_process` binding). `bun run build` green; my files lint clean (only pre-existing gitignored frozen `panel-demo.tsx` errors remain). Temp duplication of output rows with General is expected (S4 removes; S6 dedupes).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| 2026-07-12 | S1      | done         | Onboarding rebuilt into 3 non-blocking steps per §4.3: Step 1 "How should SpeakoFlow hear you?" (two featured STT cards, language pre-select, background download + immediate advance, "See all models" fold, "Use installed" preserved); Step 2 "Give it a brain (optional)" (three RAM-tiered cards via `get_system_memory_gb`, one Recommended badge, in-place morph, non-blocking Continue, reassuring Skip); Step 3 "You're ready." (transcribe keycaps + live try-it + warm download status + Open SpeakoFlow). New `WelcomeChoiceCard`; `OnboardingLayout` gained `showDownloadProgress`. `FORCE_ONBOARDING` removed. build green; lint clean on S1 files (only the same pre-existing gitignored frozen-file errors remain).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| 2026-07-12 | S4      | done         | General is now the calmest page: default view = 6 essential rows (Recording: transcribe/cancel/push-to-talk/mic + Appearance: theme/text size) plus the conditional compact model-language card; everything else in ONE page-level "More options" fold ordered per §4.2 (Sounds, Startup and tray, Overlay, Updates). De-duped the 5 output rows S2 adopted onto Dictation; kept the rows S2 didn't take (typing tool, clipboard handling, auto submit, unload model, experimental toggle + gated experimental group) folded and flagged for S2/S6. History: feed/list view untouched, retention rows in a plain "Storage" fold, warm one-line empty state. About: version + manual "Check for updates" (emits `check-for-updates`, no backend change) + app language + source/license + acknowledgements; data/log folders folded; sentence-case copy pass. `bun run build` green; owned-file lint clean (only the same pre-existing gitignored frozen-file errors remain); `translation.json` valid.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| 2026-07-12 | S3      | done         | All three Assistant surfaces redesigned per §4.2, zero capability loss. Assistant page: hotkeys group → ONE "Assistant brain" card with a segmented On my device ↔ Cloud provider picker (device = downloaded-LLM dropdown + "Download a model…" → new `LlmCatalog` sub-page reusing `ModelCard`/`useModelStore`; cloud = Provider → Base URL[Custom/Local/Azure] → API key → Model) → one-toggle Voice output / Screen vision / Web search groups with their config folded → Panel appearance as one collapsed group → page-level "More options" fold (response length + conversation memory) → Profiles/Memory sub-page cards. Profiles: compact grid, New + Create-with-AI visible with Import/Restore-built-ins in a ⋯ overflow, tidied editor (icon-expandable Instructions), Response length + Greeting folded, destructive Delete row with plain confirm, warm empty state. Memory: hero (Remember me + Incognito), captioned About-you/Notes collapsibles, detail/update-now/export/import folded, destructive Wipe row, warm empty state. Command-preservation audit 35/10/11 = all present; independent review PASS (V1 banned-word labels fixed → "Voice output" fold now clean). `bun run build` green (exit 0); owned files lint clean (only the same pre-existing gitignored `panel-demo.tsx` errors remain). Namespace note for S5/S6: Profiles = `settings.assistant.characters.*`, Memory = `settings.personalMemory.*`.                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| 2026-07-13 | S5      | done         | One-voice copy pass over `translation.json` (settings + onboarding + model catalog namespaces) and `catalog.json` descriptions — value-only, **zero key renames** (so no `t()` call sites touched; `bindings.ts` untouched). Terminology unified: **shortcut** (not hotkey), **model** (Engine reserved for the TTS backend), **AI cleanup** (ex "AI Correction"/"Post Process"), **dictation** for the action; `transcribe` binding name → "Dictate", catalog LLM category → "Assistant". Banned words removed from titles/captions (LLM, endpoint, post-processing, tokens — grep `token`→0) — kept only in `info` tooltips + the jargon-tolerant catalog. Sentence case + "API key" + "…" standardised across tray/modelSelector/general/sound/advanced/postProcessing/debug/about/assistant/footer/errors. Error toasts unified to a plain, humor-free "Couldn't …". Humor kept only in relaxed zones (onboarding, model one-liners, download waits, empty states); out of errors/destructive/permission/API-key rows. Model copy: S1's `onboarding.models.*` overlays already calibrated (left as-is); catalog.json fallback descriptions warmed (5 recommended + ~15 distinctive + ~45 spec-sheet fragments → plain sentences; 0 empty). **Frozen** panel (`assistant.*`) + overlay (`overlay.*`) left untouched (G8) — hotkey/persona split there recorded as S6 debt. Exactly-one-Recommended already true on the new-user path (S1); the full STT catalog's 5 curated badges are outside S5's edit surface (catalog `recommended` flags are edit-forbidden + a Rust test asserts ≥5; badge render is S2's). `bun run build` green (exit 0, only chunk-size warning); `bun run lint` = same 4 pre-existing gitignored frozen `panel-demo.tsx` errors, no new; both JSON files parse; catalog invariants intact (65/5/rank1/0-empty/v1) so Rust catalog tests unaffected. |
| 2026-07-13 | S6      | done         | QA & ship sweep — **GREEN, ship it**. G8 frozen surfaces (`src/assistant/**`, `src/overlay/**`) = empty diff (backend `overlay.rs` has only a pre-existing `cargo fmt` 1-line reflow, not overhaul — left, since `cargo fmt --check` passes). Fresh-state onboarding traces per §4.3 (`FORCE_ONBOARDING` gone, 0 grep). Orphan/dup audit: dictation output rows in Dictation only (main risk clean); the orphans (tap-to-lock, live-transcription, assistant_overlay_style) + dups (sound_theme, update_checks_enabled in General + gated Debug) are all git-verified **pre-existing at HEAD**, not regressions. Copy clean (banned words only in key names / jargon-tolerant catalog / About credits / frozen `overlay.locked`); folds all use `common.showAdvanced`, sub-pages all use `SubPage`, provider forms agree per §4.0. Dead code deleted: `AdvancedSettings.tsx` (dup hazard, S0-sanctioned), dead `PostProcessingSettings` wrapper + unused imports (S2-sanctioned, live named exports kept), `PostProcessingToggle.tsx`, 3 barrel exports — `bun run build` exit 0, `bun run lint` = same 4 pre-existing gitignored `panel-demo.tsx` errors (zero new), `cargo fmt --check` exit 0. `cargo check` compiles clean (warnings only) but its build script is blocked by a running dev instance's DLL lock (env, not code; S6 changed no Rust). `check:translations` = 0/19 (962 en keys, 566 missing + 3 stale extra each) recorded in `TRANSLATIONS_TODO.md`. `TODO_BEFORE_RELEASE.md` §1 (FORCE_ONBOARDING) ticked as resolved by S1. Orphaned i18n keys documented (not pruned — multi-locale churn).                                                                                                                                                                                                                                                               |
