# SpeakoFlow — Session Prompts (copy · paste · edit)

Each block below is a **complete, standalone prompt** for one AI session. Open a
fresh window, paste one prompt, let it run. Every prompt tells the AI to
**research the codebase itself and fully own the fix** — not half-ass it, not
trust a summary. Edit any prompt before pasting if you want to steer it.

Progress is tracked in [`RELEASE_CHECKLIST.md`](./RELEASE_CHECKLIST.md). Each
prompt ends with a mandatory step: after the work is built and verified, the
session ticks its own boxes there. That file is the single source of truth for
what's actually done.

## How to run them in parallel

Run one prompt per window. Prompts in the **same wave** touch different files, so
they're safe to run at the same time. Finish a wave before starting the next.

| Wave | Run these together | Why they don't collide |
|------|--------------------|------------------------|
| 1 | **P1** rebrand · **P2** logo · **P5** history fix · **P6** provider audit | config vs assets vs backend — disjoint |
| 2 | **P3** update/tray/about · **P7** model metadata · **P10** panel · (P6 cont.) | different windows + backend areas |
| 3 | **P8** models page · **P9** settings layout · **P11** assistant settings · **P13** optimization | disjoint frontend areas + backend |
| 4 | **P12** visual polish · **P14** security + QA sweep | polish after layout is final |

**The one rule:** never run two windows editing the same file. If unsure, run fewer.

Shared files that cause collisions (only one window edits these at a time):
`src/bindings.ts` (auto-generated — regenerate, don't hand-edit), `src-tauri/src/settings.rs`
(append fields only), `src-tauri/src/assistant.rs`, `src/i18n/locales/en/translation.json`.

---

## P1 — Erase every trace of "Handy" (full rebrand sweep)

```
You are working on SpeakoFlow, a local-first voice app built with Tauri 2 (Rust backend in src-tauri/src, React+TypeScript frontend in src). It was forked from an app called "Handy" (github.com/cjpais/Handy), and the fork is only partly rebranded. Your job is to find and fix EVERY remaining trace of the old brand — not just the obvious ones.

Do deep research first. Do NOT rely on one search or any summary. Build a complete inventory of old-brand references by searching the ENTIRE repo (code, config, docs, CI, installer scripts, AND file/asset names), case-insensitive, for: "handy", "cjpais", "com.pais.handy", "blob.handy.computer", "HANDY_", and any Handy-specific URLs. Search across .rs, .ts, .tsx, .json, .toml, .html, .css, .nsi, .plist, .yml/.yaml, .md, and filenames in src-tauri/icons and src-tauri/resources. Also open tauri.conf.json, Cargo.toml, package.json, the NSIS installer template, and any .github workflows.

Then classify each hit before changing anything:
- MUST rename to SpeakoFlow branding: product/app name, bundle identifier (currently com.pais.handy — pick a final id like com.abhishekbarali.speakoflow and use it everywhere, including the keychain service name in secret_store.rs and any app-data path), window titles, log file names, tray/resource asset filenames, env var prefixes, and any download URLs pointing at Handy infrastructure (e.g. blob.handy.computer for the VAD model — confirm it still works or self-host it).
- MUST KEEP (do not break): third-party names that legitimately contain the string, e.g. the "handy_keys" crate/library if it's an external dependency, and the upstream attribution required by the license (see below).

Legal requirement: SpeakoFlow is an MIT fork. Keep Handy's original copyright and MIT license text, and add a clear attribution/NOTICE crediting the upstream project. Do not strip the original license — only rebrand OUR product identity.

Implement the changes. If renaming the bundle identifier or asset files, update every place that references them (e.g. tray.rs hard-codes tray icon paths; the keychain service name affects saved secrets).

Constraints: keep the app compiling. New settings use #[serde(default)]. If you change backend commands, regenerate bindings.ts (tauri-specta).

Done when: a fresh case-insensitive repo-wide search for the old brand returns only (a) the required upstream attribution and (b) genuine third-party library names — nothing else. bun run tauri dev compiles and launches under the new identifier. Give me a written inventory of everything you found, what you changed, and what you deliberately kept and why.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P1) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

## P2 — Logo & full icon system

```
You are working on SpeakoFlow, a Tauri 2 desktop voice app (Rust + React/TS). It still ships the forked app's old hand/waveform logo. Design a fresh SpeakoFlow mark and regenerate the entire icon system.

Research first: find every icon and image the app actually uses. Look in src-tauri/icons (all platform sizes, .ico, .icns, Square*Logo.png, android mipmaps), src-tauri/resources (tray icons — there are idle/recording/transcribing variants for dark, light, and a colored/Linux set), the window/favicon references in the frontend HTML entry points, and how tray.rs::get_icon_path maps state+theme to files. Understand the full set before creating anything.

Design: SpeakoFlow is "calm, premium, quiet, trustworthy" (see docs/PRODUCT.md). Propose a simple, distinctive mark that reads at 16px (tray) and 1024px (store). One accent color moment, not RGB noise. Show me the concept (describe it or generate an SVG) before mass-producing sizes.

Implement: from one master asset, regenerate every required size/format (use the tauri icon generator for the app icons) AND all tray state variants (idle/recording/transcribing) for each theme. If you rename any file, update tray.rs and any other references so nothing breaks.

Done when: every icon slot is the new mark; the tray shows correct idle/recording/transcribing icons in light and dark; the installer and window use the new icon; nothing references a missing/old asset. bun run tauri dev compiles. Tell me which files you regenerated and confirm the tray path map is consistent.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P2) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

