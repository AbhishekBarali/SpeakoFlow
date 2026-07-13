# UX Overhaul & Handy v0.9.1 Backport — how to run this

A **self-driving project kit** for two bundled goals:

1. **Backport the worthwhile Handy v0.9.1 fixes** into SpeakoFlow — starting with the audio fix that's
   the prime suspect for the current transcription-**accuracy** problem.
2. **Overhaul the janky UI** — the recording overlay, the assistant panel's conversation view, the
   onboarding download flow, and the models settings page — so a first-time user isn't overwhelmed.

Same format as `docs/engine-migration/`: a living plan, one copy-paste prompt per session, verify + tick.

## The three files

- **`PLAN.md`** — the living source of truth. Goal, guardrails, the **precondition + parallelization
  map**, the file-by-file impact map, the **Handy v0.9.1 backport reference table** (what to port,
  what's already done, what to skip), and **6 coherent sessions** — each with a **Sub-steps** checklist,
  acceptance criteria, and checkboxes. This file gets **updated and ticked** as work completes.
- **`PROMPTS.md`** — one copy/paste prompt per session (6 total) + a shared preamble. Each makes the AI
  read `PLAN.md`, do that whole session, verify it, then tick it off.
- **`README.md`** — this file.

## ⚠️ Read this before you start: the order matters

**Finish the engine migration (`docs/engine-migration` Sessions 5→6→7) before Sessions 3, 5, and 6 here.**
Those UI sessions touch the _same files_ the migration's remaining sessions finalize (the overlay, the
onboarding flow, the models list). Cleaning them up before the migration lands means the migration will
stomp your work.

**The one exception — do Session 1 now.** Session 1 (audio & accuracy) only touches `audio_toolkit/`,
which the migration doesn't, and it fixes the accuracy bug you're hitting _right now_ (a resampler that
isn't reset between recordings — Handy's PR #1344). **Session 2** (backend robustness) is also
independent and can run anytime. Pull both forward if you like.

## The loop (what you do)

1. Open `PLAN.md`, find the next unchecked session.
2. Open `PROMPTS.md`, copy the **Shared preamble** + that session's prompt.
3. Paste into a fresh AI session; let it do the whole session.
4. The AI implements all Sub-steps, runs the _Acceptance Criteria_ verification (build / `tsc` / eslint /
   tests), then updates `PLAN.md`: boxes → `[x]`, Status → `done (date)`, **Evidence** + **Downstream
   Notes**, and the **Progress Log** row.
5. You sanity-check — some acceptance criteria are visual, so the AI will list what _you_ need to eyeball
   (an overlay screenshot, a voice turn in the panel, the onboarding walkthrough, the models page).

```
   pick next session (PLAN.md) → paste preamble+prompt (PROMPTS.md)
        → AI: read PLAN → implement all Sub-steps → VERIFY (build/tsc/eslint)
        → AI: tick [x] + Status + Evidence + Log → you eyeball the UI → next ↺
```

## The 6 sessions at a glance

| #   | Session                             | What it fixes                                                                              | When                      |
| --- | ----------------------------------- | ------------------------------------------------------------------------------------------ | ------------------------- |
| 1   | **Audio & accuracy** (#1344, #1582) | the resampler-crosstalk **accuracy bug** + faster mic start                                | **now**                   |
| 2   | **Robustness backports**            | don't wipe settings on error (#1631), tray on Windows, crash-on-quit, injection defense, … | **now**                   |
| 3   | **Overlay cleanup** (#1597)         | drop the verbose label, remove the ✓ button (keep X), show "Processing…"                   | after migration           |
| 4   | **Assistant panel polish**          | show your spoken/typed message + a cleanly-streamed reply; fix text overflow/sizing        | after migration           |
| 5   | **Onboarding non-blocking** (#1522) | stop the stuck loading screen; background progress; kill the raw-key error                 | after migration           |
| 6   | **Models settings redesign**        | search bar, plain-language descriptions, tame "Legacy", calm & uncluttered                 | after migration, after S5 |

## Parallelization (optional, to save time)

- **Run in parallel:** Sessions **1, 2, 3, 4** touch four disjoint areas (audio-toolkit / backend
  robustness / overlay / assistant). Any of them can run at the same time.
- **Sequential lane:** Sessions **5 and 6** both edit `ModelCard.tsx` and `modelStore.ts` — do **5 then
  6**, never in parallel.
- **Session 6 can split** into **6a** (layout/search/legacy-grouping) ∥ **6b** (copywriting the
  descriptions), then a combined build check.

## Big sessions on purpose

Each session is a **complete, independently-verifiable milestone** sized for one capable AI context —
related work is kept together rather than fragmented into micro-tasks. The detail lives inside each
session as a **Sub-steps** checklist, so you get fewer hand-offs _and_ full guidance.

## Guardrails baked into every prompt

G1 never break the app (build + `tsc` + dictate/chat) · G2 don't regress the engine migration (no
touching the transcribe.cpp decode/catalog internals) · G3 backports are faithful ports of the Handy
PRs · G4 all strings via i18n · G5 preserve working behavior, default-off for experimental · G6 no
commits unless you ask.

## Definition of done

The accuracy bug is gone on repeated recordings; the recording overlay is a clean minimal chip; the
assistant panel shows your message and a smoothly-streamed reply without breaking layout; onboarding
downloads in the background with visible progress and no raw-key error; the models page is calm, has a
search bar, plain-language descriptions, and doesn't overwhelm a first-timer; and the worthwhile Handy
v0.9.1 fixes (settings salvage, tray, mic init, injection defense, …) are in — with everything still
building and dictating.
