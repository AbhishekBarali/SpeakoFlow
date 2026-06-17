# Handy Ultra

**A free, local-first voice assistant for your desktop — dictation, a floating AI chat panel, screen vision, and spoken answers, all from one global hotkey.**

Handy Ultra started as a fork of the excellent [Handy](https://github.com/cjpais/Handy) dictation app and is growing into something different: a full Wispr Flow–style voice assistant that you own. Speech-to-text stays 100 % local; the assistant brain is any OpenAI-compatible LLM you point it at — Azure OpenAI, OpenAI, Groq, OpenRouter, or a fully local Ollama / LM Studio model.

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

### 🔊 Spoken answers (TTS)

- After each answer, a short spoken summary (1–3 sentences) is generated and read aloud — never the whole wall of text
- Three engines:
  - **Kokoro** — free, runs locally in the app via WebGPU, streams sentence-by-sentence
  - **OpenAI-compatible** — any `/audio/speech` endpoint: OpenAI, Azure OpenAI, Groq, or a local server
  - **ElevenLabs** — bring your API key and voice ID

### 🎨 Make it yours

- Six accent colors, three text sizes, three panel sizes, adjustable opacity — with a live preview in settings
- Configurable system prompt and TTS summary prompt
- Every hotkey is rebindable

## What's different from Handy?

|                                          | Handy | Handy Ultra                                      |
| ---------------------------------------- | ----- | ------------------------------------------------ |
| Local dictation                          | ✅    | ✅ (unchanged core)                              |
| AI assistant chat panel                  | —     | ✅ floating glass panel, streaming               |
| Screen vision (screenshots to the model) | —     | ✅ hotkey, voice intent, or camera button        |
| Spoken answer summaries                  | —     | ✅ Kokoro local / OpenAI-compatible / ElevenLabs |
| Hands-free dictation toggle              | —     | ✅ dedicated F9 binding                          |
| Local LLM preset (Ollama / LM Studio)    | —     | ✅                                               |
| Panel customization + pill mode          | —     | ✅                                               |

Everything is wired through the same provider system, so one API key works for assistant answers, post-processing, and TTS summaries.

## Default hotkeys

| Action                        | Shortcut                     |
| ----------------------------- | ---------------------------- |
| Dictate (hold)                | `Ctrl + Space`               |
| Hands-free dictation (toggle) | `F9`                         |
| Ask the assistant             | `Ctrl + Alt + Space`         |
| Ask about your screen         | `Ctrl + Alt + Shift + Space` |
| Show / hide the panel         | `Ctrl + Alt + A`             |

## Getting started

1. Install and launch — pick a transcription model when prompted (Parakeet V3 is a great default)
2. Open **Settings → Assistant**:
   - Pick a provider (e.g. _Custom_ for Azure OpenAI, _Local_ for Ollama)
   - Base URL for Azure: `https://{your-resource}.openai.azure.com/openai/v1` (or your `cognitiveservices.azure.com` domain) — model = your deployment name
   - For screen vision, the model must support images (`gpt-4o-mini`, `gpt-4.1-mini`, `gemini-flash`, …)
3. Press `Ctrl + Alt + Space` and ask something

### Building from source

```bash
bun install
bun tauri dev      # development
bun tauri build    # release build + installer
```

See [BUILD.md](BUILD.md) for platform prerequisites (Rust, CMake, Vulkan SDK on Windows/Linux).

## Privacy model

- **Voice → text**: always local, never leaves your machine
- **Assistant questions + optional screenshots**: sent only to the LLM provider _you_ configure (which can itself be local)
- **TTS**: local by default (Kokoro); remote only if you choose a remote engine
- No telemetry, no accounts, no cloud middleman

## Credits & license

Built on [Handy](https://github.com/cjpais/Handy) by CJ Pais and contributors — the local dictation core (Whisper/Parakeet pipeline, VAD, overlay, settings architecture) is their excellent work. MIT licensed, like the original. See [LICENSE](LICENSE).
