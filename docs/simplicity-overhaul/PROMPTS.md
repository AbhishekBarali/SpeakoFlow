# Simplicity Overhaul — Session Prompts (copy · paste · run)

Each block is a complete, standalone prompt for one AI session. Open a fresh
window, paste one prompt, let it run. Waves and parallel-safety rules are in
[`README.md`](./README.md); the binding spec and tracker are in [`PLAN.md`](./PLAN.md).

**Order:** S0 alone → (S1 ∥ S2 ∥ S3 ∥ S4) → S5 alone → S6 alone.

**Scope reminder (applies to every prompt):** this overhaul reorganizes the
SETTINGS WINDOW and ONBOARDING only. The floating assistant panel
(`src/assistant/**`) and the recording overlay (`src/overlay/**`, `overlay.rs`)
are frozen — no session touches them. No setting is removed; no behavior changes.

---

## S0 — New information architecture skeleton (WAVE 0 — run ALONE)

```
You are working on SpeakoFlow, a local-first voice app: Tauri 2 (Rust backend in src-tauri/src, React+TypeScript frontend in src). Read AGENTS.md first, then read docs/simplicity-overhaul/PLAN.md in FULL — it is the binding spec for this work. You are executing session S0.

Mission: restructure the settings window from 10 sidebar sections to 5 (General, Dictation, Assistant, History, About, plus the debug-gated Debug section), without losing ANY functionality, and build the shared primitives the later sessions depend on. You are the skeleton crew: later sessions redesign page interiors; you make the new structure exist and keep everything reachable.

Research first, own it fully: read src/components/Sidebar.tsx, src/App.tsx, src/components/settings/index.ts, every settings section component's top-level structure, src/components/ui/ (SettingContainer, SettingsGroup, MoreOptions, Tooltip), src-tauri/src/settings.rs, and how bindings.ts is generated (tauri-specta, exported at debug startup; cargo target dir is C:/hbt per workspace .cargo/config.toml — regenerate by running the debug exe ~8s, never hand-edit beyond the canonical format).

Do exactly what PLAN.md §5/S0 says:
1. Sidebar → 5 sections + gated Debug. Dictation section temporarily renders the existing speech-to-text content of ModelsSettings plus PostProcessingSettings stacked vertically (interiors get redesigned in S2 — your job is only that nothing becomes unreachable). Assistant section keeps AssistantSettings and adds sub-page navigation to the existing CharactersSettings and MemorySettings.
2. Build a reusable SubPage primitive in src/components/ui/ (title + back button + content swap; keep it simple and match the existing minimal aesthetic). Export it from ui/index.ts.
3. Remove Advanced from the nav; temporarily park its rows so nothing is lost: app/output/transcription rows into a MoreOptions fold on General, history-retention rows onto the History page. S4 will polish this.
4. Backend: add a get_system_memory_gb command (S1 needs it for RAM-tiered model suggestions). Register it in lib.rs and regenerate bindings.ts properly. This is the ONLY backend change in the entire overhaul — add nothing else to settings.rs.
5. Trim sectionSubtitles i18n to the 5 sections, one plain sentence each, per the Voice Guide in PLAN.md §3.

Constraints: i18n for every string (sidebar.*, sectionSubtitles.*, common.* namespaces are yours). Keep internal section keys stable where renaming would churn (repo precedent: 'characters' key stays even though the label is Profiles). Don't redesign page interiors. FROZEN SURFACES: never touch src/assistant/**, src/overlay/**, overlay.rs, onboarding, or the Rust engine/audio code. No setting removed, no behavior changed — restructure and rewire only.

Done when: bun run build + bun run lint green, cargo fmt + cargo check green, app launches, all 5 sections + Profiles/Memory sub-pages + Debug render, and a manual click-through finds every previously-existing setting still reachable somewhere.

FINAL STEP (mandatory): update docs/simplicity-overhaul/PLAN.md — tick S0's boxes you verified, set Status to done with today's date, fill Evidence (what you ran/tested) and Downstream Notes (exact names of the new command and SubPage primitive for S1–S4), and add a Progress Log row. Never tick what you didn't verify.
```

---

## S1 — Onboarding: from catalog to welcome (WAVE 1)

