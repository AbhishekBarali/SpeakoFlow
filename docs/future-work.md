# Future work / deferred improvements

This file tracks known-good improvements that are intentionally **deferred** so
we can ship the crash/memory/UX fixes first without a large architectural
change. Nothing here is a blocker for the current fixes; each item is safe to
pick up independently later.

Context: a set of stability fixes landed to stop the "memory climbs → disk hits
100% → PC freezes/shuts down" failure, the "can't stop the assistant while it's
transcribing / loading a reply" problem, and the TTS "download again every time"
annoyance. See the commit history / PR for those. The items below are what we
consciously chose **not** to do yet.

---

## 1. Move local TTS (Kokoro) from the WebView to the Rust backend

**Status:** deferred. We keep the current in-WebView Kokoro (`kokoro-js`) for now
because it works cross-platform with zero extra build/runtime setup and is fast
enough on machines with working WebGPU.

**Interim fix applied (Windows):** every window is now created with
`--enable-unsafe-webgpu` (and `--autoplay-policy=no-user-gesture-required`) via
`WEBVIEW2_BROWSER_ARGS` in `lib.rs`, so on Windows with a WebGPU-capable
GPU/driver kokoro-js runs fp32 on the GPU instead of the robotic wasm/q8
fallback, and spoken replies aren't blocked by autoplay. This does NOT help
macOS/Linux (their webviews can't enable WebGPU flags) — those still use the
wasm fallback, which is the main remaining reason to do the backend migration.

### Why the current approach is the weak link

Local TTS currently runs the Kokoro-82M model **inside the assistant panel's
WebView** via [`kokoro-js`](https://www.npmjs.com/package/kokoro-js)
(onnxruntime-web, WebGPU with a wasm fallback). See
`src/assistant/useKokoroTts.ts`. The backend `src-tauri/src/tts.rs` only handles
**network** engines (OpenAI-compatible, ElevenLabs, Azure, and local HTTP
servers such as Kokoro-FastAPI / openai-edge-tts). Its module header notes: the
"kokoro" engine runs fully locally in the panel webview and never reaches that
module.

This causes several problems at once:

- **Robotic audio.** Tauri WebViews frequently lack working WebGPU (Windows
  WebView2 is flaky; macOS WKWebView / Linux WebKitGTK generally have none), so
  `useKokoroTts.ts` silently falls back to `device: "wasm"` and downgrades
  precision to `q8`. wasm + q8 is slower and lower quality — the "robotic" sound.
- **Sometimes no audio at all.** First use must download the model from
  HuggingFace (fails offline / behind a proxy), and browser autoplay can be
  blocked (`NotAllowedError`) until a user gesture in the panel window.
- **Contributes to the WebView memory leak.** The onnxruntime session and its
  WebGPU buffers (hundreds of MB) are not reliably released. (A best-effort
  dispose/blob-revoke fix was applied, but the underlying wry `evaluate_script`
  leak — see item 6 — still makes the WebView the worst place to hold a large
  model.)
- **Confusing download state.** Kokoro is downloaded/cached by the browser
  (transformers.js cache), which is a completely separate system from the
  disk-backed model store (`managers/model.rs`) used by STT/LLM models. The two
  never reconcile.

### Target architecture

Run Kokoro natively in Rust, next to how STT/VAD already work, and play through
the existing `rodio` pipeline in `tts.rs`.

We already ship an ONNX runtime in the backend: `transcribe-rs` is built with the
`onnx` feature and `vad-rs` (Silero) is ONNX-based, and `tts.rs` already plays
audio with `rodio`. So a native Kokoro engine is consistent with the existing
stack.

Options, roughly by effort:

1. **Native backend Kokoro (recommended).** Load Kokoro-82M ONNX in Rust via the
   `ort` crate (ONNX Runtime) with a GPU execution provider where available
   (DirectML on Windows, CoreML on macOS, CUDA on Linux) and CPU otherwise;
   synthesize to PCM and play via `rodio`. Fixes robotic + snappiness + autoplay,
   unifies the download with `model.rs`, and removes a WebView leak source. This
   also needs a phonemizer (Kokoro expects phonemes); evaluate `espeak-ng`
   bindings or a bundled G2P.
2. **Local HTTP server (already supported).** `tts.rs` already speaks to
   Kokoro-FastAPI / openai-edge-tts. Zero new Rust, but the user must run a
   separate server — a power-user option, not a good default.
3. **OS-native TTS as an instant fallback.** Windows SAPI /
   `System.Speech`, macOS `AVSpeechSynthesizer`, Linux `speech-dispatcher`.
   Instant, no download, no leak, but more robotic than Kokoro. Good as the
   default fallback when there is no GPU/model.

### Suggested path

Short term: add OS-native TTS (option 3) as a "just works" fallback. Medium term:
build the native backend Kokoro (option 1) and retire the WebView path. Until
then the WebView Kokoro remains the default; keep the leak mitigations in
`useKokoroTts.ts`.

---

## 2. Engine child crash-safety on macOS / Linux

**Status:** partially covered. The Windows orphan problem is fixed with a
kill-on-close **Job Object** (`managers/local_llm.rs`) plus graceful
`RunEvent::Exit` cleanup in `lib.rs` (all platforms).

On **Linux**, add crash-safety parity by setting `PR_SET_PDEATHSIG` (e.g.
`SIGKILL`) on the child via `CommandExt::pre_exec` so the engine dies if the
parent dies unexpectedly. On **macOS** there is no direct equivalent; the
`RunEvent::Exit` handler covers graceful quit, and a hard kill is rarer, but a
watchdog or a `kqueue`-based parent-death check could close the gap.

## 3. Startup reaping of a stale engine

**Status:** deferred. Users upgrading from a build that pre-dates the Job Object
fix may have one leftover `llama-server.exe` from a previous session. Consider a
conservative, one-time startup check that reaps only an engine we can positively
identify as ours (e.g. bound to our loopback port `11435` and matching our
binary path) — being careful never to kill a user's own Ollama/llama.cpp.

## 4. Remote PCM TTS sample-rate handling (`tts.rs`)

**Status:** deferred (only affects OpenAI-compatible endpoints that return raw
PCM without a `rate=` in the Content-Type). Today the code assumes
`PCM_DEFAULT_SAMPLE_RATE = 24_000`, so a provider returning 16k/48k PCM plays at
the wrong speed (chipmunk/robotic). Make the assumed rate configurable per TTS
provider, or detect it, instead of hard-defaulting to 24 kHz.

## 5. Unify the two model-download surfaces

**Status:** deferred. STT/LLM models are disk-backed (`managers/model.rs`,
reflected by `is_downloaded`), while Kokoro TTS is browser-cached by
transformers.js. The Models tab and the assistant TTS settings therefore can
disagree about whether "the TTS model" is downloaded. A robust readiness check
against the browser cache was added as a stopgap; fully unifying these (ideally
by moving Kokoro to the backend per item 1) is the real fix.

## 6. WebView IPC volume / the upstream wry `evaluate_script` leak

**Status:** mitigated, not eliminated. The root cause is upstream
([tauri-apps/wry#1489](https://github.com/tauri-apps/wry/issues/1489); WebView2
equivalent [tauri-apps/tauri#12724](https://github.com/tauri-apps/tauri/issues/12724)):
`evaluate_script` leaks a little memory per call and never releases it. We
reduced our call volume (throttling `mic-level` to ~30 FPS — the Handy #1279
fix — and coalescing per-token `assistant-token` emits). Revisit if/when the
upstream leak is fixed, and audit for any other high-frequency emitters.

## 7. Assistant conversation persistence cost

**Status:** deferred. `managers/history.rs` re-serializes and rewrites the
**entire** conversation (including base64 image thumbnails) to SQLite on every
turn — O(n²) writes over a long image-heavy chat, adding disk I/O. Consider
append-only message writes or debounced/dirty-only persistence. Also consider
trimming the in-memory `AssistantConversation.messages` for very long sessions.

## 8. Remaining cancellation edge cases

**Status:** partially fixed. The reply/streaming phase is cancellable
(`tokio::select!` vs the cancel `Notify`), the built-in model-load phase is now
cancellable too (`run_assistant_turn` races `ensure_running` against a Stop),
and a hung stream now self-cancels via the SSE stall timeout. The frontend also
now exposes a real cancel (✗) during listening/transcribing wired to
`cancel_current_operation`.

Two narrow gaps remain, deliberately deferred to avoid regressions in the
recording pipeline:

- **STT-window race.** `run_assistant_turn` calls `begin_turn()` which clears
  the sticky `cancelled` flag (so a _stale_ Stop from a previous turn can't
  suppress a new one). A Stop pressed during the brief batch-transcription
  window (after the hotkey is released, before the reply starts) sets the flag,
  which `begin_turn()` then clears — so that specific Stop can be lost and the
  reply proceeds. Cancelling during _recording_ (the common case) already works
  because it aborts the recording before STT. A clean fix is to clear the flag
  at voice-capture start (recording start) instead of at turn start, then bail
  in `run_voice_turn` if cancelled — but that touches the shared recording path
  and needs careful testing.
- **Batch STT is not abortable mid-inference.** `tm.transcribe()` runs to
  completion; `tm.cancel_stream()` only affects the live/streaming worker. A
  Stop during a long Whisper batch pass won't interrupt the pass itself (it just
  prevents the subsequent reply). Making batch inference cancellable would need
  support in the transcription engine.
