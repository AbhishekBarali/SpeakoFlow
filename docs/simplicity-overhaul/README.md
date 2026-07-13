# SpeakoFlow — Simplicity Overhaul (operating manual)

> **The problem:** the app works, but a first-time user can't use it. Ten sidebar
> sections, three pages with their own provider/API-key forms, an onboarding that
> reads like a research catalog, and settings copy written for the developer.
>
> **The goal:** a user who downloads SpeakoFlow understands it in 60 seconds.
> Simple users dictate and never open settings. Power users find every knob —
> behind "More options", not in their face.

This folder is the single source of truth for the overhaul:

| File                         | What it is                                                                                                    | Who reads it                        |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------- | ----------------------------------- |
| [`PLAN.md`](./PLAN.md)       | The living tracker: target design spec, voice guide, session list with checkboxes, evidence, downstream notes | **Every session, at start AND end** |
| [`PROMPTS.md`](./PROMPTS.md) | One complete copy-paste prompt per session                                                                    | **You** (the human)                 |
| `README.md`                  | This file — how to run the show                                                                               | You                                 |

## How you run this (the loop)

1. Open `PROMPTS.md`, copy the next prompt for the current wave.
2. Paste it into a fresh AI session (Opus). Let it run to completion.
3. The session's mandatory final step is to update `PLAN.md` (tick its boxes,
   fill Evidence + Downstream Notes). **If a session didn't update PLAN.md, it
   isn't done — tell it to finish.**
4. When every session in a wave shows `done` in PLAN.md, start the next wave.
5. Between waves, do a 2-minute human smoke test: `bun run tauri dev`, dictate
   once, open every sidebar section. If something's broken, paste the "fix-up"
   note into a new session before proceeding.

## Waves — what runs in parallel

| Wave  | Sessions (run together)                                                                                             | Why they don't collide                                                                                              |
| ----- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| **0** | **S0** Foundation & new information architecture                                                                    | ALONE — it owns `Sidebar.tsx`, `App.tsx`, `ui/` primitives, `settings.rs`, `bindings.ts`. Everything depends on it. |
| **1** | **S1** Onboarding ∥ **S2** Dictation page ∥ **S3** Assistant + Profiles + Memory ∥ **S4** General + History + About | Disjoint component folders. Each owns its own i18n namespace (see rule below).                                      |
| **2** | **S5** Copy & humor unification                                                                                     | ALONE — it owns `translation.json` exclusively.                                                                     |
| **3** | **S6** QA & ship-readiness sweep                                                                                    | ALONE, last — global read, surgical fixes, frozen-surface audit.                                                    |

**Hard rules:**

- **Frozen surfaces — nobody touches them:** the floating assistant panel
  (`src/assistant/**`) and the recording overlay (`src/overlay/**`, `overlay.rs`).
  This overhaul is the settings window + onboarding only. No setting is removed,
  no behavior changes — reorder, regroup, fold, reword.
- **Never run two sessions from different waves at the same time.**
- **`src/i18n/locales/en/translation.json` is the one shared file in Wave 1.**
  Each session may only add/edit keys inside its assigned namespace (listed in
  its prompt), must re-read the file immediately before each edit, and must keep
  edits small. If a session notices its keys vanished (clobbered by a parallel
  session), it re-adds them — the namespaces don't overlap, so this is always safe.
- **Nobody edits `src/bindings.ts` by hand** except S0 (which follows the repo's
  regeneration procedure). No other session may add/remove Tauri commands.
- **Only English copy.** No session touches the other 19 locales; S6 records the
  translation debt in `TRANSLATIONS_TODO.md`.
- Every session must leave the app green: `bun run build` + `bun run lint`
  (+ `cargo fmt` / `cargo check` if it touched Rust).

## If a session gets blocked

It must mark its PLAN.md entry `[!] blocked` with a one-paragraph explanation in
Evidence, and stop. You then either resolve it yourself or paste the explanation
into a fresh session with: _"Read docs/simplicity-overhaul/PLAN.md. Session Sx
is blocked — here's why: … Unblock it and finish the session."_

## Relationship to older docs

- `docs/ux-overhaul/` and `docs/SESSION_PROMPTS.md` are **earlier, mostly
  completed** efforts (visual polish, rebrand, backports). This plan supersedes
  their unfinished UI items. Don't re-run them.
- `docs/TODO_BEFORE_RELEASE.md` still applies (mirrors, signing, etc.) — S6
  cross-checks it.