```
You are working on SpeakoFlow (Tauri 2 + React/TS). Read AGENTS.md, then docs/simplicity-overhaul/PLAN.md in FULL — especially §3 (Voice Guide) and §4.3 (onboarding spec), which are binding. You are executing session S1. S0 is complete — read its Downstream Notes in PLAN.md for the exact name of the system-memory command.

Mission: replace the research-catalog onboarding with a 3-step welcome that never blocks the user. Current flow (src/components/onboarding/: Onboarding.tsx, LlmOnboarding.tsx, OnboardingLayout.tsx, ModelCard.tsx, DownloadProgress.tsx; step machine in src/App.tsx) shows 9+ speech models with quantization badges and then asks a brand-new user to pick a multi-GB LLM. Research the current flow and the model catalog plumbing (src-tauri/src/catalog/catalog.json, src/lib/utils/modelTranslation.ts, download events) before changing anything.

Build exactly the PLAN.md §4.3 flow:
1. Step 1 "How should SpeakoFlow hear you?" — exactly two featured cards: Parakeet Unified EN 0.6B (fast, streaming, English) and Nemotron Streaming 3.5 (real-time, 28 languages). Pre-select by system/browser language (English → Parakeet, otherwise Nemotron). Primary button "Download and continue" starts the download IN THE BACKGROUND and advances immediately. A quiet "See all models" disclosure expands the full existing catalog for enthusiasts. Preserve the "Use installed model" fast path when a model already exists.
2. Step 2 "Give it a brain (optional)" — a 2-line plain-language explanation of the assistant, then exactly THREE local-LLM cards tiered by get_system_memory_gb (small machine / recommended mid / vision-capable; ONE Recommended badge total). Tapping a card morphs that card in place into a progress state; the primary button becomes "Continue" immediately — never make the user watch a download. "Skip for now" stays, with the reassurance line from the spec. The existing behavior where a completed background download auto-points the built-in provider at the model must keep working.
3. Step 3 "You're ready." — big keycap showing the actual transcribe hotkey (read it from settings), a live try-it textarea, background-download status lines ("Your assistant is still moving in — 42%"), one warm sign-off, "Open SpeakoFlow".
4. Remove the FORCE_ONBOARDING dev override in src/App.tsx so onboarding only shows when it should.

Copy is the point: this is the flagship humor surface. Follow the Voice Guide exactly — light and warm, no jargon (no GGUF/quant names on featured cards; jargon allowed inside "See all models"), one exclamation mark per screen max.

Ownership: src/components/onboarding/**, the onboarding step machine in src/App.tsx (ONLY that — no other App.tsx changes), i18n namespace onboarding.* only. FROZEN: src/assistant/**, src/overlay/**, bindings.ts, settings.rs — if you're missing a command/flag, mark S1 blocked in PLAN.md and stop. translation.json is shared with parallel sessions: edit only onboarding.*, re-read the file right before each edit, keep edits scoped.

Done when: bun run build + bun run lint green; you traced the full flow (fresh state → step 1 → background download → step 2 morph → step 3 → app) and describe it in Evidence.

FINAL STEP (mandatory): update docs/simplicity-overhaul/PLAN.md — tick S1's verified boxes, Status done + date, Evidence, Downstream Notes, Progress Log row.
```

---

## S2 — Dictation page: models + AI cleanup unified (WAVE 1)

