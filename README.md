# SpeakoFlow

**A free, local-first voice assistant for your desktop — dictation, a floating AI chat panel, screen vision, and spoken answers, all from one global hotkey.**

SpeakoFlow started as a fork of the excellent [Handy](https://github.com/cjpais/Handy) dictation app and is growing into something different: a full Wispr Flow–style voice assistant that you own. Speech-to-text stays 100 % local; the assistant brain is any OpenAI-compatible LLM you point it at — OpenAI, Anthropic, Groq, OpenRouter, Z.AI, Cerebras, AWS Bedrock, Azure OpenAI (via a custom endpoint), an Ollama / LM Studio server, or the **built-in local LLM** that runs fully offline with no server to set up.

## What it does

### 🎙️ Dictation (local, offline)

- Press **Ctrl + Space** and talk — your words are typed into whatever app has focus
- **Hands-free mode**: press **F9** once to start, press again to stop — no holding keys during long dictation
- Whisper / Parakeet models run on your GPU (Vulkan) or CPU; audio never leaves your machine

### 🤖 Assistant panel (the "Ultra" part)

- Press **Ctrl + Alt + Space**, ask a question by voice, and a streaming answer appears in a floating, always-on-top glass panel
- Type into the panel too — it's a full chat with conversation memory
- Markdown answers, one-click copy, draggable anywhere, remembers its position
- **Collapse to pill**: shrink the panel to a tiny floating button bar with a click-to-talk mic

### 👁️ Screen vision

- Press **Ctrl + Alt + Shift + Space** — or just _say_ "what's on my screen?" — and the assistant sees a screenshot of your active monitor
- The camera button in the panel arms a screenshot for your next message (typed _or_ spoken)
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
- Built for speed and small prompts: only short snippets come back (never full pages), a quick local heuristic skips searches for chit-chat or coding, and a slow search degrades gracefully instead of stalling the answer

### 🔊 Spoken answers (TTS)

- The assistant reads its reply aloud — Markdown, code blocks, links, and emoji are stripped first so nothing gets spelled out symbol by symbol
- Keep spoken replies short with the **Response length** setting (Short / Medium / Long), which shapes the answer itself rather than tacking on a separate summary step
- Adjustable playback speed (0.25x–4x) with quick presets
- Four engines:
  - **Kokoro** — free, runs locally in the app via WebGPU; no key, nothing leaves your machine
  - **OpenAI-compatible** — any `/audio/speech` endpoint: OpenAI, Azure OpenAI, Groq, or a local server
  - **ElevenLabs** — bring your API key and voice ID
  - **Azure AI Speech** — neural voices (e.g. `en-US-JennyNeural`) via your Speech resource key

### 🎨 Make it yours

- Six accent colors, three text sizes, three panel sizes, adjustable opacity — with a live preview in settings
- Configurable system prompt and reply-length control (Short / Medium / Long)
- Every hotkey is rebindable

## What's different from Handy?

|                                          | Handy | SpeakoFlow                                             |
| ---------------------------------------- | ----- | ------------------------------------------------------ |
| Local dictation                          | ✅    | ✅ (unchanged core)                                    |
| AI assistant chat panel                  | —     | ✅ floating glass panel, streaming                     |
| Screen vision (screenshots to the model) | —     | ✅ hotkey, voice intent, or camera button              |
| Web search                               | —     | ✅ Serper / SerpAPI / Brave / Tavily / Exa (bring your own key) |
| Spoken answers (TTS)                      | —     | ✅ Kokoro local / OpenAI-compatible / ElevenLabs / Azure |
| Hands-free dictation toggle              | —     | ✅ dedicated F9 binding                                |
| Built-in offline LLM (llama.cpp)         | —     | ✅ runs a downloaded model, no server                  |
| Local LLM preset (Ollama / LM Studio)    | —     | ✅                                                     |
| Panel customization + pill mode          | —     | ✅                                                     |

Everything is wired through the same provider system, so one API key works for assistant answers, dictation post-processing, and remote TTS.

## Default hotkeys

| Action                          | Shortcut                     |
| ------------------------------- | ---------------------------- |
| Dictate (hold)                  | `Ctrl + Space`               |
| Hands-free dictation (toggle)   | `F9`                         |
| Dictate with AI post-processing | `Ctrl + Shift + Space`       |
| Ask the assistant               | `Ctrl + Alt + Space`         |
| Ask about your screen           | `Ctrl + Alt + Shift + Space` |
| Show / hide the panel           | `Ctrl + Alt + A`             |
| Cancel the current recording    | `Esc`                        |

On macOS the modifier defaults to `Option` in place of `Ctrl`/`Alt` (e.g. `Option + Space` to dictate). Every shortcut is rebindable in settings.

## Getting started

1. Install and launch — pick a transcription model when prompted (Parakeet V3 is a great default)
2. Open **Settings → Assistant** and pick a provider:
   - **Built-in (Local)** — fully offline, no key; just download a small LLM from the model picker and you're set
   - **Custom** for Azure OpenAI — base URL `https://{your-resource}.openai.azure.com/openai/v1` (or your `cognitiveservices.azure.com` domain), model = your deployment name
   - **Local** for an Ollama / LM Studio server, or any of the hosted presets (OpenAI, Anthropic, Groq, OpenRouter, Z.AI, Cerebras, AWS Bedrock) with an API key
   - For screen vision, the model must support images (`gpt-4o-mini`, `gpt-4.1-mini`, `gemini-flash`, …)
3. Optionally enable **Web search** in the same settings page (add a free Serper API key)
4. Press `Ctrl + Alt + Space` and ask something

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
- **Fully offline option**: pair the Built-in (Local) LLM with Kokoro TTS and web search off, and nothing leaves your machine at all
- No telemetry, no accounts, no cloud middleman

## Credits & license

Built on [Handy](https://github.com/cjpais/Handy) by CJ Pais and contributors — the local dictation core (Whisper/Parakeet pipeline, VAD, overlay, settings architecture) is their excellent work. MIT licensed, like the original. See [LICENSE](LICENSE).
