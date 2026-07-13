# UX Overhaul & Backport Prompts — copy/paste one at a time

> **How to use:** paste the **Shared preamble** + the next session's prompt into a fresh AI session.
> The AI reads `PLAN.md`, does that whole session (all its Sub-steps, in one context), verifies it,
> then ticks it off in `PLAN.md`. Come back, confirm, paste the next one.
>
> There are **6 sessions**. Read **§2 of PLAN.md first** for the precondition + order:
>
> - **Sessions 1 & 2** are independent of the engine migration — run them **now** (Session 1 fixes a
>   live accuracy bug).
> - **Sessions 3–6** need the engine migration merged first. **3 and 4 can run in parallel.**
>   **5 and 6 share files — run 5 then 6, never in parallel.**
> - Session 6 can optionally split into **6a (layout/search)** ∥ **6b (copywriting)**.

---

## Shared preamble (prepend to EVERY session prompt)

```
You are an executor AI working on the SpeakoFlow desktop app (a Tauri 2 fork of Handy) at the
current repo root. This is the post-migration polish/backport effort.

BEFORE DOING ANYTHING:
1. Read docs/ux-overhaul/PLAN.md in full (especially §1 non-negotiables, §2 precondition &
   sequencing, §3 impact map, §4 the Handy v0.9.1 backport reference, and the specific Session
   block below with its Sub-steps).
2. Read docs/ux-overhaul/README.md for the workflow.

HARD RULES (PLAN.md §1):
- G1 Never break the working app — it must build (cargo build + bun x tsc --noEmit) and
  dictate + assistant-chat at the end of this session. Run the checks before ticking a box.
- G2 Don't regress the engine migration — do NOT touch the transcribe.cpp engine decode/stream/
  catalog internals (managers/transcription.rs decode arms, model_capabilities.rs, gguf_meta.rs,
  src/catalog/**, transcribe_cpp_run_plan).
- G3 Backports are faithful ports — read the referenced Handy PR, port the intent to SpeakoFlow's
  (diverged) code, and note what changed.
- G4 i18n always — every user-facing string via t() + en/translation.json. No hardcoded JSX.
- G5 Preserve behavior that works; default-off for experimental.
- G6 No git commits, no force-push, no dependency rewrites beyond this session's scope.

DO THE WHOLE SESSION BELOW (all its Sub-steps, in this one context). When finished:
- Verify against the session's Acceptance Criteria (run the actual build/tsc/eslint/tests; paste output).
- In PLAN.md: set the session checkbox and every Sub-step box to [x], Status to "done (YYYY-MM-DD)",
  fill Evidence (what proved it) and Downstream Notes, and update the §6 Progress Log row.
- If blocked, set [!] + Status "blocked" and explain in Evidence; do not fake completion.
- Some acceptance criteria need the maintainer to eyeball a screen — do everything you can verify
  headlessly (build/tsc/eslint), then clearly list what the human still needs to confirm.
- STOP after this one session. Do not start the next.
```

---

## Session 1 — Audio & accuracy fixes (backport #1344, #1582) [RUN NOW]

```
Execute Session 1 from PLAN.md (all Sub-steps). This fixes a live transcription-accuracy bug, so do
it carefully. Read Handy PR #1344 (reset resampler state between recordings). In
src-tauri/src/audio_toolkit/audio/resampler.rs, FrameResampler holds in_buf + pending buffers and has
NO reset() — add one that clears those buffers and reinitialises the FftFixedIn state, and call it at
the start of every recording (in recorder.rs run_consumer, or create a fresh resampler per recording)
so leftover samples from a previous recording can never bleed into the next. Then read Handy PR #1582
(faster mic initialization) and port their approach, reconciling with managers/audio.rs
wait_for_capture_ready (currently dead-code-flagged) so the first word isn't clipped. Optionally port
#1444 (throttle mic-level IPC) if cheap. Add a unit test for the resampler reset if feasible. Verify
cargo build is clean and the reset is provable, then update PLAN.md. NOTE in Evidence that the
maintainer must confirm the real-world accuracy improvement on repeated recordings.
```

## Session 2 — Robustness backports [RUN NOW]

```
Execute Session 2 from PLAN.md (all Sub-steps). Port these Handy v0.9.1 fixes to SpeakoFlow's diverged
code, reading each PR first and noting whether it was already present:
- #1631 (HIGH VALUE): when the settings store fails to parse, salvage the valid fields instead of
  wiping the whole store. Test with a corrupt/partial store.
- #1636 + #1158 + #1355: track tray-icon state; fix the tray icon invisible on a dark Windows taskbar
  with light apps; log tray failures instead of panicking (tray.rs).
- #1354: guard Drop impls that lock a mutex against poisoned mutexes so quit can't abort.
- #1187: fix cyrillic/unicode file-path problems (model/cache paths) on Windows.
- #1310: add prompt-injection defense to the default post-processing prompt (harden what
  build_system_prompt consumes in actions.rs).
- #1569: preserve ampersands in custom words (apply_custom_words in audio_toolkit/text.rs).
Verify cargo build + cargo test --lib clean and the corrupt-store test keeps valid settings, then
update PLAN.md (record which PRs were already present vs newly ported in Downstream Notes).
```