```
You are working on SpeakoFlow (Tauri 2 + React/TS). Read AGENTS.md, then docs/simplicity-overhaul/PLAN.md in FULL — §3 (Voice Guide) and §4.2 (Dictation spec) are binding. You are executing session S2. S0 is complete: the Dictation sidebar section currently renders the old Models (speech-to-text part) and Post Process pages stacked as a temporary dump — read S0's Downstream Notes.

Mission: turn that dump into the real Dictation page. A new user should see: which model is listening, one AI-cleanup toggle, and nothing else. A power user should find everything.

Research first: src/components/settings/models/ModelsSettings.tsx (~560 lines), src/components/settings/post-processing/PostProcessingSettings.tsx (~550 lines), the SubPage + MoreOptions primitives in src/components/ui/, and where paste method / custom words / text replacements / always-on mic currently live (parked by S0).

Build exactly PLAN.md §4.2 Dictation:
1. Hero card: the ACTIVE model — name, friendly one-liner, accuracy/speed pips, "Change model" → opens the full model catalog as a SubPage (search, language filter, downloaded list, custom Hugging Face models, delete, streaming badges — ALL current catalog capability preserved, just one level deeper; jargon is allowed inside the catalog).
2. AI cleanup group (this is Post Process, renamed everywhere in UI copy — the phrase "post process" must not survive in visible strings you own): one toggle with a plain caption, the Tone dropdown, then a MoreOptions fold containing the provider/base URL/API key/model form, timeout, custom prompt editor, and its dedicated hotkey. Keep the experimental marking and default-off.
3. "More options" fold at page level: paste method, append trailing space, always-on microphone, custom words, text replacements.

Structure every row per the Voice Guide 3-tier rule (short title / optional one-line caption / technical info tooltip). No feature removal — hide, don't delete.

Ownership: src/components/settings/models/**, src/components/settings/post-processing/**, new src/components/settings/dictation/**, i18n namespaces settings.models.*, settings.postProcess*, new settings.dictation.* only. FROZEN: src/overlay/** and src/assistant/** (the recording overlay and floating panel are NOT part of this overhaul — the overlay-style dropdown row belongs to General/S4, not you). Do not edit Sidebar.tsx, App.tsx, bindings.ts, settings.rs, or ui/ primitives (if a primitive is missing something, note it in PLAN.md instead of editing it — parallel sessions depend on ui/). translation.json is shared: edit only your namespaces, re-read before each edit.

Done when: bun run build + bun run lint green; verified manually: model switch, model download from catalog, AI cleanup toggle + folded provider form, every parked row reachable.

FINAL STEP (mandatory): update docs/simplicity-overhaul/PLAN.md — tick verified boxes, Status done + date, Evidence, Downstream Notes, Progress Log row.
```

---

## S3 — Assistant + Profiles + Memory: full redesign (WAVE 1)

```
You are working on SpeakoFlow (Tauri 2 + React/TS). Read AGENTS.md, then docs/simplicity-overhaul/PLAN.md in FULL — §3 (Voice Guide), §4.0 (consistency contract), and §4.2 (Assistant/Profiles/Memory specs) are binding. You are executing session S3. S0 is complete: the Assistant section already has SubPage navigation to Profiles (CharactersSettings) and Memory (MemorySettings) — read S0's Downstream Notes for the primitive's name.

Mission: fully redesign THREE surfaces — the Assistant settings page AND both of its sub-pages. All three currently dump every field flat on the user. Reorganize, regroup, fold, and reword — remove NOTHING, change NO behavior.

Research first: AssistantSettings.tsx (~1000 lines) fully, CharactersSettings.tsx (~720), MemorySettings.tsx (~480), the provider settings model (provider/base URL/key/model fields, the built-in local engine), and the SubPage/MoreOptions/SettingContainer-info primitives in src/components/ui/.

SURFACE 1 — Assistant page (per PLAN.md §4.2):
1. Top: hotkeys group — Ask assistant, Show/hide panel, push-to-talk toggle — keycap style identical to General's.
2. THE BRAIN PICKER — one card replacing the flat provider form: segmented choice "On my device" vs "Cloud provider". On-device: dropdown of downloaded local LLMs + "Download a model…" (reuse the existing LLM catalog via SubPage or dialog). Cloud: provider dropdown → API key → model; Base URL only for providers that need it (Custom/Local/Azure). All current providers keep working.
3. Voice output: ONE toggle "Speak responses aloud"; engine, voice name, speed, test button and engine-specific fields in the fold.
4. Screen vision: one toggle + info tooltip; capture timing in the fold. Web search: one toggle; provider + key in the fold.
5. Panel appearance (preview, text size, panel size, opacity): one collapsed group. This CONFIGURES the panel — you never edit the panel's own code (src/assistant/** is frozen).
6. Bottom: two sub-page cards with chevrons — Profiles and Memory.

SURFACE 2 — Profiles sub-page (per PLAN.md §4.2). Today it's a card grid + a 4-button action row + a permanently-open giant edit form. Target:
1. One-line header caption explaining what profiles are.
2. Compact profile grid (avatar, name, one-line role, Active badge); click selects.
3. Action row: only "New" and "Create with AI" visible; Import and Restore built-ins move into a "⋯" overflow menu.
4. Editor (selected profile only), tidied: avatar + upload, Name, Role, Instructions (textarea collapsed to a few visible lines, expandable). Fold: Response length, Greeting.
5. Footer: Duplicate / Export / Restore default as quiet buttons; Delete = red destructive row at the bottom, plain confirmation, no humor.
6. Warm one-line empty state if no profiles exist.

SURFACE 3 — Memory sub-page (per PLAN.md §4.2). Today: toggles + dropdown + About you + Notes + Manage card, all flat. Target:
1. Hero: "Remember me" toggle; Incognito toggle under it.
2. "About you" and "Notes" collapsibles stay, one-line captions.
3. Fold ("More options"): Memory detail dropdown, "Update memory now", Export, Import.
4. "Wipe memory" = red destructive row at the bottom, plain copy, no humor.
5. Warm one-line empty state when there are no notes yet.

Every row across all three surfaces gets the Voice Guide 3-tier treatment (short title / optional one-line caption / technical info tooltip). Zero capability loss — fold and regroup, never delete.

Ownership: src/components/settings/assistant/** and i18n namespaces settings.assistant.*, settings.characters.*, settings.memory.* only. FROZEN: src/assistant/** (the floating panel itself), src/overlay/**, Sidebar.tsx, App.tsx, bindings.ts, settings.rs, ui/ primitives. translation.json is shared with parallel sessions: edit only your namespaces, re-read before each edit.

Done when: bun run build + bun run lint green; verified manually: provider switching (local ↔ cloud), TTS test button, vision + search toggles, sub-page navigation, profile create/edit/duplicate/delete, memory export/import, panel preview still renders.

FINAL STEP (mandatory): update docs/simplicity-overhaul/PLAN.md — tick verified boxes, Status done + date, Evidence, Downstream Notes, Progress Log row.
```

