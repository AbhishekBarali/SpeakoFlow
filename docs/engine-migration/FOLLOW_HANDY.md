# FOLLOW_HANDY.md — pulling future Handy transcription-engine updates

> **Purpose.** The transcription core (the `transcribe.cpp` / GGUF engine, the model
> catalog, the GGUF capability probes) is deliberately structured so that adopting a
> newer Handy engine drop is a **cheap, mechanical pull** — not a re-design. This is the
> repeatable routine. Do it on a branch, verify with the smoke suite, then merge.
>
> Upstream: **Handy** — `github.com/cjpais/Handy` (`src-tauri/`), engine crate
> **transcribe.cpp** — `github.com/handy-computer/transcribe.cpp` (crates.io
> `transcribe-cpp` / `transcribe-cpp-sys`).

---

## 0. Before you start — know where we DIVERGE from Handy (shape-(b))

SpeakoFlow runs the engine **side-by-side** with the original stack; it did **not** take
Handy's shape-(a) convergence. When you diff Handy, **do not blindly copy these** — they
are intentional differences (see `PLAN.md` §2 and Session 6):

| Concern              | Handy (shape-a)                                                                                                        | SpeakoFlow (shape-b) — keep this                                                                                                                       |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `transcribe-rs`      | `0.3.8`, `["onnx"]` only (whisper removed)                                                                             | **`0.3.11`, `["whisper-cpp","onnx"]`** kept as-is (N2)                                                                                                 |
| Whisper models       | via `transcribe-cpp` (GGUF)                                                                                            | still via `transcribe-rs` **and** available as GGUF                                                                                                    |
| Windows ONNX Runtime | baseline ORT linked **dynamically** (`ORT_LIB_LOCATION` / `ORT_PREFER_DYNAMIC_LINK`), `onnxruntime.dll` staged/bundled | ORT **statically linked** by `transcribe-rs`; `ort-directml` **kept**. No ORT dynamic-link, no `onnxruntime.dll` staging, no `stage_onnxruntime_dll()` |
| macOS/Linux ORT      | ORT dylib/.so bundled via CI `jq` into `frameworks`/`deb.files`                                                        | not needed (static ORT) — those CI steps are **omitted**                                                                                               |

So `src-tauri/build.rs` has **`stage_transcribe_runtime_libs()` + `stage_vc_runtime_dlls()`
only** (no `stage_onnxruntime_dll`), the CI workflow has **no** "Install ONNX Runtime" steps,
and the Windows package audit requires `msvcp140`/`vcruntime140`/`transcribe*`/`ggml*` but
**not** `onnxruntime.dll` or `vcomp140.dll` (we build with `GGML_OPENMP=OFF`).

> **Optional future convergence to shape-(a)** is documented in `PLAN.md` Session 6
> Downstream Notes. If you take it, THEN adopt Handy's ORT-dynamic CI steps + `stage_onnxruntime_dll`.

---

## 1. Pin discipline (pre-1.0 ABI — read first)

`transcribe-cpp` / `transcribe-cpp-sys` are **pre-1.0**: minor bumps (`0.1.x → 0.2.0`) may
break the FFI ABI **and** the safe Rust API. Always pin the **exact** version and bump
deliberately:

```toml
# src-tauri/Cargo.toml — base declaration and EVERY target table use the SAME exact version
transcribe-cpp = { version = "=0.1.2", default-features = false }        # base (backend-less)
# … per-platform feature tables below …
```

- Pin with `=` so a `cargo update` can never silently move the ABI.
- Bump the version in **all** the places it appears (base `[dependencies]` + the four target
  tables — see step 3). They must match.
- After any bump, re-run the **full smoke suite** (step 6) before trusting it.

The per-platform feature matrix is fixed and should not change on a routine pull:

