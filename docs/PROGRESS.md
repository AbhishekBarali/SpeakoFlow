# SpeakoFlow — Development Progress

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

## Phase 1.7 — Web search (done)

Optional, opt-in web search so the assistant can answer current/factual questions ("who is the prime minister of…", prices, weather, news) instead of guessing from stale model knowledge.

- **Three backends**: **DuckDuckGo** (free, no key, default — keyless HTML endpoint parsed server-side), **Firecrawl** (`/v2/search`), and **Brave** (`/res/v1/web/search`). All three return **snippets only** — result pages are never fetched/scraped, so a search is one HTTP round-trip and the model only ever sees short titles + snippets.
- **Built for speed & few tokens**: results, per-snippet length, and request time are all capped (6 s timeout, ~220 chars/snippet, 1–8 results). A failed/slow search degrades gracefully — the turn just answers without web context rather than stalling.
- **No tool-calling**: search runs *inline before* the single LLM call, so it works with any small OpenAI-compatible model. A fast local heuristic (`should_search`) decides per-question whether to search at all — factual/time-sensitive questions yes; greetings, code, math, translation, rewriting no — so casual chat stays instant.
- Results are prepended to the request message with a "cite as [1], [2]" directive; stored history keeps only the clean user text (results never burn tokens on later turns, mirroring the screenshot-marker trick).
- **UI**: a globe toggle in the panel input row, a new "searching the web…" status, and a Settings → Assistant → Web Search section (enable, provider, API key for keyed providers, results count, and a "Test search" button).



- Screen vision end-to-end against Azure with the 48 KB budget
- Kokoro vs remote TTS latency comparison

## Phase 1.8 — Secure API-key storage in the OS keychain (done)

API keys now live in the OS credential store instead of in plaintext inside `settings_store.json`. With many providers in play (OpenAI, Anthropic, Groq, OpenRouter, Z.AI, Cerebras, Bedrock, Azure, ElevenLabs, Brave, Firecrawl) this gets every secret off disk while staying invisible to the rest of the app.

- **New `secret_store.rs`** wraps the [`keyring`](https://crates.io/crates/keyring) crate (v3). Service = the app id (`com.pais.handy`); accounts are namespaced (`post_process:<provider>`, `web_search:<provider>`, `assistant_tts`). Public surface is just `get` / `set` / `delete` / `sync` / `is_available`.
- **Hot-path safe.** `get_settings()` runs on every action, so the keychain is read at most once per key and then served from a process-wide cache (a missing key is cached too). `get_settings` *hydrates* the in-memory secret fields from that cache; `write_settings` *syncs* changed keys into the keychain and then blanks them before the struct is serialized to disk. The four read sites and the three "set key" commands were left **unchanged**.
- **One-time migration.** On first launch, any pre-existing plaintext keys are moved out of the store into the keychain and stripped from the JSON.
- **Cross-platform & build-safe.** Per-target features so each OS pulls only its native backend: macOS `apple-native` (Security framework), Windows `windows-native` (Credential Manager), Linux `async-secret-service` + `tokio` + `crypto-rust`. The Linux path is the **pure-Rust zbus + RustCrypto** stack — verified via `cargo tree` to pull **no libdbus / OpenSSL / C system deps** — so the export/CI build needs no extra system packages. All keychain calls run on a dedicated worker thread (the Linux secret-service backend can deadlock if called on the main/runtime thread, and the thread join also turns any backend panic into a recoverable error).
- **Graceful fallback.** If the platform store is unavailable (e.g. a headless Linux box with no Secret Service), every operation degrades to a no-op and the key stays in the settings file exactly as before, with a single logged warning. The app never blocks or crashes on a missing keychain.

## Roadmap

- **Phase 2**: SQLite conversation history (rusqlite already bundled), Anthropic/Bedrock `cache_control`, token/cost display
- **Phase 3**: local LLM sidecar (llama.cpp + Qwen3-class small model) for fully offline assistant
- **Phase 4**: deployment to the Zenbook S14 — DirectML/NPU acceleration, fan-silence tuning
