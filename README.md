<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="Logo/final/lockup-dark.svg" />
  <img src="Logo/final/lockup.svg" alt="SpeakoFlow" width="360" />
</picture>

### You think faster than you type.

**A free, local‑first voice assistant for your desktop — dictation, writing, and a real AI assistant, all by voice, all from one hotkey.**

[![License: MIT](https://img.shields.io/badge/License-MIT-2ea44f.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-informational)](#-getting-started)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![React](https://img.shields.io/badge/React-20232A?logo=react&logoColor=61DAFB)](https://react.dev)
[![Price](https://img.shields.io/badge/price-%240%20forever-2ea44f)](#-why-speakoflow)
<br/>
[![GitHub release](https://img.shields.io/github/v/release/AbhishekBarali/SpeakoFlow?include_prereleases&sort=semver)](https://github.com/AbhishekBarali/SpeakoFlow/releases)

[**⬇️ Download**](https://github.com/AbhishekBarali/SpeakoFlow/releases) &nbsp;·&nbsp; [**🌐 Website**](https://www.speakoflow.com) &nbsp;·&nbsp; [**💬 Discussions**](https://github.com/AbhishekBarali/SpeakoFlow/discussions) &nbsp;·&nbsp; [**🐛 Issues**](https://github.com/AbhishekBarali/SpeakoFlow/issues)

<!-- TODO: add a real-time transcription demo GIF here (e.g. demo-assets/hero.gif) and reference it with: <img src="demo-assets/hero.gif" alt="SpeakoFlow live dictation demo" width="720" /> -->

</div>

---

## Table of contents

- [What is SpeakoFlow?](#what-is-speakoflow)
- [✨ Why SpeakoFlow](#-why-speakoflow)
- [🚀 Features](#-features)
  - [🎙️ Dictation (local & offline)](#️-dictation-local--offline)
  - [⚡ Generate with Flow — "Hey Flow"](#-generate-with-flow--hey-flow)
  - [🌍 Translate as you speak](#-translate-as-you-speak)
  - [🧹 AI cleanup](#-ai-cleanup)
  - [🤖 The assistant panel](#-the-assistant-panel)
  - [👁️ Screen vision](#️-screen-vision)
  - [🌐 Web search](#-web-search-optional)
  - [🔊 Spoken answers (TTS)](#-spoken-answers-tts)
  - [🎭 Profiles (personas)](#-profiles-personas)
  - [🧠 Personal memory](#-personal-memory-local-private-off-by-default)
  - [🎨 Make it yours](#-make-it-yours)
- [🆚 How it compares](#-how-it-compares)
- [⌨️ Default hotkeys](#️-default-hotkeys)
- [📦 Getting started](#-getting-started)
- [🛠️ Building from source](#️-building-from-source)
- [🧱 Tech stack](#-tech-stack)
- [🔒 Privacy model](#-privacy-model)
- [🗺️ Roadmap](#️-roadmap)
- [🤝 Contributing](#-contributing)
- [📄 License](#-license)
- [🙏 Credits & acknowledgements](#-credits--acknowledgements)
- [👤 Author](#-author)

---

## What is SpeakoFlow?

SpeakoFlow turns your voice into finished text, right where you're working — email, editor, chat, anywhere. Press a key and talk, and your words are typed into whatever app has focus. Say **"Hey Flow"** first and it goes further: it drafts the reply, writes the email, or answers the question and pastes the result for you. And when you need a back‑and‑forth, a floating AI assistant is one hotkey away — it can talk back, read your screen, search the web, and remember how you like to work.

The important part: **your voice never leaves your machine.** Speech‑to‑text runs 100% locally on your GPU or CPU. The assistant "brain" is any model *you* choose — a fully offline built‑in LLM, your own Ollama / LM Studio server, or any cloud provider with your own key.

It's free, open source (MIT), and yours to inspect.

> SpeakoFlow began as a fork of the excellent [Handy](https://github.com/cjpais/Handy) dictation app and grew into a full Wispr Flow–style voice assistant that you own. See [Credits](#-credits--acknowledgements).

---

## ✨ Why SpeakoFlow

|  | SpeakoFlow |
| --- | --- |
| 💸 **Price** | **$0** — free forever, no account, no catch |
| 🔓 **Source** | Open source (MIT), fully inspectable |
| 🔐 **Privacy** | Transcription is 100% on‑device; nothing is uploaded unless *you* pick a cloud model |
| ✍️ **Does more than dictation** | Generate with Flow, AI cleanup, a chat assistant that talks back, screen vision |
| 🧩 **Your stack** | Built‑in offline model, your own server, or any provider you choose |
| 🖥️ **Cross‑platform** | Windows, macOS, and Linux |

The popular voice tools are closed and paid — Wispr Flow (~$15/mo), Aqua Voice (~$8/mo), Typeless (~$30/mo). SpeakoFlow does more, costs nothing, and its source is yours. *(Competitor pricing checked July 2026; plans and limits can change.)*

---

## 🚀 Features

### 🎙️ Dictation (local & offline)

- Press the dictation hotkey and talk — your words are typed straight into any app.
- Watch them **land live as you speak**, or all at once when you stop.
- **Hands‑free mode:** tap the lock key (Space) while holding to keep recording without holding the keys — great for long dictation.
- Whisper / Parakeet models run on your **GPU (Vulkan/Metal) or CPU**. Audio never leaves your machine.
- ~150+ WPM speaking vs. ~45 WPM typing — roughly **3× faster**, hands‑free.

### ⚡ Generate with Flow — "Hey Flow"

Start a normal dictation with the phrase **"Hey Flow"**, then just ask in your own words: *reply to this message, write an email to Priya, draft a prompt.* Flow does the work and drops the finished result where your cursor is — no panel, no copy‑paste.

- **Not an always‑listening wake word.** Nothing triggers until you deliberately start dictating.
- **Rename the trigger** to anything you like.
- **Stateless & private by design:** one request at a time, no chat history or memory involved.
- Reuses the same model you picked for the assistant — including the fully offline built‑in one.
- Optionally let it glance at your screen for context, only when a command needs it.

### 🌍 Translate as you speak

Speak in whatever language comes naturally — Spanish, Hindi, French, Japanese — and it lands in **clean English** right where you're typing. Runs fully on your device.

> **Requires a Whisper model.** Translation is a Whisper speech‑to‑text capability. Parakeet is English‑only, so pick a Whisper model in **Settings → Models** to translate as you speak.

### 🧹 AI cleanup

Say it with all the "um"s and false starts. AI cleanup strips the filler, fixes grammar, and matches the **tone you pick** — Professional, Friendly, Concise, Formal, or your own — so it reads the way you meant it. Optional, and runs only when you turn it on (its own hotkey, or as an always‑on post‑processing pass).

### 🤖 The assistant panel

- Ask by voice or text and get a **streaming answer** in a floating, always‑on‑top glass panel.
- Full chat with conversation memory, Markdown answers, one‑click copy, draggable anywhere (it remembers its position).
- **Collapse to a pill:** shrink the panel to a tiny floating mic bar for click‑to‑talk.
- Attach images or files; every screen capture and picture shows as an inline thumbnail you can click to enlarge — in the chat and in History.

### 👁️ Screen vision

- Ask about the error, chart, or email in front of you — just *say* "what's on my screen?" or arm the camera button, and the assistant answers with that context.
- On a **multi‑monitor** setup it captures the display your mouse is on.
- **You decide when it looks:** tell it to look, let it decide only when a question needs it, or tell it to skip entirely (nothing captured).
- **Capture timing** is configurable — grab the screen the moment you start asking (default) or when the message is sent.
- Captures are adaptively compressed to fit strict provider limits. One master toggle guarantees nothing is ever captured if you don't want it.

### 🌐 Web search (optional)

- Flip on web search and the assistant can answer current, factual questions — prices, weather, news — instead of guessing from stale training data.
- A globe toggle lives in the panel's input row, so you can turn it on or off per question.
- The **model itself decides** when to search and with what query, mid‑answer, so casual chatter stays instant. Only short snippets come back (never full pages), and a slow search degrades gracefully instead of stalling the reply.
- Five backends, each with a free tier (bring your own key): **Serper** (default), **SerpAPI**, **Brave**, **Tavily**, **Exa**. OpenRouter users can instead use its built‑in `:online` search.

### 🔊 Spoken answers (TTS)

- The assistant reads its reply aloud — Markdown, code blocks, links, and emoji are stripped first so nothing gets spelled out symbol by symbol.
- Adjustable playback speed (0.25×–4×) and a **Response length** dial (Short / Medium / Long) that shapes the answer itself.
- Four engines: **Kokoro** (free, fully local via WebGPU), **OpenAI‑compatible** (`/audio/speech`), **ElevenLabs**, and **Azure AI Speech**.

### 🎭 Profiles (personas)

- Switch the assistant between task‑focused profiles — each sets its name, role, instructions, and reply length.
- Ships with built‑ins: **Companion** (warm, empathetic), **Quick** (fast, one or two sentences), and **Unfiltered** (blunt, honest), alongside the default general‑purpose assistant.
- Edit name, avatar, instructions, and greeting — or **describe a profile in a sentence and let the LLM write it** for you.
- Duplicate, import/export as JSON to share, or delete. **Restore default** / **Restore built‑ins** bring the shipped ones back without touching your custom profiles.

### 🧠 Personal memory (local, private, off by default)

- Let the assistant remember you between chats: a short always‑on **"About You"** summary plus durable **notes** (preferred tone, tools you use, projects you're on).
- **Fully on‑device and yours to see** — Settings → Memory shows exactly what's stored, note by note.
- **Learns quietly, off the hot path:** it distills durable facts only when a conversation ends, never slowing down a live reply, reusing whatever model you already picked.
- **You're in control:** add/edit/delete notes, tune how much is used per reply (Light / Balanced / Detailed), export/import as JSON, or wipe it entirely.
- **Incognito** switch for a chat that's neither remembered nor personalized.
- **Safe by design:** refuses to store secrets/PII, ignores instruction‑like text, and remembered facts are always advisory — your current message always wins.

### 🎨 Make it yours

- Six accent colors, three text sizes, three panel sizes, adjustable opacity — with a live preview.
- Configurable system prompt and reply‑length control.
- Pick your dictation feedback sound — Default, Marimba, Pop, Click, or your own clips — with a preview button.
- **Every hotkey is rebindable.**

---

## 🆚 How it compares

SpeakoFlow keeps Handy's rock‑solid local dictation core and builds a full voice assistant on top:

|  | Handy | SpeakoFlow |
| --- | :---: | --- |
| Local dictation | ✅ | ✅ (unchanged core) |
| Generate with Flow ("Hey Flow") | — | ✅ speak a command, get finished writing pasted |
| Speak‑any‑language → English | — | ✅ on‑device translation (Whisper models) |
| AI cleanup (tones) | — | ✅ Professional / Friendly / Concise / custom |
| AI assistant chat panel | — | ✅ floating glass panel, streaming |
| Screen vision | — | ✅ voice intent, camera button, multi‑monitor |
| Web search | — | ✅ Serper / SerpAPI / Brave / Tavily / Exa |
| Spoken answers (TTS) | — | ✅ Kokoro local / OpenAI / ElevenLabs / Azure |
| Assistant profiles (personas) | — | ✅ gallery, per‑profile length, import/export |
| Personal memory | — | ✅ two‑tier, offline distillation, incognito |
| Built‑in offline LLM (llama.cpp) | — | ✅ runs a downloaded model, no server |
| Panel customization + pill mode | — | ✅ |

Everything is wired through the same provider system, so one API key works for assistant answers, dictation post‑processing, and remote TTS.

---

## ⌨️ Default hotkeys

Defaults differ per platform (and every shortcut is rebindable in Settings):

| Action | Windows | macOS | Linux |
| --- | --- | --- | --- |
| **Dictate** (push‑to‑talk) | `Left Ctrl + Left Super` | `Option + Space` | `Ctrl + Space` |
| **Dictate + AI cleanup** | `Ctrl + Shift + Space` | `Option + Shift + Space` | `Ctrl + Shift + Space` |
| **Ask the assistant** | `Left Ctrl + Left Alt` | `Option + Ctrl + Space` | `Ctrl + Alt + Space` |
| **Show / hide the panel** | `Ctrl + Shift + A` | `Option + Ctrl + A` | `Ctrl + Alt + A` |

- **Hands‑free:** while holding a dictation/assistant hotkey, **tap `Space`** to lock recording on — let go and keep talking. Stop the same way.
- **Generate with Flow:** just start a dictation with **"Hey Flow"** (no separate key needed).
- **Screen vision:** say *"what's on my screen?"* during an assistant question, or use the camera button on the panel.
- **Cancel:** a global `Esc` binding is available but **off by default** (so `Esc` still works for your other apps) — enable it in Settings by recording a key.

---

## 📦 Getting started

1. **Install and launch.** Download the latest build for your OS from [**Releases**](https://github.com/AbhishekBarali/SpeakoFlow/releases). A short setup wizard walks you through picking a transcription model (**Parakeet V3** is a great default) and, optionally, downloading a local AI model for the assistant. Downloads auto‑retry and resume where they left off if the connection drops.

2. **Pick an assistant provider** in **Settings → Assistant**:
   - **Built‑in (Local)** — fully offline, no key; download a small LLM from the model picker and you're set.
   - **Local** — an Ollama / LM Studio server, or any hosted preset (OpenAI, Anthropic, Groq, OpenRouter, Z.AI, Cerebras, AWS Bedrock) with your own API key.
   - **Custom (e.g. Azure OpenAI)** — base URL `https://{your-resource}.openai.azure.com/openai/v1`, model = your deployment name.
   - For screen vision, choose a model that supports images (`gpt-4o-mini`, `gpt-4.1-mini`, `gemini-flash`, …).

3. *(Optional)* Enable **Web search** in the same page (a free Serper key works great).

4. *(Optional)* Turn on **Personal memory** in **Settings → Memory** so the assistant remembers you between chats (off by default).

5. Press the assistant hotkey and ask something. 🎉

---

## 🛠️ Building from source

**Prerequisites:** [Rust](https://rustup.rs/) (latest stable) and [Bun](https://bun.sh/).

```bash
# 1. Clone
git clone https://github.com/AbhishekBarali/SpeakoFlow.git
cd SpeakoFlow

# 2. Install dependencies
bun install

# 3. Download the required VAD model
mkdir -p src-tauri/resources/models
curl -o src-tauri/resources/models/silero_vad_v4.onnx \
  https://blob.handy.computer/silero_vad_v4.onnx

# 4. Run in development
bun run tauri dev

# 5. Build a release + installer
bun run tauri build
```

See [**BUILD.md**](BUILD.md) for platform‑specific prerequisites (CMake, Vulkan SDK on Windows/Linux, etc.).

---

## 🧱 Tech stack

**Framework:** [Tauri 2.x](https://tauri.app) (Rust backend + web frontend)

**Backend (Rust):**
- [`whisper-rs`](https://github.com/tazz4843/whisper-rs) + Parakeet — local speech‑to‑text with GPU acceleration
- [`vad-rs`](https://github.com/) / Silero VAD — voice activity detection
- [`cpal`](https://github.com/RustAudio/cpal) — cross‑platform audio I/O · [`rubato`](https://github.com/HEnquist/rubato) — resampling · [`rodio`](https://github.com/RustAudio/rodio) — feedback sounds
- `handy-keys` + [`rdev`](https://github.com/Narsil/rdev) — global shortcuts & low‑level input
- `llama.cpp` — the built‑in offline assistant LLM · an OpenAI‑compatible client for hosted providers

**Frontend (TypeScript):**
- [React 18](https://react.dev) + [Vite 6](https://vitejs.dev) + [Tailwind CSS v4](https://tailwindcss.com)
- [Zustand](https://github.com/pmndrs/zustand) (state) · [i18next](https://www.i18next.com/) (i18n) · [kokoro‑js](https://github.com/hexgrad/kokoro) (local TTS via WebGPU)

Three windows ship from one app: the settings window, the floating assistant panel, and the recording overlay.

---

## 🔒 Privacy model

- **Voice → text:** always local, never leaves your machine.
- **Assistant questions + optional screenshots:** sent only to the LLM provider *you* configure — which can be the built‑in local model or your own server.
- **Web search** (off by default): only your search query goes to the provider you pick, and just short snippets come back — your conversation is never sent.
- **TTS:** local by default (Kokoro); remote only if you choose a remote engine.
- **Personal memory** (off by default): stays entirely on your device; nothing is uploaded. Incognito skips it; inspect, edit, export, or wipe it anytime.
- **Fully offline option:** pair the built‑in local LLM with Kokoro TTS and web search off, and *nothing* leaves your machine.
- **No telemetry, no accounts, no cloud middleman.**

---

## 🗺️ Roadmap

SpeakoFlow is actively developed. On the horizon:

- 📚 Full documentation site (this README is the entry point for now)
- ✍️ Code signing for Windows & macOS installers
- 🌱 Wider model catalog and more one‑click local models
- 🌍 More community translations ([contribute here](CONTRIBUTING_TRANSLATIONS.md))

Have an idea? Open a [Discussion](https://github.com/AbhishekBarali/SpeakoFlow/discussions) — feature proposals are welcome before a PR.

---

## 🤝 Contributing

Contributions are very welcome! Please read [**CONTRIBUTING.md**](CONTRIBUTING.md) for the full workflow, and [**CONTRIBUTING_TRANSLATIONS.md**](CONTRIBUTING_TRANSLATIONS.md) if you'd like to help translate the app.

A quick primer:

```bash
bun run lint          # ESLint for the frontend
bun run format        # Prettier + cargo fmt
```

- Use conventional commit prefixes (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`).
- All user‑facing strings go through i18next (ESLint enforces this).
- For new features, open a Discussion first to gather feedback.

---

## 📄 License

Released under the [**MIT License**](LICENSE) — free to use, modify, and distribute.

---

## 🙏 Credits & acknowledgements

SpeakoFlow is built on [**Handy**](https://github.com/cjpais/Handy) by **CJ Pais** and contributors — the local dictation core (Whisper/Parakeet pipeline, VAD, overlay, and settings architecture) is their excellent work, and SpeakoFlow keeps it under the same MIT license. Huge thanks to them.

Also grateful to the open‑source projects that make this possible: Tauri, whisper.cpp / whisper‑rs, llama.cpp, Silero VAD, Kokoro, and the wider Rust and React ecosystems.

---

## 👤 Author

**Abhishek Barali**

- 🌐 Website: [speakoflow.com](https://www.speakoflow.com)
- 🐙 GitHub: [@AbhishekBarali](https://github.com/AbhishekBarali)
- 💬 Questions & ideas: [GitHub Discussions](https://github.com/AbhishekBarali/SpeakoFlow/discussions)

<div align="center">

---

**If SpeakoFlow saves you some typing, consider giving it a ⭐ — it genuinely helps the project get discovered.**

*Just speak. It writes.*

</div>