| Target                                     | `transcribe-cpp` features         | Posture                       |
| ------------------------------------------ | --------------------------------- | ----------------------------- |
| `cfg(all(windows, target_arch="x86_64"))`  | `["dynamic-backends","vulkan"]`   | shared libs + Vulkan GPU      |
| `cfg(all(windows, target_arch="aarch64"))` | _(none)_ `default-features=false` | **static CPU-only**           |
| `cfg(target_os="macos")`                   | `["metal"]`                       | static, Metal GPU compiled in |
| `cfg(target_os="linux")`                   | `["dynamic-backends","vulkan"]`   | shared .so + Vulkan GPU       |

---

## 2. Bump the engine crate

1. Pick the new version from crates.io (or the Handy `Cargo.toml`
   `raw.githubusercontent.com/cjpais/Handy/main/src-tauri/Cargo.toml`).
2. Edit `src-tauri/Cargo.toml`: update the `=<version>` in the base `[dependencies]` line **and**
   the three feature-bearing target tables (Windows x86_64, macOS, Linux). Leave the Windows
   aarch64 table backend-less.
3. `cargo update -p transcribe-cpp -p transcribe-cpp-sys` (regenerates `Cargo.lock`).
4. Build once so `transcribe-cpp-sys` recompiles its native tree (see step 5 for the env).
   Watch for **new runtime DLLs/.so's** in the staged set (step 4) and for **API drift**
   (step 3 modules).

> **Do NOT bump `transcribe-rs`** on a routine engine pull — N2. That is a separate,
> deliberate decision (shape-(a) convergence only).

---

## 3. Port the transcription-core modules (the diff-and-merge)

These are the files that trace to Handy's engine. Diff Handy's version against ours and port
**behavioural** changes (new capability keys, new `RunOptions`, new stream policy, catalog
schema bumps). Ours carry SpeakoFlow-specific glue (settings dials, post-processing reuse,
the `LoadedEngine`/`EngineType` enums) — keep that glue, take their engine logic.

| Ours                                           | Handy upstream                                     | What to watch for                                                                                                                                                                            |
| ---------------------------------------------- | -------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src-tauri/src/catalog/catalog.json`           | `src-tauri/src/catalog/catalog.json`               | **verbatim copy** — see step 4                                                                                                                                                               |
| `src-tauri/src/catalog/mod.rs`                 | (loader is SpeakoFlow's; Handy embeds differently) | keep ours; only touch if the catalog **schema** (`catalog_version`) bumps                                                                                                                    |
| `src-tauri/src/managers/gguf_meta.rs`          | `src-tauri/src/managers/gguf_meta.rs`              | new GGUF value types / version support; keep it dep-free                                                                                                                                     |
| `src-tauri/src/managers/model_capabilities.rs` | `src-tauri/src/managers/model_capabilities.rs`     | new `stt.capability.*` keys; the **"never guess"** rule (absent key ⇒ `None`, reconcile at load)                                                                                             |
| `src-tauri/src/managers/transcription.rs`      | `src-tauri/src/managers/transcription.rs`          | new `RunOptions`/`StreamOptions` fields, `CommitPolicy`, family extensions, `Capabilities` fields. Port into `transcribe_cpp_run_plan` / `run_stream_worker`, **not** the transcribe-rs arms |

**Diff commands** (pin Handy to a tag/commit you trust, not always `main`):

```bash
for f in managers/transcription.rs managers/model_capabilities.rs managers/gguf_meta.rs; do
  curl -fsSL "https://raw.githubusercontent.com/cjpais/Handy/main/src-tauri/src/$f" -o "/tmp/handy-$(basename $f)"
  echo "=== $f ==="; diff -u "/tmp/handy-$(basename $f)" "src-tauri/src/$f" || true