## Session 3 — Recording overlay cleanup (backport #1597) [after migration merged]

```
Execute Session 3 from PLAN.md (all Sub-steps). In src/overlay/RecordingOverlay.tsx (+ .css), clean up
the recording overlay to match SpeakoFlow's minimalist look, for BOTH the compact pill and the
streamingWindow card:
- Drop the verbose "Recording hands-free — press the hotkey…" text from the visible chip (keep a terse
  state word or just the waveform); KEEP the full phrasing on the aria-label for accessibility.
- Remove the ✓ confirm button (card-confirm → commands.commitRecording()); keep only the X cancel
  button. Hands-free now stops via the hotkey only (intended). Leave the Rust commit_recording command.
- Backport #1597: show a clear "Processing…" state for the non-streaming (batch) case so a batch model
  doesn't look frozen after you stop talking.
- Tidy spacing/alignment for a calm card.
Verify bun x tsc --noEmit + bun x eslint on the changed files are clean, then update PLAN.md. Note in
Evidence that the maintainer should eyeball the overlay.
```

## Session 4 — Assistant panel conversation polish [after migration merged; parallel with S3]

```
Execute Session 4 from PLAN.md (all Sub-steps). This is a FRONTEND rendering/UX pass on
src/assistant/AssistantPanel.tsx (+ .css) — do NOT touch assistant.rs / run_assistant_turn.
1. AUDIT FIRST: read AssistantPanel.tsx + .css and record in Downstream Notes what the conversation
   view does today (does the user's spoken/typed message render as a bubble? how does the reply stream?
   where does text overflow/break layout?).
2. Make the user's message always show as a clean bubble — typed OR spoken (the transcript appears in
   the thread, not just the answer).
3. Make the assistant reply stream cleanly into a readable box; if the raw token stream is too fast/
   janky, smooth the render (rAF-batched or a small typewriter cadence) — readability over raw speed.
4. Fix the "weird" cases: long words/URLs/code overflowing the width, messages exceeding panel height
   without scroll, markdown/code blocks breaking layout, text clipping at edges. Ensure proper wrap,
   max-width, and scroll-to-latest. Keep it consistent across the three panel sizes + the collapsed pill.
Verify bun x tsc --noEmit + eslint clean, then update PLAN.md. Note the screens the maintainer should
confirm (a voice turn + a long/code reply).
```

## Session 5 — Onboarding: non-blocking downloads + fix the error (backport #1522) [after migration; before S6]

```
Execute Session 5 from PLAN.md (all Sub-steps). Fix the onboarding download experience:
- Make the Step-1 (STT) model download NON-BLOCKING, like Step-2 (LLM) already is: start the download,
  show background progress, and let the user proceed instead of being stuck on a loading screen. Reuse
  the aiModel.downloadingHint / downloadStarted pattern in LlmOnboarding.tsx + modelStore.ts.
- Show download progress in a consistent, non-blocking place (progress row / corner toast) with size +
  percent, for both STT and LLM downloads.
- Fix the onboarding.errors.selectModel toast: (a) it currently renders the RAW KEY — make it resolve
  to the friendly string (the key exists in en/translation.json under onboarding.errors; diagnose the
  resolution/timing issue). (b) Diagnose WHY selectModel fails right after a fresh download (engine load?
  race with verify/extract?) and make selection robust so a retry that will succeed doesn't spam an
  error. (c) Backport #1522: surface REAL download errors clearly, not a generic key.
- Don't block Skip/Continue while a background download runs.
Verify bun x tsc --noEmit + eslint clean, then update PLAN.md. Record the root cause of the selectModel
failure in Downstream Notes; note the onboarding walkthrough the maintainer should confirm.
```

## Session 6 — Models settings redesign [after migration + after S5]

```
Execute Session 6 from PLAN.md (all Sub-steps). Redesign the Models settings page to be clean, calm and
obvious, matching SpeakoFlow's minimalist look (reference the minimalist-ui / impeccable design skills).
Files: src/components/settings/models/ModelsSettings.tsx, src/components/onboarding/ModelCard.tsx
(shared with onboarding — keep it working there), src/lib/utils/modelQuant.ts, en/translation.json.
- Add a model-NAME search bar (like Handy) to the Transcription tab (and LLM/Speech). Keep Downloaded vs
  Available clearly separated, recommended first.
- Reduce congestion: de-emphasize the accuracy/speed meters (smaller/secondary), lead with the model
  name + a one-line plain-language description.
- Tame "Legacy": stop the loud per-card badge (isLegacyStt in ModelCard.tsx ~L116); group old-engine
  models under a collapsed "Older models" section (or a quiet low-contrast tag) so a first-timer sees
  only the recommended few.
- Rewrite the descriptions in en/translation.json to be simple and human ("Blazingly fast, great for
  everyday dictation", "Real-time, works on any machine", "Slower but the most accurate", "Best for many
  languages") — and verify each matches its model. Apply across Transcription, Language Model, Speech.
OPTIONAL: split into 6a (layout/search/legacy-grouping in .tsx) ∥ 6b (copywriting in .json), then a
combined build check. Verify bun x tsc --noEmit + eslint clean, then update PLAN.md with before/after
screenshots for the maintainer to confirm.
```
