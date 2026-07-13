# Migration Prompts — copy/paste one at a time

> **How to use:** paste the **Shared preamble** + the next session's prompt into a fresh AI session.
> The AI reads `PLAN.md`, does that whole session (all its Sub-steps, in one context), verifies it,
> then ticks it off in `PLAN.md`. Come back, confirm, paste the next one.
>
> There are **7 sessions**, each a coherent end-to-end milestone. Do them in order. The only
> parallel options: Session 3 can split into **3a backend ∥ 3b frontend**, and Session 7's build
> config can be drafted alongside Sessions 3–5 (see `PLAN.md` §5).

---

## Shared preamble (prepend to EVERY session prompt)

```
You are an executor AI working on the SpeakoFlow desktop app (a Tauri 2 fork of Handy) at the
current repo root. We are migrating transcription to Handy's native transcribe.cpp engine.

BEFORE DOING ANYTHING:
1. Read docs/engine-migration/PLAN.md in full (especially §1 non-negotiables, §2 architecture,
   §3 impact map, §4 upstream facts, and the specific Session block below with its Sub-steps).
2. Read docs/engine-migration/README.md for the workflow.

HARD RULES (PLAN.md §1):
- N1 Never break the working app — it must build and dictate at the end of this session.
- N2 Additive/side-by-side — do NOT remove or downgrade transcribe-rs 0.3.11 or its features
  unless the session explicitly says so.
- N3 New settings/features default OFF.
- N4 Do NOT touch assistant.rs, memory.rs, tts.rs, web_search.rs, screenshot.rs, llm_client.rs,
  or history storage.
- N5 No git commits, no force-push, no dependency rewrites beyond this session's scope.

DO THE WHOLE SESSION BELOW (all its Sub-steps, in this one context — that's intentional; don't
split them into separate runs). When finished:
- Verify against the session's Acceptance Criteria (run the actual build/tests; paste output).
- In PLAN.md: set the session checkbox and every Sub-step box to [x], Status to "done (YYYY-MM-DD)",
  fill Evidence (what proved it) and Downstream Notes (anything later sessions need), and update the
  §9 Progress Log row.
- If blocked, set [!] + Status "blocked" and explain in Evidence; do not fake completion.
- STOP after this one session. Do not start the next.
```

---

## Session 1 — Native engine spike & isolation proof (GO/NO-GO)

```
Execute Session 1 from PLAN.md (all Sub-steps). Create branch feat/transcribe-cpp-migration; confirm
the app builds + dictates and record a baseline. Add transcribe-cpp 0.1.2 (default-features=false;
Windows x86_64 features ["dynamic-backends","vulkan"]) keeping transcribe-rs 0.3.11 untouched. Install
prereqs (cmake, Vulkan SDK 1.4.309.0, SPIRV-Headers via vcpkg + CMAKE_PREFIX_PATH, CARGO_TARGET_DIR=C:\t)
and `cargo build`. Then write a throwaway src-tauri/examples/cpp_spike.rs that init_backends_default(),
prints devices(), loads parakeet-unified-en-0.6b-Q8_0.gguf, runs batch on a 16 kHz mono WAV, and
exercises stream()/feed/finalize — confirm both work, delete the example. This is the make-or-break
build spike: write the EXACT prereqs/env into PLAN.md Evidence. Do NOT wire the engine into app code.
Update PLAN.md.
```

## Session 2 — Batch engine integration (side-by-side)

```
Execute Session 2 from PLAN.md (all Sub-steps). Add EngineType::TranscribeCpp (is_transcription()=true)
and LoadedEngine::TranscribeCpp(...); regenerate bindings.ts; confirm getModelCategory default→"stt".
Implement the load arm (Model::load_with(..)?.session()?), a shared transcribe_cpp_run_plan(settings,
caps)->RunOptions (language if in caps.languages else auto; translate→Task::Translate+target="en" when
caps.supports_translate&&src≠en; timestamps; RunExtension::Whisper{initial_prompt} only when
arch=="whisper"), and the transcribe() batch arm (session.run(&audio,&run_opts)?.text) reusing existing
post-processing + mutex-lease. Add init_logging()+init_backends_default() at startup (lib.rs ~L211) and
extend get_available_accelerators() with transcribe_cpp::devices() (decide device setting; avoid two
dials). Temporarily add one GGUF ModelInfo (parakeet-unified-en Q8_0) and verify a correct batch
transcript through transcribe.cpp while existing models still work; cargo build + bunx tsc --noEmit pass.
Record the LoadedEngine shape + run_plan signature in Downstream Notes. Update PLAN.md.
```

## Session 3 — GGUF catalog, capability probing & model UI

