# AGENTS.md

This file provides guidance to AI coding assistants working with code in this repository.

## Development Commands

**Prerequisites:**

- [Rust](https://rustup.rs/) (latest stable)
- [Bun](https://bun.sh/) package manager

**Core Development:**

```bash
# Install dependencies
bun install

# Run in development mode
bun run tauri dev
# If cmake error on macOS:
CMAKE_POLICY_VERSION_MINIMUM=3.5 bun run tauri dev

# Build for production
bun run tauri build

# Frontend only development
bun run dev        # Start Vite dev server
bun run build      # Build frontend (TypeScript + Vite)
bun run preview    # Preview built frontend
```

**Linting and Formatting (run before committing):**

```bash
bun run lint              # ESLint for frontend
bun run lint:fix          # ESLint with auto-fix
bun run format            # Prettier + cargo fmt
bun run format:check      # Check formatting without changes
bun run format:frontend   # Prettier only
bun run format:backend    # cargo fmt only
```

**Model Setup (Required for Development):**

```bash
mkdir -p src-tauri/resources/models
curl -o src-tauri/resources/models/silero_vad_v4.onnx https://blob.handy.computer/silero_vad_v4.onnx
```

For detailed platform-specific build setup, see [BUILD.md](BUILD.md).

## Architecture Overview

SpeakoFlow is a cross-platform desktop voice assistant (dictation, AI chat panel, screen vision, and a local-first personal memory) built with Tauri 2.x (Rust backend + React/TypeScript frontend). It started as a fork of [Handy](https://github.com/cjpais/Handy) by CJ Pais; the local dictation core (Whisper/Parakeet pipeline, VAD, overlay, settings architecture) traces back to that project. See [README.md](README.md#credits--license) for full attribution.

### Backend Structure (src-tauri/src/)

- `lib.rs` - Main entry point, Tauri setup, manager initialization
- `managers/` - Core business logic:
  - `audio.rs` - Audio recording and device management
  - `model.rs` - Model downloading and management: resilient downloads that auto-retry with exponential backoff and resume from a `.partial` file via HTTP `Range` (`attempt_download`, `AttemptOutcome`, `HttpStatusError` sorting transient vs. permanent failures), plus an ordered source list — a reliable mirror first, the canonical Hugging Face URL as fallback (`download_candidates` / `mirror_url_for`; mirrors not yet populated). It also owns **SpeakoFlow Mini** (`SPEAKOFLOW_MINI_MODEL_ID`), our own 0.8B English dictation-cleanup fine-tune. It is a `LlamaCpp` model but not a chat model, and `is_cleanup_specialist` is what encodes that: the predicate matches a normalized model _name_, not just the catalog id, so the same weights are recognized whether they arrived as the bundled download, a `.gguf` the user imported, or a model served from their own Ollama/LM Studio endpoint. Being a specialist changes three things — the cleanup catalog features it, the assistant catalog hides it (it cannot converse), and the request path drops the app's own prompt scaffolding. The derived flag reaches the frontend through `ModelInfo.is_cleanup_specialist`, stamped in `get_available_models` / `get_model_info` rather than at the ~30 construction sites so every insert path is classified by one rule
  - `transcription.rs` - Speech-to-text processing pipeline. Compute-device enumeration is crash-isolated on Linux: `PROBE_DEVICES_OUT_OF_PROCESS` (true only on Linux) sends `cached_gpu_devices` / `cached_transcribe_cpp_devices` through `probe_devices_out_of_process`, which spawns `self --probe-devices` and parses one line of JSON. The vendored, statically linked whisper.cpp/ggml in `transcribe-rs` is built with ggml's default `GGML_NATIVE=ON` (`-march=native`), so loading its Vulkan backend can raise SIGILL when a package runs on a narrower CPU than the one that built it — out of process, that costs a GPU listing instead of the whole app at launch. Windows/macOS keep the in-process call unchanged
  - `history.rs` - Transcription history storage
  - `local_models.rs` - Registering models the user **already has on disk**, so an existing collection or a self-made fine-tune needs no re-download and no hand-copying (issue #18). Two entry points, one destination: pick a file, or link any number of folders (`settings.model_folders`) that are then scanned recursively (`scan_folder`, bounded by `MAX_SCAN_DEPTH` / `MAX_SCAN_ENTRIES`, symlink-loop guarded). The hard part is **classification**, and it is done by content rather than filename, because a bare `.gguf` doesn't say whether it's speech-to-text, a chat model, or a vision projector that isn't a standalone model at all: `classify_model_file` reads `general.architecture` from the GGUF header (via `gguf_meta`) and routes an arch in `model_capabilities::KNOWN_ARCHES` to `TranscribeCpp`, a `clip`/`mmproj` arch to a projector (rejected on its own, auto-paired when it sits beside an LLM), and anything else to `LlamaCpp` — an unknown arch is far more likely to be a new chat model than a broken file. A `.bin` is validated against the ggml magic and treated as Whisper. On the `model.rs` side, `ModelInfo.local_path` / `local_folder` carry the result and every path-resolution site (`get_model_path`, `update_download_status`, `apply_gguf_header_hints`, `resolve_mmproj_path`, `local_llm::gguf_path_for`) reads the user's path instead of `<models_dir>/<filename>`. The standing invariant: **these files are not ours** — registering copies nothing, `delete_model` only unregisters, and a folder-derived entry refuses individual removal (the next scan would re-add it) and points at unlinking the folder instead. Only individually picked files are persisted (`local_models.json`); folder contents are re-derived on every scan so a model added to or deleted from a linked folder appears or disappears on its own
- `audio_toolkit/` - Low-level audio processing:
  - `audio/` - Device enumeration, recording, resampling
  - `vad/` - Voice Activity Detection (Silero VAD)
- `commands/` - Tauri command handlers for frontend communication
- `cli.rs` - CLI argument definitions (clap derive)
- `shortcut/mod.rs` - Global keyboard shortcut handling (two engines: `handy-keys` and the Tauri global-shortcut plugin). Both engines route through the shared `shortcut/handler.rs::handle_shortcut_event`, which strips the `.lock` suffix and resolves the recording mode via `transcription_coordinator::recording_mode`. Every recording shortcut (dictation, dictation + post-processing, and the assistant) follows the single `push_to_talk` setting: on means hold to record, off means tap to start and tap to stop. Each recording shortcut also has an auto-derived Shift variant whose binding id carries a `.lock` suffix; that variant requests the **opposite** mode, so Shift plus your record combo gives you a hands-free recording while `push_to_talk` is on. The mid-recording tap-to-lock gesture was removed in favor of this simpler model (`lock_watch.rs` and `TapToLock.tsx` are leftovers with no live call sites). It also routes the cancel/`Esc` binding to either an active recording or a busy assistant reply
- `settings.rs` - Application settings management (recent additions: `sound_theme`; the experimental AI-Correction fields `post_process_enabled` / `post_process_tone` / `post_process_timeout_secs` / `post_process_unload_timeout` — cleanup keeps its **own** engine-residency policy, separate from the assistant's `local_llm_unload_timeout`; and the `experimental_enabled` gate that hides in-development features). **AI-cleanup prompting is a fixed two-layer hierarchy**: layer 1 is the selected cleanup system prompt (`post_process_selected_prompt_id` → `post_process_prompts`, which decides what gets corrected), layer 2 is the writing style (`post_process_selected_tone_id`, layered on top, which decides how the result reads). A third layer — the app's own final-output contract — is appended last and only for a general-purpose model. The former `post_process_cleanup_strength`, `post_process_fix_misheard`, and `post_process_raw_prompt` settings are **gone**: the first two were extra prose competing with the prompt for a small model's attention, and the third asked the user to know something the app derives (see `ResolvedPostProcessConfig::trained_for_cleanup`). Two bundled prompts ship: the long general-purpose one (`DEFAULT_POST_PROCESS_PROMPT_ID`) and the short one SpeakoFlow Mini was trained on (`SPEAKOFLOW_MINI_PROMPT_ID`); `restore_post_process_prompt` puts either back to its shipped text. `post_process_last_cloud_provider_id` / `assistant_last_cloud_provider_id` persist the cloud provider that the "On my device" switch would otherwise erase — both provider ids are single slots, so this is what makes the device ⇄ cloud switch restore a real choice instead of guessing. `assistant_enabled` (default true) is the master switch for the assistant: off means the panel window is never created at launch (and is destroyed, not hidden, when it is turned off), its two bindings — `assistant::ASSISTANT_BINDINGS`, enforced in `shortcut/handler.rs`, unregistered by `set_assistant_enabled`, skipped at launch by both engines' `init_shortcuts`, and refused for external/CLI triggers in `signal_handle::send_transcription_input` — do nothing, the assistant's llama.cpp process is stopped rather than left to idle out, and Profiles/Memory disappear from Settings. "Generate with Flow" and AI Correction deliberately survive it: they share `assistant_provider_id` / `assistant_models`, so the brain picker stays visible. The built-in provider declares `supports_structured_output: true`: llama.cpp compiles the JSON schema into a decoding grammar, which is what stops small chat models from narrating their plan instead of returning the cleaned transcript (measured on Gemma 4 E2B: 400 tokens of monologue without it, a correct 48-token answer with it) — and it is deliberately **not** sent to a cleanup fine-tune, which was trained to emit bare text and would be forced into JSON by the same grammar
- `managers/local_llm.rs` - Bundled llama.cpp sidecar. **Two engine instances, one per `LlmRole`**: `Assistant` (port 11435, vision projector, full context) and `Cleanup` (port 11436, no projector, context capped at `CLEANUP_CONTEXT_SIZE`, `LLAMA_ARG_THINK_BUDGET=0`). Sharing one engine meant a different model on either side evicted the other on every use. **Both roles take the GPU** (`ALL_GPU_LAYERS`): cleanup used to pass `-ngl 0` for small models because a cold load is ~2x faster on CPU and skips the one-time 13–20s shader compile, but that optimised the wrong half — measured on a 2B Q8_0, generation runs at ~212 tok/s on a dedicated GPU against ~21.8 tok/s on the CPU, and generation is what happens on every dictation, while the load happens once behind `warm_up()`. Device _selection_ is pinned with `--device`, not `--main-gpu`: the latter does not choose where the model runs (measured on a 4070 Ti SUPER + Ryzen iGPU box, `--main-gpu 1` still ran on the dedicated card), so on any machine whose ggml device 0 is the integrated GPU the engine silently ran there at ~7 tok/s. `pinned_device_arg` / `select_engine_device_arg` resolve the flag from the engine's **own** `--list-devices` output (`parse_engine_devices`, cached per run) matched against our probe by adapter description, because the two enumerations are independent and their order need not agree. It only engages on a mixed dedicated + integrated machine (`needs_device_pinning`) — elsewhere the engine's own choice is already right and multi-GPU layer splitting is left intact. `warm_up()` sends a synthetic prefill so page-in and GPU pipeline compilation happen during recording rather than on the user's first cleanup
- `secret_store.rs` - API keys in the OS keychain (`keyring`), hydrated into settings on load
- `assistant.rs` - Assistant turn pipeline (`run_assistant_turn`): LLM chat, screen vision, profiles/personas, per-profile response-length. Runs a bounded tool-calling loop (`llm_client::send_chat_stream_with_tools`, `tool_choice = "auto"`, max ~3 rounds) exposing `assistant_tool_defs()` — the `web_search` and `get_current_datetime` tools — so the model itself decides when to search. Injects the advisory personal-memory block late in the prompt and triggers offline memory distillation when a conversation ends. **Panel window lifecycle is main-thread-only:** `build_assistant_panel` / `present_assistant_panel` are private and every public entry point (`create_assistant_panel`, `show_assistant_panel`, `destroy_assistant_panel`) queues them via `run_on_main_thread`. Shortcut actions run on the keyboard engine's thread, and building a WebView window from there deadlocks the event loop — the whole app then freezes (no overlay, no dictation, undraggable windows). Queuing also keeps a create ordered behind a destroy, so a fast off → on of the master switch cannot leave the app believing in a window that is gone
- `memory.rs` - Local-first personal memory: a two-tier profile (always-on "About You" summary + durable notes) selected by keyword relevance within a per-turn token budget, learned via offline distillation at the end of a conversation, with safety guardrails (no secrets/PII, no instruction-shaped text, advisory-only) plus consolidation/decay/pruning
- `screenshot.rs` - Screen capture for vision: grabs the monitor under the mouse cursor (`capture_screen_data_url_at`, multi-monitor aware), adaptively JPEG-compresses to the provider's payload budget, and builds the small persisted display thumbnails (`data_url_to_thumbnail`)
- `llm_client.rs` - Shared OpenAI-compatible chat client used by the assistant, post-processing, and remote TTS: SSE streaming, tool-calling (the web-search path), structured/JSON-schema output, model listing, provider auth (Anthropic `x-api-key`, Azure `api-key`, OpenRouter `HTTP-Referer`/`X-Title`), Azure base-URL normalization to `/openai/v1`, and system-prompt folding for the built-in local engine (Gemma-style templates that reject a `system` role)
- `tts.rs` - Spoken answers (Kokoro local / OpenAI-compatible / ElevenLabs / Azure AI Speech). Only the `kokoro` engine synthesizes **inside the assistant WebView** (`src/assistant/useKokoroTts.ts` + `kokoro-js`): ~310 MB of fp32 ONNX weights, and on WebGPU a second copy as GPU buffers. Because the panel window is created at launch and only ever hidden, that used to be resident for the app's lifetime — and it loaded even for users on a remote engine. `src/assistant/localTts.ts` now holds the policy shared by the panel and Settings: load only when the selected engine is local (`localTtsActive`), prepare only while the panel is actually on screen, and release the session after an idle window (60 s hidden / 10 min visible) unless speech is in flight (`isSpeechInFlight`)
- `web_search.rs` - Optional web search, fully model-decided: the assistant calls a `web_search` tool and `run_tool_search` turns the tool args into a one-query `SearchPlan` fed to `search_with_plan` (parallel snippet search + local rerank) — there is no separate planner or keyword heuristic. Six snippet-only backends (Serper default, Brave, Tavily, Exa, SerpAPI, TinyFish); a `WEB_SEARCH_CAPABILITY_NOTE` is folded into the system prompt when search is enabled. OpenRouter's server-side `:online` search is the one opt-in exception for OpenRouter users
- `transcription_coordinator.rs` - Single-threaded recording state machine; also owns `recording_mode`, which maps the `push_to_talk` setting plus the `.lock` variant flag onto `Hold` or `Lock`
- `actions.rs` - Post-recording output pipeline: pastes the transcript and runs the optional, experimental **AI Correction** post-processing pass (`post_process_transcription`, `resolve_post_process_provider_and_model`, `prewarm_builtin_llm`) — cleanup plus a tone directive (`PostProcessTone::directive()`) in a single LLM call, wrapped in `tokio::time::timeout(post_process_timeout_secs)` and falling back to the raw transcription on timeout or failure. Cleanup runs on its **own** llama.cpp process (`CleanupLlm`, port 11436) which `prewarm_builtin_llm` starts _and warms with a real prefill_ at recording start; `prewarm_assistant_llm` does the same for the assistant's engine. Output is validated by `validate_cleaned_output` → `sanitize_post_process_output` (strips fences, `<transcript>` tags, leaked `<think>` blocks) plus `is_implausibly_long`, which rejects the classic small-model failure of narrating a plan instead of cleaning the text
- `overlay.rs` - Recording overlay window (platform-specific)
- `audio_feedback.rs` - Feedback-sound playback with selectable themes (`SoundTheme`: Default/Marimba/Pop/Click/Custom, resolved per start/stop from bundled resources), a `SoundType::Lock` cue that is currently unreachable (it belonged to the removed mid-recording lock gesture), and `play_test_sound` for in-settings previews
- `signal_handle.rs` - `send_transcription_input()` reusable function
- `utils.rs` - Platform detection helpers

### Frontend Structure (src/)

The app ships three Vite entry points: the main settings window (`App.tsx`), the
floating assistant panel (`assistant/`), and the recording overlay (`overlay/`).

- `App.tsx` - Main settings window: renders the custom `TitleBar`, the sidebar, and the active section (also drives the onboarding flow)
- `components/` - React UI components:
  - `TitleBar.tsx` - Custom window chrome (brand wordmark + minimize/close). The native chrome is disabled in `lib.rs`, so this bar also acts as the drag region; macOS keeps native traffic lights via an overlay title bar
  - `Sidebar.tsx` - Section navigation rail (`SECTIONS_CONFIG` defines the sections: general, models, advanced, history, post-processing, assistant, characters, memory, debug, about). The `characters` section is labeled "Profiles" in the UI; the internal key stays `characters` so code and locale keys don't churn
  - `settings/` - Settings UI, one folder/section (`general/`, `advanced/`, `history/`, `assistant/`, `models/`, `post-processing/`, `debug/`, `about/`) plus shared row components. The `memory` and `characters`/Profiles sections both live under `assistant/` (`MemorySettings.tsx`, `CharactersSettings.tsx`)
  - `model-selector/` - Model management interface
  - `onboarding/` - First-run setup wizard: a two-step flow (`OnboardingLayout.tsx` chrome + segmented "Step N of total" progress) — Step 1 picks a speech-to-text model (`Onboarding.tsx`), Step 2 optionally downloads a local assistant LLM in the background (`LlmOnboarding.tsx`), pointing the built-in provider at the model only once the file is on disk
  - `ui/` - Shared primitives; `ui/tones.ts` defines the semantic icon-tile / pill color tones (`SettingTone`, `TONE_TILE`, `TONE_PILL`) used by the iOS-style setting rows
  - `footer/`, `icons/` - Footer and icon components
- `assistant/` - Floating always-on-top AI chat panel (own window): streaming chat, screen vision, TTS, collapse-to-pill, and inline image thumbnails (screen captures + attached images) with click-to-enlarge (`AssistantPanel.tsx`, `preview.tsx`)
- `hooks/useSettings.ts` - Settings state management hook
- `stores/settingsStore.ts` - Zustand store for settings
- `bindings.ts` - Auto-generated Tauri type bindings (via tauri-specta)
- `overlay/` - Recording overlay window entry point
- `lib/types.ts` - Shared TypeScript type definitions

### Key Architecture Patterns

**Manager Pattern:** Core functionality organized into managers (Audio, Model, Transcription) initialized at startup and managed via Tauri state.

**Command-Event Architecture:** Frontend → Backend via Tauri commands; Backend → Frontend via events.

**Pipeline Processing:** Audio → VAD → Whisper/Parakeet → Text output → Clipboard/Paste

**State Flow:** Zustand → Tauri Command → Rust State → Persistence (tauri-plugin-store)

**Custom Title Bar:** Native window decorations are disabled on Windows/Linux (`decorations(false)` in `lib.rs`); the webview draws the chrome via `TitleBar.tsx` (brand + minimize/close, which needs the `core:window:allow-minimize`/`allow-close` capabilities). macOS keeps the window decorated with an overlay title bar (`TitleBarStyle::Overlay` + `hidden_title`) so the native traffic lights still work. Close hides to the tray (see `on_window_event`).

**Paste Safety Net:** Synthetic-paste flows (`input.rs`) always release modifiers after a key combo, via `input::release_all_modifiers`, so an interrupted paste can never leave Ctrl/Shift/Alt/Cmd stuck "pressed" at the OS level.

**Personal Memory (local-first):** `memory.rs` maintains a two-tier profile — an always-on "About You" summary plus durable notes selected by keyword relevance within a per-turn character/token budget (the `MemoryDetail` dial: Light/Balanced/Detailed). `build_memory_block` appends an advisory, delimiter-wrapped block late in the prompt in `run_assistant_turn` (kept late so the earlier prefix stays cache-friendly). Learning ("distillation") runs OFF the hot path at the end of a conversation — when the panel is hidden (`hide_assistant_panel`), the conversation is cleared (`assistant_clear_conversation`), or the user clicks "Update memory" (`assistant_distill_memory_now`) — reusing the active assistant provider (which can be the fully-offline built-in engine). A `last_distilled_len` dirty-guard prevents redundant passes. Safety is layered: capture, consolidation, and injection all reject secrets/PII and instruction-shaped text (`is_sensitive`), injected memory never overrides the user's current message, and consolidation dedupes/merges (Jaccard overlap), decays stale low-confidence auto notes (~45 days), and prunes to a hard cap. Off by default; an Incognito toggle skips both use and learning for a conversation. It's all stored on-device in settings and fully user-editable/exportable in Settings → Memory.

**Screen Vision (capture timing + thumbnails):** `screenshot.rs` captures the monitor under the mouse cursor (`capture_screen_data_url_at`), so multi-monitor users get the screen they're actually working on, and adaptively JPEG-compresses to the provider's payload budget (verified against Azure's tight cap). The `VisionCaptureTiming` setting controls _when_ a voice turn grabs the screen: `Immediate` (default) stashes a frame at recording start (in `AssistantAction::start`) so it reflects what the user saw when they began, consumed by `run_voice_turn`; `OnSend` captures after transcription. Typed messages always capture on send. Every message stores small persisted display thumbnails (`ChatMessage.images`, built by `build_message_thumbnails`) shown inline in the panel and history with click-to-enlarge — only the full-resolution frame is sent to the model (once), and only the compact thumbnail persists.

### Technology Stack

**Core Libraries:**

- `whisper-rs` - Local Whisper inference with GPU acceleration
- `cpal` - Cross-platform audio I/O
- `vad-rs` - Voice Activity Detection
- `handy-keys` - Global keyboard shortcuts (supports modifier-only combos like `Ctrl+Super`); Tauri's global-shortcut plugin is the alternative engine, selected via the `keyboard_implementation` setting
- `rdev` - Low-level input access (cursor position / virtual input)
- `rubato` - Audio resampling
- `rodio` - Audio playback for feedback sounds

### Application Flow

1. **Initialization:** App starts minimized to tray, loads settings, initializes managers
2. **Model Setup:** First-run downloads preferred Whisper model (Small/Medium/Turbo/Large)
3. **Recording:** Global shortcut triggers audio recording with VAD filtering
4. **Processing:** Audio sent to Whisper model for transcription
5. **Output:** Text pasted to active application via system clipboard

### Settings System

Settings are stored using Tauri's store plugin with reactive updates:

- Keyboard shortcuts (configurable, supports push-to-talk)
- Audio devices (microphone/output selection)
- Model preferences (Small/Medium/Turbo/Large Whisper variants)
- Audio feedback and translation options

### Single Instance Architecture

The app enforces single instance behavior — launching when already running brings the settings window to front rather than creating a new process. Remote control flags (`--toggle-transcription`, etc.) work by launching a second instance that sends args to the running instance via `tauri_plugin_single_instance`, then exits.

## Internationalization (i18n)

All user-facing strings must use i18next translations. ESLint enforces this (no hardcoded strings in JSX).

**Adding new text:**

1. Add key to `src/i18n/locales/en/translation.json`
2. Use in component: `const { t } = useTranslation(); t('key.path')`

**File structure:**

```
src/i18n/
├── index.ts           # i18n setup
├── languages.ts       # Language metadata
└── locales/
    ├── en/translation.json  # English (source)
    ├── de/, es/, fr/, ja/, ru/, zh/, ...
    └── ...
```

For translation contribution guidelines, see [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md).

## Code Style

**Rust:**

- Run `cargo fmt` and `cargo clippy` before committing
- Handle errors explicitly (avoid unwrap in production)
- Use descriptive names, add doc comments for public APIs

**TypeScript/React:**

- Strict TypeScript, avoid `any` types
- Functional components with hooks
- Tailwind CSS for styling
- Path aliases: `@/` → `./src/`

## CLI Parameters

SpeakoFlow supports command-line parameters on all platforms for integration with scripts, window managers, and autostart configurations.

**Implementation:** `cli.rs` (definitions), `main.rs` (parsing), `lib.rs` (applying), `signal_handle.rs` (shared logic)

| Flag                     | Description                                                |
| ------------------------ | ---------------------------------------------------------- |
| `--toggle-transcription` | Toggle recording on/off on a running instance              |
| `--toggle-post-process`  | Toggle recording with post-processing on/off               |
| `--cancel`               | Cancel the current operation on a running instance         |
| `--start-hidden`         | Launch without showing the main window (tray icon visible) |
| `--no-tray`              | Launch without system tray (closing window quits the app)  |
| `--debug`                | Enable debug mode with verbose (Trace) logging             |

**Key design decisions:**

- CLI flags are runtime-only overrides — they do NOT modify persisted settings
- Remote control flags work via `tauri_plugin_single_instance`: second instance sends args, then exits
- `send_transcription_input()` in `signal_handle.rs` is shared between signal handlers and CLI

## Debug Mode

Access debug features: `Cmd+Shift+D` (macOS) or `Ctrl+Shift+D` (Windows/Linux)

## Platform Notes

- **macOS**: Metal acceleration, accessibility permissions required for keyboard shortcuts
- **Windows**: Vulkan acceleration, code signing
- **Linux**: OpenBLAS + Vulkan, limited Wayland support, overlay uses GTK layer shell (disable with `SPEAKOFLOW_NO_GTK_LAYER_SHELL=1`)

## Troubleshooting

See the [Troubleshooting](README.md#troubleshooting) section in README.md.

## GitHub workflow for AI coding assistants

**MANDATORY. Before opening any PR or issue in this repo: you MUST read the relevant template file and follow it strictly.** That includes sections that look "ceremonial" — checklists, AI Assistance disclosures, "Human Written Description". A generic Summary/Test-plan layout is not acceptable.

- **Opening a PR:** If this repo has a `.github/PULL_REQUEST_TEMPLATE.md`, read it and follow it strictly, including sections that look "ceremonial" (checklists, AI Assistance disclosures, "Human Written Description"). If a section requires a human-written paragraph, leave a clear TODO placeholder and ask the human contributor to fill it in — do not invent their voice.
- **Opening an issue:** If this repo has `.github/ISSUE_TEMPLATE/`, pick the right template rather than a blank issue.
- **Translations:** Follow [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md).
- **Full contributor workflow:** [CONTRIBUTING.md](CONTRIBUTING.md).

**Commits:** Use conventional commit prefixes (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`). Focus the message on _why_, not _what_.