---

## S4 — General, History, About polish (WAVE 1)

```
You are working on SpeakoFlow (Tauri 2 + React/TS). Read AGENTS.md, then docs/simplicity-overhaul/PLAN.md in FULL — §3 (Voice Guide) and §4.2 are binding. You are executing session S4. S0 is complete and temporarily parked the old Advanced section's rows: app/output rows in a General fold, retention rows on History — read S0's Downstream Notes.

Mission: make General the calmest page in the app (≤7 default rows), fold everything else properly, and tidy History + About.

Research first: src/components/settings/general/GeneralSettings.tsx (+ ModelSettingsCard.tsx), history/HistorySettings.tsx (~940 lines — the view is already good, touch only settings rows), about/AboutSettings.tsx, and what S0 parked where.

Build exactly PLAN.md §4.2:
1. General default view: transcribe hotkey + push-to-talk + cancel, microphone, appearance + text size, the compact model-language card when relevant. MoreOptions fold: audio feedback + sound theme + output device + volume, launch on startup, start hidden, tray icon, overlay style, update checks. Every row gets the 3-tier Voice Guide treatment. (The overlay-style dropdown only picks a style — you never edit the overlay's own code, src/overlay/** is frozen.)
2. History: keep the content list view untouched; organize the retention/limit rows S0 parked into a small fold or bottom group with plain copy; give the empty state one warm line per the Voice Guide.
3. About: version, update check, app language, source/license, acknowledgements; data + log folder rows folded. Copy pass.
4. Note anything that ended up ownerless (a row S0 parked that belongs on Dictation, which S2 owns) in Downstream Notes rather than moving it yourself.

Ownership: src/components/settings/general/**, src/components/settings/history/**, src/components/settings/about/**, i18n namespaces settings.general*, settings.history.*, settings.about.*, settings.advanced.* only. FROZEN: src/assistant/**, src/overlay/**, Sidebar.tsx, App.tsx, bindings.ts, settings.rs, ui/ primitives. translation.json is shared with parallel sessions: edit only your namespaces, re-read before each edit.

Done when: bun run build + bun run lint green; all three pages eyeballed; General default view has ≤7 rows.

FINAL STEP (mandatory): update docs/simplicity-overhaul/PLAN.md — tick verified boxes, Status done + date, Evidence, Downstream Notes, Progress Log row.
```

---

## S5 — Copy & humor unification (WAVE 2 — run ALONE)

