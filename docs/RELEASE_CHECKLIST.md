# SpeakoFlow — Release Checklist (single source of truth for progress)

This is the shared progress tracker. Every session updates it **after** finishing
and **actually verifying** its task. The prompts in
[`SESSION_PROMPTS.md`](./SESSION_PROMPTS.md) map 1:1 to the tasks here.

## Ticking rules (read before you check a box)

- Only change `[ ]` → `[x]` when the work is **done AND verified** — you built it
  (`bun run tauri dev` compiles), you ran the checks, and the acceptance
  sub-items below actually hold. A command exiting without error is **not** proof.
- Tick each acceptance sub-item individually. Only tick the main task box when
  **all** its sub-items are ticked.
- If something is partially done or you're unsure, **leave it unticked** and add a
  note. Do not tick to "look finished."
- When you tick a task, fill its `Verified:` line with a one-line note of what you
  tested + the date. Commit this file with your change.
- If you discover the task can't be fully completed, leave it unticked, tick only
  the sub-items that are truly done, and write what's blocking it under `Notes:`.

Legend: `[ ]` not started/incomplete · `[~]` in progress · `[x]` done & verified

---

## Wave 1 — run in parallel

### [ ] P1 — Erase every trace of "Handy" (full rebrand)
- [ ] Repo-wide case-insensitive search for old brand returns only required attribution + genuine 3rd-party names
- [ ] Bundle identifier changed everywhere (config, keychain service name, app-data path)
- [ ] Icons/resources/tray asset filenames + references rebranded (no broken paths)
- [ ] Handy infra URLs (e.g. VAD download) replaced or verified working
- [ ] Upstream MIT license + attribution/NOTICE kept intact
- [ ] `bun run tauri dev` launches under the new identifier
- Verified: _(who / what you tested / date)_
- Notes:

### [ ] P2 — Logo & full icon system
- [ ] New master mark created (reads at 16px and 1024px)
- [ ] All app icon sizes/formats regenerated (.ico/.icns/png/mipmaps/Square*)
- [ ] Tray idle/recording/transcribing regenerated for light + dark (+ colored set)
- [ ] `tray.rs` icon path map consistent; window/installer icon updated
- [ ] No reference to a missing/old asset; compiles
- Verified:
- Notes:

### [ ] P5 — Fix History limit & recording retention
- [ ] Root cause identified and fixed (not papered over)
- [ ] Count limit prunes oldest entries beyond N (incl. WAV files)
- [ ] Retention deletes recordings older than the period (incl. WAV files)
- [ ] Enforced on insert AND on app start
- [ ] Rust test covers prune-by-count and prune-by-age; `cargo test` passes
- Verified:
- Notes:

### [ ] P6 — Provider audit (LLM + web search + TTS)
- [ ] Every LLM provider request diffed against current official docs
- [ ] Every web-search provider diffed against current docs
- [ ] Every TTS engine diffed against current docs
- [ ] `docs/PROVIDER_AUDIT.md` matrix complete + honest
- [ ] Keyless/free paths verified live; keyed ones shape-checked + flagged
- [ ] Cheap drift fixed; risky items written up (not guessed)
- Verified:
- Notes:

---

## Wave 2 — after P1 merges

### [ ] P3 — Update system, tray (+ Home), About page
- [ ] Update flow traced; endpoint points at the correct repo
- [ ] Tray "Home" item opens/focuses main window (Settings-vs-Home decision made)
- [ ] Update UX honest (up-to-date / available / graceful failure), no fork branding
- [ ] `docs/RELEASE_UPDATES.md` documents signing + latest.json release steps
- [ ] About page shows only SpeakoFlow info + license/attribution
- Verified:
- Notes:

### [ ] P7 — Model catalog: names, descriptions, sizes, licenses
- [ ] Display names + descriptions consistent and accurate
- [ ] Sizes + accuracy/speed indicators correct; grouping/labels sensible
- [ ] `docs/MODELS.md` lists every model with license + source
- [ ] No references to removed/renamed model ids; compiles
- Verified:
- Notes:

### [ ] P10 — Assistant panel: size, layout, screenshot feature
- [ ] Compact truly compact; Standard/Large intentional; nothing clips at any size
- [ ] Comfortable default; drag + position persist; pill mode works
- [ ] Screenshot flow: capture → thumbnail/preview → send or discard
- [ ] Capture failures show a clear error; image-size budget respected
- [ ] Verified against one vision-capable provider (or clear manual test)
- Verified:
- Notes:

---

## Wave 3 — feature/IA build

### [ ] P8 — Redesign the Models page (needs P7)
- [ ] Clear active/downloaded/available states + one primary action per card
- [ ] Download progress + cancel + delete + language filter all work
- [ ] Add-custom-model flow works; sensible empty state
- [ ] Light + dark correct; matches design tokens (no new one-off colors)
- Verified:
- Notes:

### [ ] P9 — Reorganize Settings (information architecture)
- [ ] Every existing control mapped old→new
- [ ] Grouped by user intent; each screen has one clear purpose
- [ ] Rare/experimental options behind progressive disclosure
- [ ] No control's behavior changed; nothing lost; i18n labels updated
- Verified:
- Notes:

### [ ] P11 — Assistant settings: split file, prompts, personas, preview
- [ ] Monolith split into focused subcomponents with identical behavior
- [ ] Panel-appearance preview matches the REAL panel (theme/accent/size/text/opacity)
- [ ] System-prompt / reference feature clear + token-lean; custom prompt still works
- [ ] (Optional) selectable personas work; custom preserved
- Verified:
- Notes:

### [ ] P13 — Performance & resource optimization
- [ ] Before/after numbers captured: startup, idle RAM, bundle size
- [ ] Model unload verified (ModelUnloadTimeout fires)
- [ ] Targeted wins applied; no behavior change; no new leaks
- Verified:
- Notes:

---

## Wave 4 — polish + gate

### [ ] P12 — Global visual polish (after P9)
- [ ] Consistent spacing/type/elevation/interactive states across every screen
- [ ] "One color moment" lands; no invented colors / one-off styles
- [ ] WCAG AA contrast in light + dark; visible focus rings; reduced-motion honored
- [ ] Per-screen before/after notes provided; functionality unchanged
- Verified:
- Notes:

### [ ] P14 — Security review + pre-release QA sweep (GATE)
- [ ] Real CSP set; `assetProtocol.scope` tightened from `**`
- [ ] Secrets confirmed in OS keychain; none in settings file or logs
- [ ] `[run]` magic command off by default + warns; can't enable silently
- [ ] Network egress enumerated; PRIVACY section written; no silent telemetry
- [ ] `cargo audit` + `bun audit` run; advisories fixed/flagged
- [ ] Updater signing key kept out of repo; artifacts verified
- [ ] Full QA sweep logged (install, dictation, assistant, errors, models, history, themes, i18n)
- [ ] `docs/RELEASE_QA.md` delivered with a clear GO / NO-GO + blockers
- Verified:
- Notes:

---

## Release cut (do last, in order — human-owned)

- [ ] Final bundle identifier + version locked (`tauri.conf.json`, `Cargo.toml`, `package.json`)
- [ ] Changelog + `PROGRESS.md` updated
- [ ] Tag + GitHub release; CI builds signed artifacts + `latest.json`
- [ ] Auto-update verified end to end (old build → update → new build)
- [ ] README screenshots current; LICENSE + attribution present
- [ ] GO decision recorded

---

_Progress at a glance: 0 / 13 task-gates + release cut complete. Update this line
when you tick a task._