done
```

Reconcile carefully:

- **`EngineType`/`LoadedEngine`** are SpeakoFlow enums (Handy's are shaped differently). Keep
  `EngineType::TranscribeCpp` + `LoadedEngine::TranscribeCpp(Session)`.
- **Whisper-arch custom-words caveat** (`PLAN.md` S2 notes): the `apply_custom_words` gate keys on
  `EngineType::Whisper` (transcribe-rs). If a whisper-arch GGUF is added, don't double-apply
  custom words (run-slot initial prompt **and** fuzzy post-correction).

---

## 4. Refresh the model catalog (usually the only content change)

The full Handy catalog ships **verbatim**, embedded via `include_str!`, so a catalog refresh is
literally one file copy:

```bash
curl -fsSL \
  https://raw.githubusercontent.com/cjpais/Handy/main/src-tauri/src/catalog/catalog.json \
  -o src-tauri/src/catalog/catalog.json
```

Then:

- `crate::catalog::catalog()` parses it (cached `OnceLock`; malformed ⇒ empty + warn, never panics).
- `ModelManager::insert_catalog_models` surfaces the `recommended` entries as
  `EngineType::TranscribeCpp` single-file `.gguf` downloads (internal id `"<slug>-gguf"`).
  To surface more of the catalog, drop the `.filter(|m| m.recommended)` there.
- Confirm the recommended default id (`RECOMMENDED_MODEL_ID` in `managers/model.rs`, currently
  `parakeet-unified-en-0.6b-gguf`) still exists in the new catalog; if Handy re-ranks, update it.
- Run the `catalog::tests` (schema/rank sanity) — they'll flag a schema break.

---

## 5. Build environment (native prereqs)

The `transcribe-cpp-sys` native build needs **cmake + a Vulkan SDK** (Windows/Linux) and
`SPIRV-Headers` findable via `CMAKE_PREFIX_PATH`.

**Local Windows x86_64** (as verified this repo, Session 1 Evidence):

- VS 2022 (MSVC + Windows SDK), `cmake`, **Vulkan SDK** (the SDK ships
  `Lib\cmake\SPIRV-Headers\SPIRV-HeadersConfig.cmake`, so no vcpkg needed locally).
- `CMAKE_PREFIX_PATH=<VulkanSDK dir>` and `CARGO_TARGET_DIR=<short path, e.g. C:\t>` (the
  vulkan-shaders build paths overflow `MAX_PATH` under a long repo path).
- A reproducible env batch is at `C:\t\cargo_ct.bat` (vcvars64 + those two env vars + `cargo %*`),
  and `C:\t\cargo_test_ct.bat` additionally puts the built backend-DLL dir on `PATH` so a test
  binary can load `transcribe.dll` + the ggml backends.

**CI** does the same via `.github/workflows/build.yml` — see step 7.

---

## 6. Re-run the acceptance smoke suite (the gate)

Run all of these after any bump/port. On Windows use the `C:\t\cargo_test_ct.bat` env so the
test/exe binaries can load the backend DLLs.

```bash
# 1. Build clean (stages the runtime libs into src-tauri/transcribe-libs/).
cargo build --bin speakoflow
#    → expect "Staged N transcribe-cpp runtime library file(s)".

# 2. Unit + integration tests (no regressions to transcribe-rs path — N1).
cargo test --lib
#    → all pass (baseline at Session 6/7: 144 passed, 2 ignored).

# 3. Frontend typecheck (bindings still line up).
bunx tsc --noEmit

# 4. Batch smoke — real transcribe.cpp load + run on a cached GGUF + 16 kHz WAV.
TRANSCRIBE_CPP_MODEL=<parakeet-unified-en-0.6b-Q8_0.gguf> TRANSCRIBE_CPP_WAV=<jfk.wav> \
  cargo test --lib transcribe_cpp_batch_end_to_end -- --ignored --nocapture
#    → correct JFK transcript via the GPU (e.g. Vulkan0).

# 5. Streaming smoke — native stream feed/finalize/reset.
TRANSCRIBE_CPP_MODEL=… TRANSCRIBE_CPP_WAV=… \
  cargo test --lib transcribe_cpp_stream_end_to_end -- --ignored --nocapture
#    → committed text matches batch; reuse-after-finalize + cancel/reset leak-free.

