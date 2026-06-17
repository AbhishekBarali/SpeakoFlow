# Handy Ultra — Development Progress

A running log of everything built on top of the original [Handy](https://github.com/cjpais/Handy) dictation core. Newest at the bottom.

## Project goal

Replace paid voice-assistant subscriptions (Wispr Flow–style) with a free, local-first app you own:

- **Dictation** — always local STT (Parakeet/Whisper), text typed into any app
- **Assistant** — speak a question, get a streaming LLM answer in a floating panel
- **Screen vision** — let the model see your screen when you ask about it
- **Spoken answers** — short TTS summaries, never the whole wall of text
- Works with Azure OpenAI / OpenAI / Groq / OpenRouter / local Ollama & LM Studio

Dev machine: Windows 11 desktop (Ryzen 7 7700X, RTX 4070 Ti SUPER). Final target: fan-silent operation on an ASUS Zenbook S14 (Lunar Lake, Arc iGPU + NPU).

---

## Phase 0 — Toolchain & build (done)

- Forked Handy at upstream `cfab1dd`; verified all integration points against the codebase map
- Fixed three Windows build blockers:
  1. bindgen needs `LIBCLANG_PATH` → use VS 2022's bundled LLVM
  2. MSBuild FileTracker fails on >260-char paths (`FTK1011`) regardless of `LongPathsEnabled` → cargo `target-dir` redirected to a short path
  3. `ort-sys` prebuilt DirectML binaries need MSVC 14.44+ STL → VS 17.14 update + stale whisper-rs-sys CMake cache cleared

## Phase 1 — Assistant mode (done)

- New global hotkeys: **Ctrl+Alt+Space** (ask by voice) and **Ctrl+Alt+A** (show/hide panel), fully rebindable with settings migration
- `AssistantAction` pipeline: record → VAD → local STT → LLM → stream into panel (never pastes)
- Always-on-top frameless glass panel (third Vite entry): streaming chat, status states (listening/transcribing/thinking), copy button, text input fallback, draggable, position persisted
- SSE streaming client (`send_chat_stream`) for OpenAI-compatible `chat/completions`
- Cache-friendly request design: byte-identical system prompt → append-only history → newest message last (maximizes provider prompt-cache hits)
- Assistant settings page: provider picker (shared `PostProcessProvider` system), base URL, API key, model, system prompt
- Azure OpenAI verified on the v1 endpoint (`https://{resource}.openai.azure.com/openai/v1`, Bearer auth, model = deployment name)

## Phase 1.5 — Reliability & UX wave (done)

**Duplicate messages eliminated**

- Backend re-entrancy guard rejects concurrent turns (double-fired hotkeys)
- Panel renders exclusively from full conversation snapshots → duplicate events can never duplicate messages
- Enter key-repeat guarded

**Screen vision**

- **Ctrl+Alt+Shift+Space** captures the monitor under the cursor and sends it as a multimodal `image_url` part
- Saying "what's on my screen", "look at this error", etc. auto-attaches a capture on the _normal_ hotkey
- Camera button in the panel arms a one-shot screenshot for the next message — typed _or_ spoken
- Master privacy toggle; capture failure aborts the turn with a visible error instead of silently sending text-only

**The Azure payload saga** (worth documenting — three rounds of 400 errors)

- Symptom: `400 Bad Request — Unterminated string … image_url.url` at inconsistent byte positions (~140–416 KB)
- Round 1: capped raw JPEG at 110 KB → still failed (base64 inflates ×4/3, so 147 KB hit the wire)
- Round 2: budgeted the _encoded_ size ≤96 KB → still failed at smaller positions
- Final: aggressive ladder (1280px/q52 → 640px/q32) targeting **≤48 KB encoded** (real-screen test: 37 KB), HTTP/1.1 forced, history capped (4 msgs / 6 K chars on screenshot turns). Vision models downscale to 512–768px tiles internally, so the lost resolution costs little
- A unit test asserts the size ceiling on a real capture

**Text-to-speech (three engines)**

- **Kokoro** — local & free, runs in the panel webview via WebGPU (fp32) with wasm/q8 fallback; sentence-by-sentence streaming with a gapless audio queue; model download progress shown live
- **OpenAI-compatible** — any `/audio/speech` endpoint (OpenAI, Azure OpenAI, Groq, local servers); fetched and played natively in Rust so it works with the panel hidden
- **ElevenLabs** — API key + voice ID
- Instead of reading the whole answer, a separate cheap LLM request produces a 1–3 sentence spoken recap (prompt configurable)

**Panel polish**

- Markdown rendering in answers (react-markdown)
- Size presets (Compact / Standard / Large) + collapse-to-**pill** mode: a tiny floating bar with a click-to-talk mic
- Six accent colors, three text sizes, opacity slider — live preview in settings
- Transparent window with CSS-drawn rounded card + shadow

**Dictation improvements**

- **Hands-free dictation**: dedicated **F9** binding — press once to start, press again to stop. (Replaced a tap-duration heuristic that proved confusing; explicit beats clever)
- Overlay shows "Recording — press hotkey to stop" while engaged

**Provider lineup**

- Added **Local (Ollama / LM Studio)** preset with editable base URL alongside OpenAI / Azure / Groq / OpenRouter / Anthropic / custom

## Currently testing

- Screen vision end-to-end against Azure with the 48 KB budget
- Kokoro vs remote TTS latency comparison

## Roadmap

- **Phase 2**: SQLite conversation history (rusqlite already bundled), Anthropic/Bedrock `cache_control`, token/cost display
- **Phase 3**: local LLM sidecar (llama.cpp + Qwen3-class small model) for fully offline assistant
- **Phase 4**: deployment to the Zenbook S14 — DirectML/NPU acceleration, fan-silence tuning
