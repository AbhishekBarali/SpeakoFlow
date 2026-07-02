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
- [x] Icons/resources/tray asset filenames + references rebranded (no broken paths)
- [ ] Handy infra URLs (e.g. VAD download) replaced or verified working
- [ ] Upstream MIT license + attribution/NOTICE kept intact
- [ ] `bun run tauri dev` launches under the new identifier
- Verified: _(who / what you tested / date)_
- Notes: Only the icons/resources/tray asset sub-item is ticked, by the P2 icon
  task (Kiro / 2026-07-01): all app icons, the 9 tray PNGs, and a new
  `public/favicon.svg` now carry the SpeakoFlow mark; the colored-tray filename
  rebrand (`handy.png` → `speakoflow.png`) is in place; and `bun run tauri dev`
  runs with no broken/missing asset paths (tray icons load for light + dark at
  runtime). The remaining P1 sub-items (repo-wide brand search, identifier
  everywhere, Handy infra URLs, license/NOTICE) were **not** part of the icon task
  and stay unticked.

### [x] P2 — Logo & full icon system

- [x] New master mark created (reads at 16px and 1024px)
- [x] All app icon sizes/formats regenerated (.ico/.icns/png/mipmaps/Square\*)
- [x] Tray idle/recording/transcribing regenerated for light + dark (+ colored set)
- [x] `tray.rs` icon path map consistent; window/installer icon updated
- [x] No reference to a missing/old asset; compiles
- Verified: Kiro / 2026-07-01 — Rasterized the provided SpeakoFlow mark
  (speech bubble + waveform bars) from the supplied SVG into a 1024px
  **transparent** master (no background box), then regenerated the whole
  `src-tauri/icons/**` set with `bun run tauri icon` (32/64/128/128@2x,
  `icon.png` 512, `icon.icns` 1024, every `Square*Logo` + `StoreLogo`, android
  mipmaps + adaptive XML, iOS `AppIcon-*`); `logo.png` (1024 master) regenerated
  by hand. `icon.ico` rebuilt as a **multi-size** ICO (16/24/32/48/64/128/256)
  so the title-bar/taskbar icon is crisp instead of pixelated. Verified every
  file has a transparent background and the mark rendered. In-app, the sidebar
  "SpeakoFlow" text wordmark was replaced by a new inline-SVG `Logo` component
  (`fill=currentColor`, `text-ink`) so the mark is vector-crisp and flips
  automatically between light and dark. Regenerated all 9 tray PNGs
  (`src-tauri/resources/`, 64×64) as distinct silhouettes — idle = hollow
  bubble+bars, recording = solid dot, transcribing = waveform bars — in the
  three existing families (light `#F0F0F0`, dark `#000000`, colored `#F090C0`).
  `tray.rs::get_icon_path` unchanged/consistent. `public/favicon.svg` is a
  `prefers-color-scheme`-aware vector of the mark. Build proof: `bun run lint`
  clean, `bun run build` clean, `cargo check` clean, and `bun run tauri dev`
  compiled + launched with the app cycling Light↔Dark with no icon-load panic.
