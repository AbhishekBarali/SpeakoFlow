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
- **No tool-calling**: search runs _inline before_ the single LLM call, so it works with any small OpenAI-compatible model. A fast local heuristic (`should_search`) decides per-question whether to search at all — factual/time-sensitive questions yes; greetings, code, math, translation, rewriting no — so casual chat stays instant.
- Results are prepended to the request message with a "cite as [1], [2]" directive; stored history keeps only the clean user text (results never burn tokens on later turns, mirroring the screenshot-marker trick).
- **UI**: a globe toggle in the panel input row, a new "searching the web…" status, and a Settings → Assistant → Web Search section (enable, provider, API key for keyed providers, results count, and a "Test search" button).

- Screen vision end-to-end against Azure with the 48 KB budget
- Kokoro vs remote TTS latency comparison

## Phase 1.8 — Secure API-key storage in the OS keychain (done)

API keys now live in the OS credential store instead of in plaintext inside `settings_store.json`. With many providers in play (OpenAI, Anthropic, Groq, OpenRouter, Z.AI, Cerebras, Bedrock, Azure, ElevenLabs, Brave, Firecrawl) this gets every secret off disk while staying invisible to the rest of the app.

- **New `secret_store.rs`** wraps the [`keyring`](https://crates.io/crates/keyring) crate (v3). Service = the app id (`com.abhishekbarali.speakoflow`); accounts are namespaced (`post_process:<provider>`, `web_search:<provider>`, `assistant_tts`). Public surface is just `get` / `set` / `delete` / `sync` / `is_available`.
- **Hot-path safe.** `get_settings()` runs on every action, so the keychain is read at most once per key and then served from a process-wide cache (a missing key is cached too). `get_settings` _hydrates_ the in-memory secret fields from that cache; `write_settings` _syncs_ changed keys into the keychain and then blanks them before the struct is serialized to disk. The four read sites and the three "set key" commands were left **unchanged**.
- **One-time migration.** On first launch, any pre-existing plaintext keys are moved out of the store into the keychain and stripped from the JSON.
- **Cross-platform & build-safe.** Per-target features so each OS pulls only its native backend: macOS `apple-native` (Security framework), Windows `windows-native` (Credential Manager), Linux `async-secret-service` + `tokio` + `crypto-rust`. The Linux path is the **pure-Rust zbus + RustCrypto** stack — verified via `cargo tree` to pull **no libdbus / OpenSSL / C system deps** — so the export/CI build needs no extra system packages. All keychain calls run on a dedicated worker thread (the Linux secret-service backend can deadlock if called on the main/runtime thread, and the thread join also turns any backend panic into a recoverable error).
- **Graceful fallback.** If the platform store is unavailable (e.g. a headless Linux box with no Secret Service), every operation degrades to a no-op and the key stays in the settings file exactly as before, with a single logged warning. The app never blocks or crashes on a missing keychain.

## Phase 1.9 — Web search quality overhaul (done)

Phase 1.7 optimized web search for speed and few tokens: snippet-only results, the raw voice transcript used verbatim as the query, and a keyword heuristic deciding when to search. In practice that produced shallow, often wrong answers. This phase flips the priority to **answer quality** (slower is fine) and is built around the [Firecrawl](https://docs.firecrawl.dev/api-reference/endpoint/search) `/v2/search` endpoint.

- **Full page content, not snippets.** With Firecrawl, search now sends `scrapeOptions: { formats: ["markdown"], onlyMainContent: true }`, so the model reads the actual top-result pages (bounded to ~2k chars each, ~8k total) instead of a one-line preview. This is the single biggest quality lever — Firecrawl's own guidance is that snippets alone aren't enough to ground an answer. `parsers: []` + `proxy: "auto"` keep credit cost down. Brave/DuckDuckGo remain snippet-only fallbacks with larger budgets.
- **LLM search planner replaces the keyword heuristic.** A capable model now _decides_ whether a live search is actually needed and _rewrites_ the request — typically a rough voice transcription — into 1–3 clean, keyword-rich queries (fixing misheard names, dropping filler, resolving follow-up pronouns from recent turns, splitting compound questions) and picks a freshness window (`day`/`week`/`month`/`year` → Firecrawl `tbs`, Brave `freshness`, DDG `df`). Output is constrained by a strict JSON schema when the provider supports structured output, parsed leniently otherwise. This is language-agnostic, unlike the old English-only keyword lists.
- **Robust + graceful.** A cheap local pre-gate (`should_search`) still skips obvious chit-chat/code/math so we don't burn a planner round-trip on them. The built-in local model skips planning (searches the raw question). Any planner error falls back to the raw-question search, and any search failure/timeout answers without web context — the turn never breaks. Every stage races a Stop press. Timeouts were raised (Firecrawl 45 s) since quality now outranks latency.
- **Settings & UI.** New "Read full pages (best quality)" toggle (`assistant_web_search_fetch_content`, on by default); results count raised to 1–10 (default 5); provider copy now recommends Firecrawl for full-content answers. New `set_assistant_web_search_fetch_content` command + regenerated bindings; `SearchResult` gained a `content` field.
- Grounded in research via the Firecrawl MCP (search API shape, `scrapeOptions`/`tbs`/geo params) and query-rewriting best practices (entity extraction, typo fixing, multi-query, gated rewriting; small models suffice for the planner).

## Phase 2.0 — Web search: reliability, answer quality, and a snippet-only provider set (done)

Three problems made web search feel broken even on strong models. Fixed in order:

- **It often didn't search, and when it did, it came back empty.** The LLM planner was over-confident and returned `needs_search=false` for plain factual questions ("who is the prime minister of Nepal") unless the user literally said "search the web". Added a deterministic `looks_time_sensitive` override so role-holder/price/score/weather/recent-year questions always search. Separately, Serper intermittently returns HTTP 200 with an _empty_ result set (verified against the live API: first/uncached hit, or a too-tight `tbs`); `search_serper` now retries once (dropping the time filter) on an empty response.
- **The output quality was awful.** The grounding directive said results were "included with the user's message", so the model kept saying "the results you sent me / your search results" and hedged ("I can only confirm one score") even with several results present. Rewrote it as `web_search_system_directive(tts_enabled)`: frames results as the assistant's _own_ retrieval (never user-provided), enforces a direct BLUF answer, bans asking-to-clarify when results exist, and is **TTS-aware** — speech-friendly prose when the reply is read aloud, compact Markdown (tables/bullets) when it's only on screen. The injected block header was relabeled to match. Retrieval was also widened per depth tier (more snippets reach the model). Research basis: ChatGPT/Gemini/Perplexity teardowns — retrieval quality, not the model, is the bottleneck; answers are built from ranked snippets with the answer up front.
- **Firecrawl removed; more free SERP providers added.** This is a quick search-and-chat assistant, not a research tool, so full-page fetching/scraping was dropped entirely. Removed the Firecrawl provider, the `/v2/scrape` stage, and the Firecrawl-specific credit guard (and its UI: "Read full pages" + "daily credit budget"). Web search is now strictly snippet-first. Provider set is now **Serper** (default), **Brave**, **Tavily** (AI-search, returns a synthesized answer), **Exa** (neural), and **SerpAPI** (Google) — all single-key, all with a free tier, all routed through the same planner → snippet-search → local rerank pipeline. Freshness maps to each API's own param (Serper/SerpAPI `tbs`, Brave `freshness`, Tavily `time_range`, Exa `startPublishedDate`). Legacy `firecrawl`/`duckduckgo` settings migrate to Serper on load. The deprecated `fetch_content`/`daily_credit_budget` settings + commands are kept as no-ops for back-compat/bindings stability.
- **New doc: [`prompts-reference.md`](./prompts-reference.md)** maps every LLM instruction in the app (the system-prompt assembly order, the planner prompt, the grounding/capability directives, response-length directives, the post-process prompt) with file/symbol locations and a "symptom → which prompt" table.

## Phase 2.1 — Assistant characters (personas) (done)

The assistant can now take on a **character**: a named persona with its own prompt, avatar, and greeting. Characters live in their own settings section (`characters` in the sidebar) and are backed by `assistant_characters` in settings.

- **Persona gallery.** Pick an active character; its persona prompt overrides the plain system prompt for LLM turns. Each character has a name, optional avatar image, persona prompt, and greeting.
- **Author however you like.** Create a blank character, **generate one with the LLM** from a one-line description, duplicate an existing one, or import/export as JSON to share. Delete removes it from the gallery.
- **Voice-authored descriptions.** The "generate from a description" flow accepts **in-app dictation** — describe the character out loud and the local STT fills the field, so persona creation stays hands-on-keyboard-optional.
- **Built-in Cat.** A special `cat` character ignores the model entirely and just meows — a zero-cost way to sanity-check the panel without spending a token.

## Phase 2.2 — Desktop shell & settings redesign (done)

A branding + UI wave that reshapes the window shell and the settings surface, plus two input-reliability fixes.

- **Custom title bar.** Native window decorations are dropped on Windows/Linux (`decorations(false)` in `lib.rs`); the webview now draws its own chrome via `TitleBar.tsx` (brand wordmark + minimize/close, doubling as the drag region). macOS keeps the window decorated but uses an **overlay title bar** (`TitleBarStyle::Overlay` + `hidden_title`) so the native traffic lights still work. Needs the `core:window:allow-minimize`/`allow-close` capabilities; close still hides to the tray. The brand moved out of the sidebar (now pure navigation) into the title bar.
- **Warm palette + depth.** The cool neutral-gray theme was reworked into a **warm cream** palette (stepped so bright cards float off the pane), with new soft, warm-tinted elevation utilities (`.elev-card`, `.elev-chip`, `.elev-pane`) and a frosted `.glass-menu` for popovers. Dark mode swaps to deeper, cooler shadows.
- **iOS-style setting rows.** A new tone system (`ui/tones.ts`: `SettingTone`, `TONE_TILE`, `TONE_PILL`) gives each setting row a soft-tinted rounded icon tile, and `SettingsGroup` gained an optional accent-icon header. Applied to the **General** and **Models** sections first; the treatment is opt-in per component so the remaining sections keep the quieter label style until they're migrated.
- **Tap-to-lock hardening.** The assistant tap-to-lock key now **defaults to Shift** instead of Space (the default assistant shortcut already holds Space, and a lock key contained in the record shortcut can't work — the held key would instantly lock the recording). `transcription_coordinator.rs` gained `tap_lock_within_shortcut()`, which skips arming when the lock key is a subset of the active record shortcut (modifier aliases normalized so `alt`/`option` and `cmd`/`super` match).
- **Paste modifier safety net.** Synthetic-paste key combos (`input.rs`) now always release their modifiers — even if an intermediate `enigo` call fails — via the new `input::release_all_modifiers`. This fixes the class of bug where an interrupted paste leaves Ctrl/Shift/Alt/Cmd "pressed" at the OS level (a key appearing stuck down).
- **Screen-vision toggle.** The collapsed assistant pill's camera badge is now a true toggle: hover to reveal it when off (click to arm), and once armed it stays visible in every state as the "capture is on" indicator (click to disarm) — one control for both directions instead of an arm-only button.

## Phase 2.3 — Personas rework, shared LLM client & provider polish (done)

A follow-on wave that reframes characters as task-focused **personas**, factors the LLM transport into one module, and rounds out provider/window behavior.

- **Characters → Personas.** The section is now labeled **Personas** in the UI (the internal `characters` key and locale keys are kept to avoid churn). The playful built-in set (Math Teacher / Short Explainer / Karen / Cat) was replaced with a task-focused lineup — **Concise**, **In-Depth**, **Coding**, **Wordsmith**, **Research** — alongside the default assistant. The `cat` kind stays in the code for back-compat with anyone who already made one, but it's no longer a shipped default.
- **Per-persona role + response length.** Each persona gained an optional one-line **role** (`description`, shown as the card subtitle — purely cosmetic, never sent to the model) and an optional **response_length** override. `AppSettings::effective_response_length()` resolves the active persona's override, falling back to the global `assistant_response_length`, so a "Concise" persona can stay short while an "In-Depth" one runs long, independently.
- **Shared `llm_client.rs`.** The OpenAI-compatible transport used by the assistant, post-processing, and remote TTS is now one module: SSE streaming, tool-calling (the web-search path), structured/JSON-schema output, model listing, provider auth (Anthropic `x-api-key`, Azure `api-key`, OpenRouter `HTTP-Referer`/`X-Title`), Azure base-URL normalization to `/openai/v1` (portal/AI-Foundry/Cognitive-Services domains all rewritten), and **system-prompt folding** for the built-in local engine so Gemma-style templates that reject a `system` role still work. HTTP/1.1 is forced to dodge h2 flow-control truncation on large image payloads. Covered by unit tests (Azure normalization + the system-prompt fold).
- **OpenRouter built-in search.** New `assistant_prefer_provider_web_search` setting (default on): when the active provider has its own web search (OpenRouter's `:online`), the app prefers it over its own snippet search; providers without native search always fall back to the app's search.
- **Window size is remembered.** The main window's logical size is saved on resize/close and restored (clamped to the current monitor) on next launch via `main_window_width`/`main_window_height`. Only the size is persisted, not the position, so the window can't reopen off-screen after a monitor change.
- **TTS failure clarity.** New user-facing errors distinguish an OS **autoplay block** (`tts_blocked` — "click the panel to hear it") from an output-device **playback failure** (`tts_playback`), instead of one generic "voice failed".
- **Local context guidance.** The built-in model's context-window help now recommends a larger window (default 8192, 16384 when RAM allows) and explains the budget — system prompt + history + screenshot (vision can cost 1,000+ tokens) + reply — since screen vision and web search need the headroom.

## Roadmap

- **Phase 2**: SQLite conversation history (rusqlite already bundled), Anthropic/Bedrock `cache_control`, token/cost display
- **Phase 3**: local LLM sidecar (llama.cpp + Qwen3-class small model) for fully offline assistant
- **Phase 4**: deployment to the Zenbook S14 — DirectML/NPU acceleration, fan-silence tuning
