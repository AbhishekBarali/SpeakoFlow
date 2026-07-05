# SpeakoFlow

**A free, local-first voice assistant for your desktop — dictation, a floating AI chat panel, screen vision, spoken answers, and a private on-device memory, all from one global hotkey.**

SpeakoFlow started as a fork of the excellent [Handy](https://github.com/cjpais/Handy) dictation app and is growing into something different: a full Wispr Flow–style voice assistant that you own. Speech-to-text stays 100 % local; the assistant brain is any OpenAI-compatible LLM you point it at — OpenAI, Anthropic, Groq, OpenRouter, Z.AI, Cerebras, AWS Bedrock, Azure OpenAI (via a custom endpoint), an Ollama / LM Studio server, or the **built-in local LLM** that runs fully offline with no server to set up.

## What it does

### 🎙️ Dictation (local, offline)

- Press **Ctrl + Space** and talk — your words are typed into whatever app has focus
- **Hands-free mode**: press **F9** once to start, press again to stop — no holding keys during long dictation
- Whisper / Parakeet models run on your GPU (Vulkan) or CPU; audio never leaves your machine

### 🤖 Assistant panel (the "Ultra" part)

- Press **Ctrl + Alt + Space**, ask a question by voice, and a streaming answer appears in a floating, always-on-top glass panel
- Type into the panel too — it's a full chat with conversation memory
- Attach images or files to a message; every screen capture and picture you send shows as a small inline thumbnail you can click to enlarge, right in the chat and in History
- Markdown answers, one-click copy, draggable anywhere, remembers its position
- **Collapse to pill**: shrink the panel to a tiny floating button bar with a click-to-talk mic

### 👁️ Screen vision

- Press **Ctrl + Alt + Shift + Space** — or just _say_ "what's on my screen?" — and the assistant sees your screen. On a multi-monitor setup it grabs the display your mouse is on, so it captures the screen you're actually working on
- The camera button on the panel (and its collapsed pill) toggles screen vision on or off — one click to arm a capture for your next message (typed _or_ spoken), click again to turn it back off
- **Capture timing** (Settings → Assistant → Screen Vision): for voice questions, grab the screen the moment you start asking (the default — so it reflects what you were looking at when you began) or wait until the message is sent. Typed messages always capture on send
- What you sent shows as a small thumbnail on the message — click to enlarge — so you can always see exactly what the assistant saw
- Captures are adaptively compressed to fit strict provider payload limits (verified against Azure's 128 KiB JSON-string cap)
- One master toggle guarantees nothing is ever captured if you don't want it

### 🌐 Web search (optional)

- Flip on web search and the assistant can answer current, factual questions — prices, weather, news, "who is the prime minister of…" — instead of guessing from stale training data
- A globe toggle lives right in the panel's input row, so you can turn it on or off per question
- Five backends, all snippet-first and each with its own free tier (bring your own API key):
  - **Serper** — fast, cheap Google results with a generous free tier (default)
  - **SerpAPI** — Google results via an alternative SERP API
  - **Brave** — Brave's own independent index
  - **Tavily** — search tuned for AI assistants, returns a short synthesized answer too
  - **Exa** — neural/semantic search
- If your assistant provider is **OpenRouter**, you can instead let it run the search itself (its `:online` mode, billed to your OpenRouter credits) rather than plugging in a separate search key — toggle "Use OpenRouter's built-in search" in the same settings page
- Built for speed and small prompts: the assistant model itself decides when to search and with what query — it calls a `web_search` tool mid-answer only when a question actually needs fresh facts, so casual chatter stays instant. Only short snippets come back (never full pages), and a slow search degrades gracefully instead of stalling the answer

### 🔊 Spoken answers (TTS)

- The assistant reads its reply aloud — Markdown, code blocks, links, and emoji are stripped first so nothing gets spelled out symbol by symbol
- Keep spoken replies short with the **Response length** setting (Short / Medium / Long), which shapes the answer itself rather than tacking on a separate summary step
- Adjustable playback speed (0.25x–4x) with quick presets
- Four engines:
  - **Kokoro** — free, runs locally in the app via WebGPU; no key, nothing leaves your machine
  - **OpenAI-compatible** — any `/audio/speech` endpoint: OpenAI, Azure OpenAI, Groq, or a local server
  - **ElevenLabs** — bring your API key and voice ID
  - **Azure AI Speech** — neural voices (e.g. `en-US-JennyNeural`) via your Speech resource key

### 🎭 Profiles (personas)

- Switch the assistant between task-focused profiles: each one sets the assistant's name, role, instructions, and how long its replies run
- Ships with ready-made built-ins — **Companion** (warm and empathetic, for talking something through), **Quick** (fast, friendly, one or two sentences), and **Unfiltered** (blunt, honest feedback, no sugar-coating) — alongside the default general-purpose assistant
- Each profile carries a one-line **role** (shown on its card) and an optional **response length** (Short / Medium / Long, or inherit your global Assistant setting), so "Quick" stays terse while another runs long — independent of the others
- Edit the name, avatar, instructions, and greeting — or describe a profile in a sentence and let the LLM write it for you (dictate the description by voice, right in the app)
- Duplicate, import/export as JSON to share, or delete — and switch the active profile anytime from the panel header
- Tweaked a built-in and want it back? **Restore default** resets it to the shipped version, and **Restore built-ins** re-adds any you deleted — your custom profiles are left untouched

### 🧠 Personal memory (local, private, off by default)

- Let the assistant remember you between chats: a short always-on **"About You"** summary plus a list of durable **notes** (your preferred tone, the tools you use, projects you're working on)
- **Fully on-device and yours to see** — everything lives in your local settings, and Settings → Memory shows exactly what's stored, note by note, with confidence and whether it was learned or added by you
- **Learns quietly, off the hot path**: it distills durable facts from a conversation only when it ends (you close the panel, clear the chat, or hit "Update memory"), never slowing down a live reply — and it reuses whatever assistant model you already picked, including the fully-offline built-in one
- **You're in control**: add/edit/delete notes yourself, tune how much gets used per reply (Light / Balanced / Detailed), export or import it as JSON, or wipe it entirely
- **Incognito** switch for a private chat that's neither remembered nor personalized from memory
- **Safe by design**: it refuses to store secrets, passwords, or ID/card numbers, ignores instruction-like text (so a saved note can't hijack the assistant), and remembered facts are always advisory — your current message always wins
- Off until you turn it on

### 🎨 Make it yours

- Six accent colors, three text sizes, three panel sizes, adjustable opacity — with a live preview in settings
- Configurable system prompt and reply-length control (Short / Medium / Long)
- Pick your dictation feedback sound — Default, Marimba, Pop, Click, or your own custom start/stop clips — with a preview button that plays it before you commit
- Every hotkey is rebindable

## What's different from Handy?

|                                          | Handy | SpeakoFlow                                                           |
| ---------------------------------------- | ----- | -------------------------------------------------------------------- |
| Local dictation                          | ✅    | ✅ (unchanged core)                                                  |
| AI assistant chat panel                  | —     | ✅ floating glass panel, streaming                                   |
| Screen vision (screen sent to the model) | —     | ✅ hotkey, voice intent, or camera button; multi-monitor; capture timing |
| Inline image thumbnails (chat + history) | —     | ✅ click-to-enlarge, persisted with each message                     |
| Web search                               | —     | ✅ Serper / SerpAPI / Brave / Tavily / Exa (bring your own key)      |
| Spoken answers (TTS)                     | —     | ✅ Kokoro local / OpenAI-compatible / ElevenLabs / Azure             |
| Assistant profiles (task personas)       | —     | ✅ profile gallery, per-profile length, LLM-generated, import/export |
| Personal memory (local "About You" + notes) | —  | ✅ two-tier, offline distillation, incognito, export/import          |
| Hands-free dictation toggle              | —     | ✅ dedicated F9 binding                                              |
| Built-in offline LLM (llama.cpp)         | —     | ✅ runs a downloaded model, no server                                |
| Local LLM preset (Ollama / LM Studio)    | —     | ✅                                                                   |
| Panel customization + pill mode          | —     | ✅                                                                   |

Everything is wired through the same provider system, so one API key works for assistant answers, dictation post-processing, and remote TTS.

## Default hotkeys

| Action                          | Shortcut                     |
| ------------------------------- | ---------------------------- |
| Dictate (hold)                  | `Ctrl + Space`               |
| Hands-free dictation (toggle)   | `F9`                         |
| Dictate with AI post-processing (experimental) | `Ctrl + Shift + Space` |
| Ask the assistant               | `Ctrl + Alt + Space`         |
| Ask about your screen           | `Ctrl + Alt + Shift + Space` |
| Show / hide the panel           | `Ctrl + Alt + A`             |
| Cancel the recording or a streaming reply | `Esc`              |

On macOS the modifier defaults to `Option` in place of `Ctrl`/`Alt` (e.g. `Option + Space` to dictate). Every shortcut is rebindable in settings.

## Getting started

1. Install and launch — a short setup wizard walks you through picking a transcription model (Parakeet V3 is a great default) and, optionally, downloading a local AI model for the assistant. Model downloads auto-retry and resume where they left off if the connection drops, so large files don't have to start over
2. Open **Settings → Assistant** and pick a provider:
   - **Built-in (Local)** — fully offline, no key; just download a small LLM from the model picker and you're set
   - **Custom** for Azure OpenAI — base URL `https://{your-resource}.openai.azure.com/openai/v1` (or your `cognitiveservices.azure.com` domain), model = your deployment name
   - **Local** for an Ollama / LM Studio server, or any of the hosted presets (OpenAI, Anthropic, Groq, OpenRouter, Z.AI, Cerebras, AWS Bedrock) with an API key
   - For screen vision, the model must support images (`gpt-4o-mini`, `gpt-4.1-mini`, `gemini-flash`, …)
3. Optionally enable **Web search** in the same settings page (add a free Serper API key)
4. Optionally turn on **Personal memory** in **Settings → Memory** so the assistant remembers you between chats (off by default)
5. Press `Ctrl + Alt + Space` and ask something

### Building from source

```bash
bun install
bun tauri dev      # development
bun tauri build    # release build + installer
```

See [BUILD.md](BUILD.md) for platform prerequisites (Rust, CMake, Vulkan SDK on Windows/Linux).

## Privacy model

- **Voice → text**: always local, never leaves your machine
- **Assistant questions + optional screenshots**: sent only to the LLM provider _you_ configure — which can be the built-in local model or your own Ollama / LM Studio server
- **Web search** (off by default): when on, only your search query goes to the provider you pick (Serper by default), and just short snippets come back — your conversation is never sent
- **TTS**: local by default (Kokoro); remote only if you choose a remote engine
- **Personal memory** (off by default): stays entirely on your device in local settings — nothing is uploaded. When on, only the small "About You" block and the notes relevant to your message ride along in the prompt to your chosen LLM, just like the rest of the conversation. Incognito skips it, and you can inspect, edit, export, or wipe it anytime
- **Fully offline option**: pair the Built-in (Local) LLM with Kokoro TTS and web search off, and nothing leaves your machine at all
- No telemetry, no accounts, no cloud middleman

## Credits & license

Built on [Handy](https://github.com/cjpais/Handy) by CJ Pais and contributors — the local dictation core (Whisper/Parakeet pipeline, VAD, overlay, settings architecture) is their excellent work. MIT licensed, like the original. See [LICENSE](LICENSE).