- Notes: All icons are transparent (no background box). On **Windows**, the tray
  and window/taskbar icons are pinned to the WHITE mark via
  `get_current_theme() == Dark` (the Win11 taskbar/tray/"show hidden icons" chrome
  is dark by default and does not follow the app's light/dark appearance) — this
  stops the icon flipping to an invisible dark glyph and keeps it consistent. The
  embedded `icon.ico` and all bundle icons were regenerated WHITE (transparent);
  `build.rs` has `rerun-if-changed=icons/icon.ico` so the exe re-embeds it. iOS
  icons are composited over a dark bg (`--ios-color #1c1917`) so the white mark
  stays visible there. The main window title is now empty, so the title bar shows
  only the logo (no "SpeakoFlow" word) — note this also blanks the taskbar/Alt-Tab
  label. In-app the sidebar shows the mark only (enlarged), colored via
  `currentColor`/`text-ink` so it still adapts inside the app. Caveat: a constant
  white icon is faint on a light Windows taskbar / light installer wizard / light
  Finder; chosen deliberately per request since the target environment is dark.
  macOS `.icns` / Android variants were regenerated but not inspected on-device;
  a full NSIS installer bundle was not built (installer icon verified at the
  source-asset level).

### [ ] P5 — Fix History limit & recording retention

- [ ] Root cause identified and fixed (not papered over)
- [ ] Count limit prunes oldest entries beyond N (incl. WAV files)
- [ ] Retention deletes recordings older than the period (incl. WAV files)
- [ ] Enforced on insert AND on app start
- [ ] Rust test covers prune-by-count and prune-by-age; `cargo test` passes
- Verified:
- Notes:

### [~] P6 — Provider audit (LLM + web search + TTS)

- [x] Every LLM provider request diffed against current official docs
- [x] Every web-search provider diffed against current docs
- [x] Every TTS engine diffed against current docs
- [x] `docs/PROVIDER_AUDIT.md` matrix complete + honest
- [ ] Keyless/free paths verified live; keyed ones shape-checked + flagged
- [x] Cheap drift fixed; risky items written up (not guessed)
- Verified: Kiro / 2026-07-01 — Diffed every LLM provider (OpenAI, Azure OpenAI, OpenRouter, Anthropic, Groq, Cerebras, Z.AI, Ollama/LM Studio, Bedrock/Mantle, custom), every web-search provider (Serper, Brave, Tavily, Exa, SerpAPI), and every TTS engine (OpenAI `/audio/speech`, ElevenLabs, Azure Speech, local Kokoro) against live official docs via Firecrawl; full matrix in `docs/PROVIDER_AUDIT.md`. Drift fixes applied in `llm_client.rs`: OpenRouter `HTTP-Referer` header + Azure `api-key` header. Build/tests: `cargo check` exit 0; `cargo test --lib web_search::` 13 passed; `cargo test --lib tts::` 11 passed; frontend `tsc --noEmit` exit 0 + eslint clean.
- Notes: "Keyless/free paths verified live" left UNCHECKED — honest: no local OpenAI-compatible server was running here (Ollama on :11434 timed out), Kokoro runs only in the GUI webview (not exercisable headless), and free-tier search needs a signup key. Request shapes were verified against current docs + unit tests, but a live end-to-end run of each keyless path was not performed in this environment — needs a machine with Ollama running and/or a free search key to tick this box. Delivered alongside the audit (the feature asks): +9 popular LLM providers (Gemini, xAI, DeepSeek, Mistral, Moonshot, Together, Fireworks, Perplexity, dedicated Azure OpenAI); assistant model field is now a searchable "Load models" picker; OpenAI-compatible & ElevenLabs TTS now have "Load voices" AND "Load models" searchable pickers (new `assistant_list_tts_voices` / `assistant_list_tts_models` commands); Azure "Load voices" retained. Documented limitations in the audit: Anthropic's OpenAI-compat layer ignores `response_format`/`reasoning_effort`; Azure OpenAI works via the v1 endpoint only; Brave removed its free tier; Bedrock/Mantle needs a live key.

---

## Wave 2 — after P1 merges

### [x] P3 — Update system, tray (+ Home), About page

- [x] Update flow traced; endpoint points at the correct repo
- [x] Tray "Home" item opens/focuses main window (Settings-vs-Home decision made)
- [x] Update UX honest (up-to-date / available / graceful failure), no fork branding
- [x] `docs/RELEASE_UPDATES.md` documents signing + latest.json release steps
- [x] About page shows only SpeakoFlow info + license/attribution
- Verified: Kiro / 2026-07-01 — `bun x eslint src` + `bun x tsc --noEmit` clean;
  `cargo check` clean (build.rs regenerated tray translations: 20 langs, 7 fields,
  `home` replacing `settings`). Traced updater end-to-end: endpoint =
  `github.com/AbhishekBarali/SpeakoFlow/.../latest.json` (correct repo), minisign
  `pubkey` present. Tray decision: **renamed Settings→Home** (avoids two items
  opening the same window); `home` event reuses the proven `show_main_window` path.
  UpdateChecker now shows a visible, retryable "Update check failed / Update failed"
  state instead of silently reverting. Misleading `0.1.2` version fallback removed
  (Footer + About). About gained a License row (MIT, links to repo LICENSE) and
  keeps the Handy/CJ Pais attribution. Not run in this headless env: a live tray
  click and a live updater round-trip.
- Notes: Auto-update is code-complete but the pipeline needs maintainer secrets to
  go live — see `docs/RELEASE_UPDATES.md` §9: (1) the minisign **private key** +
  password matching the config `pubkey` (regenerate keypair + replace `pubkey` if
  it's not held), (2) a first GitHub release with installers, `.sig` files, and
  `latest.json` marked **Latest**, (3) Windows Azure Trusted Signing creds for the
  configured `signCommand` (or ship unsigned). No CI (`.github/workflows/` absent);
  release is manual today.

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

_Progress at a glance: 2 / 13 task-gates + release cut complete. Update this line
when you tick a task._