# 6. Device enumeration (backends load).
./speakoflow --list-devices          # exit 0, lists the GPU + "backend Vulkan available: true"
```

Both `#[ignore]`d smoke tests self-skip if `TRANSCRIBE_CPP_MODEL` / `TRANSCRIBE_CPP_WAV` are
absent, so they're safe to leave in the default `cargo test` run.

---

## 7. Packaging / installer (rarely changes on a pull)

Owned by `src-tauri/build.rs`, `src-tauri/tauri.windows.conf.json`, `.github/workflows/build.yml`,
and `scripts/ci/stage-transcribe-libs.sh`. On a routine engine pull the only thing to re-check is
**the staged DLL/.so set** (a new engine version may add or rename a ggml module):

- **Windows:** `build.rs::stage_transcribe_runtime_libs()` copies every `.dll` from
  `DEP_TRANSCRIBE_CPP_RUNTIME_DIR` + `DEP_TRANSCRIBE_CPP_MODULE_DIR` into `src-tauri/transcribe-libs/`;
  `tauri.windows.conf.json` bundles that dir beside `speakoflow.exe`. It panics if 0 libs are
  found, and the CI audit fails if a staged DLL is missing from the MSI **or** NSIS or if the
  installed `speakoflow.exe --list-devices` doesn't exit 0. No maintenance needed unless the
  audit's name patterns (`transcribe*`, `ggml*`) stop matching.
- **VC++ runtime:** CI sets `SPEAKOFLOW_VC_REDIST_DIRS` (from `vswhere` → the VC CRT redist dir);
  `build.rs::stage_vc_runtime_dlls()` stages `msvcp140`/`vcruntime140` (required) app-local.
- **Linux:** `build.rs` bakes a `$ORIGIN/../lib` rpath; CI pre-builds to locate the install lib
  dir (`TRANSCRIBE_LIBDIR`) and `scripts/ci/stage-transcribe-libs.sh` co-locates
  `libtranscribe.so*` + `libggml*.so*` into the AppImage's `usr/lib`. The Linux audit greps the
  deb/rpm/AppImage for `libtranscribe.so` + a `libggml-cpu*.so` and runs `--list-devices` under
  `xvfb-run`.
- **macOS:** the `metal` build is **static** — nothing to stage or bundle. (This is why there is
  no `tauri.macos.conf.json` / `tauri.linux.conf.json`; upstream Handy has neither. Windows is the
  only platform that bundles libs via a per-platform conf.)
- **Signing:** `tauri.conf.json`'s `bundle.windows.signCommand` (`trusted-signing-cli`) signs
  every bundled `.exe`/`.dll` during packaging; CI installs `trusted-signing-cli` and provides the
  Azure Trusted Signing secrets when `sign-binaries: true` (release.yml).

### Known follow-up (shape-(b) only, pre-existing — not GGUF scope)

Because we keep `ort-directml`, the DirectML EP loads a runtime `DirectML.dll` (present next to the
build artifacts, **not** currently staged/bundled). On a clean machine the legacy transcribe-rs
**ONNX-on-GPU (DirectML)** path would fall back to CPU-ORT. The GGUF path and CPU-ORT path are
unaffected. If you want the DirectML EP to work on clean machines, stage `DirectML.dll` alongside
the transcribe-cpp DLLs (analogous to Handy's `stage_onnxruntime_dll`, keyed off the `ort` build
output dir) and add it to the Windows audit. Tracked in `PLAN.md` Session 7 Downstream Notes.

---

## 8. Definition of done for a pull

- `cargo build` stages the runtime libs; `cargo test --lib` green (no transcribe-rs regressions);
  `bunx tsc --noEmit` clean.
- Batch + streaming smoke tests transcribe correctly on the cached GGUF.
- `speakoflow --list-devices` exits 0 and lists the expected backend.
- If the catalog changed: recommended models still resolve; `catalog::tests` green.
- Exact version pins updated consistently across `Cargo.toml`; `Cargo.lock` committed.
- CI's packaged-build audit (MSI + NSIS contain every staged DLL, installed `--list-devices`
  passes) still green on the release workflow.