```
Execute Session 3 from PLAN.md (all Sub-steps). (3a) Fetch the live catalog.json from
raw.githubusercontent.com/cjpais/Handy/main/src-tauri/src/catalog/catalog.json; add ModelInfo entries
for the recommended set (PLAN.md §4) as EngineType::TranscribeCpp single-file .gguf downloads from
huggingface.co/handy-computer/<slug>-gguf, marking streaming ones is_recommended, reusing the existing
download/resume/mirror machinery (prefer a bundled catalog.json loader over hardcoding — note the
choice). (3a) Create managers/gguf_meta.rs (dep-free GGUF v2/v3 LE header parser) + managers/
model_capabilities.rs (read stt.capability.streaming/translate/lang_detect etc.), with post-load
reconcile via session.model().capabilities(); mirror Handy's module names. (3b) Model-selector UI: new
models auto-appear in the STT tab; add a Streaming badge, quant, accuracy/speed, recommended ordering;
i18n labels. NOTE: 3a (backend) and 3b (frontend) may be run as two parallel sessions if you prefer.
Verify the new recommended models list with a streaming badge, download+select, and transcribe.
Update PLAN.md.
```

## Session 4 — Real native streaming (the payoff)

```
Execute Session 4 from PLAN.md (all Sub-steps). Add a native streaming worker for
LoadedEngine::TranscribeCpp when caps.supports_streaming: session.stream(&run_opts,&StreamOptions{
commit_policy:Auto,..}) then accumulate the recorder's 30ms frames to ~80ms and stream.feed(&pcm)?;
on committed_changed||tentative_changed emit "stream-text" {committed, tentative}. Finalize →
stream.finalize()? then return stream.text().display() (post-processed like batch); Cancel →
stream.reset(). PRESERVE the engine-lease + cancel_stream() release contract + batch fallback for
non-streaming models. Reuse transcribe_cpp_run_plan (S2). In the overlay, render committed solid +
tentative dimmed (optional StreamPhaseEvent spinner). Retire the VadChunked/EnergyVad isolated-chunk
path for cpp models and delete dead code. Verify: parakeet-unified-en-0.6b + live_transcription_enabled
→ live committed+tentative text matching batch quality; a 10-min session stays stable and keeps pace vs
the S1 baseline; batch models still fall back; cancel + empty-recording release with no leak; build
clean. Record commit policy + feed cadence in Downstream Notes. Update PLAN.md.
```

## Session 5 — Optional live-transcription window

```
Execute Session 5 from PLAN.md (all Sub-steps). Add the boolean setting
live_transcription_window_enabled (default false) via the full 7-point toggle template (PLAN.md §7).
Then, following Handy, reuse the recording overlay and add a "streaming" state resized to 400×120
(add OVERLAY_STREAM_WIDTH/HEIGHT in overlay.rs) showing committed+tentative; keep the compact 128×40
pill otherwise; gate the larger card on the new setting; reuse get_monitor_with_cursor /
calculate_overlay_position and replicate per-OS window treatment (macOS NSPanel, Linux gtk-layer-shell,
Windows HWND_TOPMOST). Verify: toggle ON + a streaming model → readable live card during dictation that
hides after; OFF → only the compact pill; default off; no regression to existing overlay states.
Update PLAN.md.
```

## Session 6 — Recommended defaults, onboarding & legacy (+ optional convergence)

```
Execute Session 6 from PLAN.md (all Sub-steps). Make parakeet-unified-en-0.6b the recommended default
(offer nemotron-3.5-asr-streaming-0.6b for multilingual users), keeping the existing default working if
the new one isn't downloaded (touch onboarding, managers/model.rs is_recommended/default, settings.rs
default_model). Ensure already-downloaded ONNX/whisper models stay listed and usable via transcribe-rs;
never delete user files; label legacy vs new. OPTIONAL (only if the human explicitly opts in): converge
to Handy's shape (a) — downgrade transcribe-rs to 0.3.8 ONNX-only, move whisper to transcribe.cpp, drop
ort-directml. Verify fresh onboarding recommends the streaming model, existing users' models still work,
and (if converged) the app still builds + dictates. Update PLAN.md.
```

## Session 7 — Build, installer, CI, signing & FOLLOW_HANDY.md

```
Execute Session 7 from PLAN.md (all Sub-steps). Finalize per-platform Cargo target tables (Win x86_64
["dynamic-backends","vulkan"]; Win aarch64 default-off static CPU-only with -DGGML_NATIVE=OFF
-DGGML_OPENMP=OFF, Ninja+clang-cl; macOS ["metal"]; Linux ["dynamic-backends","vulkan"]). In build.rs,
stage .dll/.dylib/.so* from DEP_TRANSCRIBE_CPP_RUNTIME_DIR/MODULE_DIR into src-tauri/transcribe-libs/,
bundle beside the exe via tauri.windows.conf.json (+ VC++ redist on Win, $ORIGIN/../lib rpath on Linux).
In CI, add Vulkan SDK + SPIRV-Headers + CARGO_TARGET_DIR shortening; sign every bundled DLL on Windows;
audit all staged DLLs are in MSI+NSIS and speakoflow.exe --list-devices runs. Finally write
docs/engine-migration/FOLLOW_HANDY.md (bump transcribe-cpp → copy Handy's catalog.json → diff+port
Handy's transcription/model_capabilities/gguf_meta modules → re-run the acceptance smoke suite; note the
pre-1.0 ABI pin). Verify a PACKAGED build runs on a clean machine (no dev toolchain / no Vulkan SDK) and
transcribes with a GGUF model (hard gate), a signed installer is produced, and FOLLOW_HANDY.md is
complete. Update PLAN.md.
```