## P3 — App shell: update system, tray menu (+ Home), and About page

```
You are working on SpeakoFlow, a Tauri 2 voice app (Rust + React/TS) forked from Handy. Three things around the "app shell" are rough: the check-for-updates flow, the system-tray menu, and the About page. Investigate all three end to end and make them correct and clear.

Research first (trace the whole path, don't assume):
- Update system: read the updater config in tauri.conf.json (endpoints, pubkey), the frontend update-checker component (src/components/update-checker), the tray "check for updates" item, and how the footer shows the version. Understand exactly how a release becomes an update: who signs artifacts, what latest.json is, and where it's published. Identify what's missing or misleading for THIS repo (it was forked, so verify the endpoint points at the correct GitHub repo and that the signing pubkey has a matching private key story).
- Tray menu: read tray.rs and the tray menu event handler in lib.rs (the on_menu_event match). 
- About page: read src/components/settings/about/AboutSettings.tsx.

Implement:
- Add a "Home" item to the tray that opens/focuses the main window. Note that "Settings" already does this — decide with me in your writeup whether to rename Settings→Home or keep both; pick the least confusing option and implement it. Add the i18n string (tray_i18n.rs + locales).
- Make the update UX honest and clear: correct app name/version everywhere, a sensible "you're up to date / update available" state, and no leftover fork branding. If the release/signing pipeline is incomplete, implement what you can in-app and DOCUMENT the exact manual release steps (how to build signed updater artifacts and publish latest.json to GitHub releases) in docs/RELEASE_UPDATES.md.
- Clean the About page: SpeakoFlow name, version, links to the correct repo, license + upstream attribution.

Constraints: i18n for all strings; regenerate bindings.ts if commands change; keep it compiling.

Done when: tray has a working Home item; check-for-updates behaves correctly against the real endpoint (or fails gracefully with a clear message) and the release process is documented; About shows only SpeakoFlow info. Report what the update pipeline needs from me (keys, secrets) to fully work.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P3) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

## P5 — Fix History limit & recording retention (they don't prune)

```
You are working on SpeakoFlow, a Tauri 2 voice app. In Settings → Advanced there are "History limit" (max entries) and "Recording retention period" controls. They appear to not actually delete anything. Find the real cause and fix it properly.