```
You are working on SpeakoFlow (Tauri 2 + React/TS). Read AGENTS.md, then docs/simplicity-overhaul/PLAN.md in FULL — §3 (Voice Guide) is your entire job. You are executing session S5, alone: you have exclusive ownership of src/i18n/locales/en/translation.json for this session.

Mission: five different sessions wrote copy in waves 0–1. Your job is to make the whole app sound like ONE warm, concise writer.

Method — be systematic, not vibes-based:
1. Read src/i18n/locales/en/translation.json top to bottom (~1300+ lines). For every string, check: 3-tier structure respected (title ≤4 words / caption ≤1 sentence / technical detail in info)? Banned words absent from titles+captions (post-process, inference, quantization, GGUF, endpoint, dtype, LLM, STT, VAD, tokens — allowed only in info tooltips and the advanced model catalog)? Sentence case? Humor only in allowed zones (model descriptions, empty states, download waits, onboarding, success), NEVER in errors/destructive confirmations/permissions/API-key rows?
2. Terminology unification: pick ONE term per concept and enforce it everywhere — hotkey vs shortcut, model vs engine, assistant vs AI, dictation vs transcription (user-facing). List your choices in Evidence.
3. Model copy pass: every speech model and LLM gets a friendly one-liner via the onboarding.models.<id>.name/description overlay keys (see src/lib/utils/modelTranslation.ts; fallback text lives in src-tauri/src/catalog/catalog.json — you may align those descriptions too, they're plain data, no logic). "Blazingly fast — great for everyday dictation" is the calibration bar. Exactly one Recommended per catalog context.
4. CRITICAL SAFETY: you may rewrite VALUES freely, but renaming KEYS breaks t() call sites. If you rename or delete a key, grep the src tree for it first and update every call site, then re-verify with build+lint. Prefer not renaming.
5. Do NOT touch the other 19 locale files. They fall back to English where keys are missing/stale; S6 records the debt.

Ownership: src/i18n/locales/en/translation.json (exclusive), src-tauri/src/catalog/catalog.json (description/name strings only), and any t() call-site updates forced by key changes. Nothing else. FROZEN: src/assistant/**, src/overlay/**, all component logic.

Done when: bun run build + bun run lint green; you provide in Evidence a summary of terminology decisions, humor placements added/removed, and any key renames with their call-site fixes.

FINAL STEP (mandatory): update docs/simplicity-overhaul/PLAN.md — tick verified boxes, Status done + date, Evidence, Downstream Notes, Progress Log row.
```

---

## S6 — QA & ship-readiness sweep (WAVE 3 — run ALONE, LAST)

```
You are working on SpeakoFlow (Tauri 2 + React/TS). Read AGENTS.md, then docs/simplicity-overhaul/PLAN.md in FULL, including every session's Evidence and Downstream Notes. You are executing session S6, the final gate before ship. Everything S0–S5 is done; your job is to catch what fell between the cracks.

Do a full, honest pass:
1. Fresh-state onboarding: clear/simulate first-run state, walk all steps including background downloads, "See all models", skip paths, and the try-it screen. Fix small issues directly.
2. Every sidebar section, every sub-page (model catalog, Profiles, Memory), every MoreOptions fold, every info tooltip: open them all. Check for: orphaned rows (settings that lost their home in the restructure — grep the settings store fields against rendered rows), duplicated rows (parked in two places by S0 and adopted twice), broken navigation, copy that violates the Voice Guide (PLAN.md §3), and inconsistencies between pages (fold labels, hotkey row styles, provider form layouts — they must be identical per §4.0).
3. FROZEN-SURFACE AUDIT (critical): confirm via git diff that src/assistant/** and src/overlay/** were NOT modified by this overhaul. If any session leaked changes into them, revert those hunks and note it.
4. Dead code sweep: the removed Advanced section shell, old onboarding components, orphaned i18n keys (grep for keys with zero t() call sites in the namespaces this overhaul touched). Delete confidently or document why kept.
5. Run bun run check:translations; record the missing-key debt per locale in TRANSLATIONS_TODO.md (do NOT machine-translate 19 locales in this session).
6. Cross-check docs/TODO_BEFORE_RELEASE.md: tick anything this overhaul resolved, add anything it surfaced.
7. Final green: bun run build, bun run lint, cargo fmt + cargo check, and describe a full manual smoke test (dictate, assistant chat, model switch, profile edit, memory export) in Evidence.

You may make small fixes anywhere in the repo except the frozen surfaces (this session has global ownership since nothing runs in parallel with it), but keep fixes surgical — big issues get filed as notes in PLAN.md's S6 Downstream Notes for the human, not heroically rewritten at the finish line.

FINAL STEP (mandatory): update docs/simplicity-overhaul/PLAN.md — tick S6's verified boxes, Status done + date, Evidence (the full QA checklist results), Downstream Notes (remaining known issues for the human), Progress Log row. Then write a short ship-readiness verdict at the top of the Progress Log.
```
