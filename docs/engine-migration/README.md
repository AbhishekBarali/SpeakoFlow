# Engine Migration — how to run this

A **self-driving project kit** for migrating SpeakoFlow's transcription to Handy's native
`transcribe.cpp` engine (GGUF models, real streaming, an optional live-transcription window, and
Handy's recommended model set) — **without breaking the current app**.

## The three files

- **`PLAN.md`** — the living source of truth. Architecture, the side-by-side (non-breaking) strategy,
  the confirmed file-by-file impact map, upstream facts, and **7 coherent sessions** — each with a
  detailed **Sub-steps** checklist, acceptance criteria, and checkboxes. This file gets **updated and
  ticked** as work completes.
- **`PROMPTS.md`** — one copy/paste prompt per session (7 total). Each makes the AI read `PLAN.md`, do
  that whole session, verify it, then tick it off.
- **`FOLLOW_HANDY.md`** — created in Session 7: the repeatable routine to pull future Handy engine
  updates cheaply.

## Why 7 sessions (not 21)

Each session is a **complete, independently-verifiable milestone** sized for one AI context — related
work (e.g. adding the engine enum + its load path + its transcribe call) is kept **together on
purpose**, because a capable model does better carrying those decisions forward in one session than
re-deriving them across many. The old per-step detail lives inside each session as a **Sub-steps**
checklist, so you get fewer hand-offs _and_ full guidance.

## The loop (what you do)

1. Open `PLAN.md`, find the next unchecked session (start at **Session 1**).
2. Open `PROMPTS.md`, copy the **Shared preamble** + that session's prompt.
3. Paste into a fresh AI session; let it do the whole session.
4. The AI implements all Sub-steps, runs the session's _Acceptance Criteria_ verification, then updates
   `PLAN.md`: session + Sub-step boxes → `[x]`, Status → `done (date)`, **Evidence** + **Downstream
   Notes**, and the **Progress Log** row.
5. You sanity-check (build runs? dictation works?), then move on.

```
   pick next session (PLAN.md) → paste preamble+prompt (PROMPTS.md)
        → AI: read PLAN → implement all Sub-steps → VERIFY
        → AI: tick [x] + Status + Evidence + Log → you confirm → next ↺
```

## Do the spike first

**Session 1 is the make-or-break step** — it proves `transcribe.cpp` compiles on your Windows/Vulkan
machine and transcribes one GGUF model _in isolation_ (batch + streaming), before any app code
changes. If anything's going to be hard, it surfaces here, cheaply. Don't skip ahead.

## Parallel options (optional, to save time)

Most sessions are sequential by design (each builds on the last, one agent carrying context). The
only splits worth doing:

- **Session 3** → **3a backend** (catalog + GGUF capability probe) ∥ **3b frontend** (model-selector
  UI). Run them as two sessions, then do a combined build check.
- **Session 7** build config can be **drafted alongside Sessions 3–5** and validated last.

## The headline features (so they don't get lost)

- **Real streaming** = **Session 4** — native `transcribe.cpp` streaming with committed/tentative
  text, replacing today's crude VAD-chunk hack. This is where Handy's quality comes from, and it only
  works with **streaming-capable models** (Parakeet Unified EN, Nemotron Streaming, …).
- **Optional live-transcription window** = **Session 5** — an on/off setting plus a resizable live
  card mirroring Handy's 400×120 streaming overlay. Off by default.
- **New recommended models** = **Session 3** (catalog) + **Session 6** (onboarding default) — Parakeet
  Unified EN 0.6B (#1), Nemotron Streaming 3.5 (#2), etc., mirroring Handy's ranking.

## Guardrails baked into every prompt

N1 never break the app · N2 additive/side-by-side (keep `transcribe-rs 0.3.11`) · N3 new features
default-off · N4 don't touch assistant/memory/TTS/web-search/vision · N5 no commits unless you ask.
Everything is on the `feat/transcribe-cpp-migration` branch, so `git checkout main` always restores
today's working app.

## Definition of done

A packaged build (from a clean machine, Session 7) dictates with a GGUF model, live streaming with
Parakeet Unified EN matches Handy's quality on a 10-minute session, the live-transcription window
toggles on/off, the new models are the recommended set, existing behavior is unchanged when streaming
is off, and `FOLLOW_HANDY.md` lets you pull the next Handy update without re-planning.