Research first: trace the entire path. The setting fields live in settings.rs (history_limit, recording_retention_period); the commands are in commands/history.rs (update_history_limit, update_recording_retention_period); enforcement lives in managers/history.rs (there's count-based and time-based logic). Read all of it. Determine WHERE it breaks: is the limit never applied on insert? only on startup? is retention comparing the wrong units (seconds vs ms)? does it prune DB rows but leak WAV files (or vice-versa)? Confirm the frontend actually persists the values (HistoryLimit.tsx, RecordingRetentionPeriod.tsx).

Fix the root cause — don't paper over it. Make sure both dimensions work: (a) count limit prunes oldest entries beyond N, (b) retention deletes recordings older than the period, and (c) both delete the associated audio files, not just rows. Apply on insert AND on app start.

Add a focused test (Rust) proving pruning by count and by age. 

Done when: setting limit=5 with 10 entries prunes to 5 (and removes their WAVs); setting a short retention removes old recordings; a test covers both; cargo test + bun run tauri dev pass. Explain the actual bug you found.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P5) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

## P6 — Provider audit (LLM + web search + TTS) with live docs

```
You are working on SpeakoFlow, a Tauri 2 voice app. It integrates many external providers, and some may have drifted from current APIs. Audit every provider against its OFFICIAL, CURRENT documentation and fix what's wrong. Use the Firecrawl MCP tools (firecrawl_search / firecrawl_scrape) to read each provider's live API reference — do not rely on memory.

Scope — find the exact request each of these builds (URL, auth header, body shape, response parsing) by reading the code, then compare to the current official docs:
- Assistant LLM (assistant.rs, llm_client.rs): OpenAI, Azure OpenAI, Groq, OpenRouter, Anthropic, local Ollama/LM Studio, custom OpenAI-compatible.
- Web search (web_search.rs): Serper, Brave, Tavily, Exa, SerpAPI.
- Text-to-speech (tts.rs): Kokoro (local), OpenAI-compatible /audio/speech, ElevenLabs.

For each provider produce a row: endpoint · auth method · request body · response parse · matches current docs? (yes/no) · what's wrong · fix applied or issue to file. Save this as docs/PROVIDER_AUDIT.md.

Test what you can WITHOUT my keys: the keyless/free paths (e.g. Kokoro local, Ollama if available, any free-tier search) end to end. For keyed providers you can't run, verify the request shape against docs and note "needs live key to confirm."

Fix cheap drift directly (wrong header name, deprecated path, changed field). For anything risky or ambiguous, write it up rather than guessing.

Constraints: don't leak or hardcode any secret; keep it compiling; regenerate bindings.ts if commands change.

Done when: docs/PROVIDER_AUDIT.md is complete and honest; keyless paths verified live; clear fixes applied; remaining risks listed with the doc links you used.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P6) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

## P7 — Model catalog: names, descriptions, sizes, licenses

```
You are working on SpeakoFlow, a Tauri 2 voice app that downloads local models (transcription, language, speech). The model list shown to users has inconsistent names/descriptions and no license record. Curate the catalog and document licenses.

Research first: find the model registry/definitions in the backend (managers/model.rs and anywhere models are declared — id, display name, description, size, language flags, accuracy/speed ratings), and how the frontend renders them (settings/models, model-selector, lib/utils/modelTranslation and modelCategory). Understand the current entries (Whisper variants, Parakeet, Moonshine, an embedding/reranker if any, Kokoro TTS, the assistant LLMs like Gemma).

Curate: consistent, human display names; accurate one-line descriptions; correct sizes; honest accuracy/speed indicators; sensible grouping/labels ("Recommended", "English only", "Multi-language", "Translate to English"). Use Firecrawl to confirm real model details where unsure.

Legal (required before shipping download links): record each model's license and source (e.g. Whisper, Moonshine, NVIDIA Parakeet terms, Gemma license, Silero VAD, Kokoro). Put this in docs/MODELS.md.

Constraints: this is a metadata/curation task — don't redesign the Models page UI (that's a separate session). Keep the app compiling; i18n for user-facing strings; regenerate bindings.ts if the model struct changes.

Done when: catalog entries are consistent and accurate; docs/MODELS.md lists every bundled/downloadable model with its license + source; nothing references removed/renamed ids. Summarize the changes.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P7) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

## P8 — Redesign the Models page (run after P7)

```
You are working on SpeakoFlow, a Tauri 2 voice app (React/TS frontend). Redesign the Models settings page so choosing and managing models is clear and feels premium. Assume model names/descriptions/licenses are already curated (docs/MODELS.md).

Research first: read the current Models UI (src/components/settings/models — ModelsSettings.tsx, AddCustomModelDialog.tsx — and src/components/model-selector). Understand the three tabs (Transcription / Language Model / Speech), the active-model state, download/delete flows, download progress events, and the accuracy/speed indicators. Study docs/PRODUCT.md for the design language (quiet, premium, hierarchy over uniformity, one color moment) and reuse existing ui/ primitives.

Redesign (be tasteful and a bit creative, but stay consistent with the rest of the app): clearer active/downloaded/available states, obvious primary action per card, readable accuracy/speed, clean download progress + cancel, working language filter, and a sensible empty state. Don't invent new colors or break the token system.

Constraints: frontend only; touch only the models settings + model-selector files; i18n for all strings; keep it compiling and lint-clean. Do not edit unrelated settings pages.

Done when: the page has clear hierarchy, all flows work (select, download, cancel, delete, filter, add custom), light+dark are correct, and it matches the app's visual language. Show me before/after notes.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P8) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

## P9 — Reorganize Settings for clarity (information architecture)

```
You are working on SpeakoFlow, a Tauri 2 voice app (React/TS). The settings are confusing — Advanced is a grab-bag and related options are scattered. Redesign the settings information architecture so a normal user finds what they need fast and rare options recede.

Research first: map EVERY settings control that exists. Read src/components/Sidebar.tsx (the section list), src/App.tsx (routing + section headers/subtitles), and every component under src/components/settings (General, Advanced, History, Post-processing, Assistant, Debug, About and all the individual setting components). List each control and which group it's in today.

Design the new IA: group by user intent (e.g. essentials up top, output/paste behavior, transcription tuning, history/privacy, advanced/experimental hidden). Every screen should have one clear primary purpose; use progressive disclosure (the existing MoreOptions pattern) for rare settings. Keep the Assistant settings page out of scope (separate session owns it). Follow docs/PRODUCT.md.

Implement the reordering/regrouping. It's fine to move controls between sections and rename sections, but don't change what each control DOES. Update section labels + subtitles via i18n (sidebar.* and sectionSubtitles.*).

Constraints: touch Sidebar, App.tsx, and the settings section wrappers/components — NOT AssistantSettings.tsx and NOT the models page internals. i18n for all strings. Keep it compiling and lint-clean.

Done when: a first-time user can find shortcut, mic, model, output behavior, and history without hunting; advanced/experimental stuff is tucked away; nothing lost. Give me the old→new mapping.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P9) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

## P10 — Assistant panel: size, layout, and screenshot feature

```
You are working on SpeakoFlow, a Tauri 2 voice app. The floating Assistant panel (its own window) is too big and annoying, some content clips/overflows, and the screen-vision (screenshot) feature isn't used well. Investigate the whole panel and rework it.

Research first: read the assistant window frontend (src/assistant — AssistantPanel.tsx, AssistantPanel.css, main.tsx, useKokoroTts.ts), how the window is sized/positioned (overlay.rs or wherever the assistant window is created), the panel-size presets and pill/collapsed mode, and the screenshot pipeline end to end (screenshot.rs capture + the image-part assembly in assistant.rs + the arm/preview UI in the panel). Reproduce the problems: what clips, what's oversized, how the workflow list (e.g. Spec / Bug Fix) overflows, and how a user currently attaches a screenshot.

Rework (be thoughtful about UX):
- Sizing/layout: make Compact genuinely small, Standard/Large intentional, and ensure NOTHING clips at any size. Comfortable default. Drag + position persist. Fix the cramped workflow/entry area.
- Screenshot: make capture obvious and reliable — arm a capture, show a thumbnail/preview, let the user send or discard, and surface a clear error on failure. Respect the existing image-size budget so vision requests don't fail.

Constraints: this window is fairly isolated — stay in src/assistant, screenshot.rs, and the assistant-window sizing code. If you must edit assistant.rs, keep changes minimal and note them (other sessions may touch it). i18n for strings; keep it compiling.

Done when: the panel feels right at every size with zero clipping; the screenshot flow is obvious and reliable (verified against one vision-capable provider or with a clear manual test); pill mode works. Describe the problems you found and how you fixed them.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P10) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

## P11 — Assistant settings: split the file, fix prompts/personas/preview

```
You are working on SpeakoFlow, a Tauri 2 voice app. The Assistant settings page (src/components/settings/assistant/AssistantSettings.tsx) is a ~1400-line monolith covering provider, screen vision, web search, TTS, panel appearance (with a live preview), and the system prompt. It needs to be optimized, and the preview + multi-prompt/reference features need to actually work well.

Do this in two clear stages within the session:

Stage 1 — refactor with NO behavior change: read the whole file and the related backend (assistant.rs prompt assembly, settings.rs assistant_* fields, docs/prompts-reference.md). Split it into focused subcomponents (e.g. ProviderSection, ScreenVisionSection, WebSearchSection, VoiceSection, PanelAppearanceSection, SystemPromptSection). Verify nothing changed functionally.

Stage 2 — improve:
- Preview parity: the Panel Appearance preview must match the REAL assistant panel for theme, accent, size, text size, and opacity. Share styling with the actual panel so they can't drift.
- System prompt / reference: read docs/prompts-reference.md and the prompt assembly. Make the "reference/multiple system prompt" feature clear and token-lean (understand the caching-friendly append-only design before changing it). Ensure a custom prompt still works.
- Optional (fun, only if time): add a small set of selectable assistant "characters"/personas as prompt presets, with the custom option preserved. If you add a persona field, use #[serde(default)] and regenerate bindings.ts.

Constraints: own AssistantSettings.tsx and the assistant prompt/persona logic; coordinate any assistant.rs edits (other sessions may touch it — keep them surgical). i18n for strings; keep it compiling and lint-clean.

Done when: the file is cleanly split with identical behavior, the preview matches the real panel exactly, the system-prompt reference is clear and efficient, and (if added) personas work with custom still available. Summarize the split and the improvements.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P11) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

## P13 — Performance & resource optimization

```
You are working on SpeakoFlow, a Tauri 2 voice app (Rust + React/TS). Measure and improve its performance so it feels instant and stays out of the way on modest hardware.

Research/measure first (get real numbers, don't guess): cold-start time to usable UI, idle memory, memory during/after transcription and assistant use, model load/unload behavior (there's a ModelUnloadTimeout setting — confirm it actually unloads), and the release bundle size. Note the biggest costs.

Optimize the clear wins without changing behavior: avoid redundant work on hot paths (e.g. settings are re-read on every action — confirm caching is sane), ensure models unload when idle, trim obviously heavy startup work, and reduce bundle bloat where safe. Prefer measured, targeted changes over broad rewrites. Do NOT undertake the streaming-transcription core rewrite (that's explicitly post-v1).

Constraints: keep behavior identical; keep it compiling; note anything that needs my decision (e.g. dropping a dependency).

Done when: you report before/after numbers for startup, idle RAM, and bundle size; model unload verified; no new leaks; changes are safe and explained.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P13) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

## P12 — Global visual polish (run after P9)

```
You are working on SpeakoFlow, a Tauri 2 voice app (React/TS). The base UX works and settings are reorganized; now raise the whole interface to feel sleek and premium, consistently, without breaking anything.

Research first: read docs/PRODUCT.md (brand: calm, premium, quiet, one color moment, hierarchy over uniformity, anti-slop) and the existing design tokens/global styles (App.css and the shared ui/ components). Audit each screen (settings sections, models page, history, onboarding, the assistant panel, overlay, tray) and list where it looks generic, inconsistent, or cheap (spacing, type scale, shadows, borders, focus states, empty states).

Polish against the tokens — do not invent new colors or one-off styles. Improve consistency of spacing, typography, elevation, and interactive states; refine empty/loading/error states; make the "one color moment" land. Respect accessibility: WCAG AA contrast in light AND dark, visible focus rings on every control, and honor prefers-reduced-motion.

Constraints: ONE session owns the global token/CSS file to avoid churn — that's you. Prefer shared ui/ components over per-page hacks. Keep it compiling and lint-clean; don't change functionality.

Done when: the app looks cohesive and premium across every screen and both themes, AA contrast holds, focus rings are visible, reduced-motion is honored. Give me a per-screen before/after list.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P12) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

## P14 — Security review + pre-release QA sweep (gate before publishing)

```
You are working on SpeakoFlow, a Tauri 2 voice app about to be released as open source. Do a thorough security review and a full pre-release QA sweep, and produce a go/no-go report. Investigate for real — this is the last gate.

Security (find and fix or flag):
- tauri.conf.json: the CSP is null — define a proper Content-Security-Policy. The assetProtocol scope is "**" (the webview can read ANY file) — tighten it to only the dirs the app needs (recordings/models).
- Secrets: confirm API keys are stored in the OS keychain (secret_store.rs) on Win/mac/Linux and are NEVER written to settings_store.json or logs. Grep logs for secret leakage.
- Arbitrary execution: the text-replacements "[run]" magic command runs shell commands — confirm it's OFF by default and warns clearly, and can't be enabled silently by an imported rule file.
- Network egress: enumerate EVERY outbound request (model downloads from Hugging Face, VAD download, each provider API, web-search APIs, GitHub update check). Confirm there is NO silent telemetry. Write a PRIVACY section (what leaves the machine, when, and how to disable each) for the README.
- Dependencies: run cargo audit and bun audit; report/fix advisories.
- Updater: confirm the signing pubkey has a matching private key kept out of the repo, and that update artifacts are verified.

QA sweep (test and log pass/fail): fresh install → onboarding → first dictation into 2-3 real apps; assistant voice ask, typed ask, screen-vision, TTS, web search on at least one provider each; every global shortcut rebinds + persists; error states (no model, no mic, no network, bad key, provider down) show friendly messages and never crash; model download/cancel/resume/delete/switch; history add/prune-by-limit/prune-by-retention/save/delete/retry; light+dark on every screen; i18n falls back gracefully.

Constraints: fix the cheap, safe security issues directly; for anything risky, flag it with a clear recommendation instead of guessing.

Done when: CSP + asset scope tightened, secret handling verified, egress documented, audits run, and you deliver docs/RELEASE_QA.md with a checked list and a clear GO / NO-GO with blockers.

FINAL STEP — update the tracker (do not skip): only after the above is genuinely built and verified, open docs/RELEASE_CHECKLIST.md, tick ([ ]→[x]) the checkboxes for THIS task (P14) that truly pass, and fill its Verified: line with what you tested + today's date. Leave any unmet item unchecked with a short note — never tick something you didn't verify.
```

---

### Not in v1 (tell any session to ignore these)

Streaming/eager transcription (core rewrite — do alone, later), SQLite history,
token/cost display, provider prompt-cache tuning, local-LLM sidecar for offline
assistant, NPU/fan-silence tuning, and a large persona library. Park them and
ship the base experience first.

> Deeper reference (optional): [`RELEASE_PLAN.md`](./RELEASE_PLAN.md).
