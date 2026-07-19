<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="Logo/final/lockup-dark.svg" />
  <img src="Logo/final/lockup.svg" alt="SpeakoFlow" width="320" />
</picture>

**A free, local voice assistant for your desktop. Dictation, writing, and an AI assistant, all by voice.**

[![License: MIT](https://img.shields.io/badge/License-MIT-2ea44f.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-informational)](#install)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)

[Download](https://github.com/AbhishekBarali/SpeakoFlow/releases) &nbsp;·&nbsp; [Website](https://www.speakoflow.com) &nbsp;·&nbsp; [Discussions](https://github.com/AbhishekBarali/SpeakoFlow/discussions)

<!-- Add a real-time transcription demo GIF here (e.g. demo-assets/hero.gif). -->

</div>

---

Press a hotkey and talk, and your words are typed into whatever app you're using. Say "Hey Flow" to turn what you say into a finished reply or email, or open a floating assistant panel to chat by voice.

Speech-to-text runs locally on your machine, so your voice never leaves your device. The assistant runs on any model you choose, including a fully offline one, so you can keep everything local if you want to.

## Features

- **Dictation.** Press a hotkey and talk. Words type into any app, live as you speak or all at once when you stop. Transcription runs on your GPU or CPU.
- **Generate with Flow.** Begin a dictation with "Hey Flow" and it writes the reply, email, or draft and pastes it for you. You can rename the phrase.
- **Translate.** Speak another language and get clean English, on your device (with a Whisper model).
- **AI cleanup.** Strip filler and fix grammar in a tone you choose: Professional, Friendly, Concise, or your own.
- **Assistant panel.** A floating chat you open with a hotkey. Ask by voice or text, get streaming answers, and have them read back aloud.
- **Screen vision.** Ask about what's on your screen and the assistant answers with that context. It only looks when you tell it to.
- **Web search.** Optional. The assistant can look things up for current, factual answers.
- **Profiles.** Switch the assistant between personas, each with its own voice and reply length.
- **Personal memory.** Optional, on-device memory so the assistant learns how you like to work. Off until you turn it on.

Everything lives in Settings, and every hotkey is rebindable.

## Default hotkeys

| Action | Windows | macOS | Linux |
| --- | --- | --- | --- |
| Dictate | `Left Ctrl + Left Super` | `Option + Space` | `Ctrl + Space` |
| Ask the assistant | `Left Ctrl + Left Alt` | `Option + Ctrl + Space` | `Ctrl + Alt + Space` |

Hold to talk, or tap `Space` while holding to keep recording hands-free. All shortcuts are configurable in Settings.

## Install

Download the latest build for Windows, macOS, or Linux from [Releases](https://github.com/AbhishekBarali/SpeakoFlow/releases). A short setup wizard helps you pick a transcription model and, optionally, a local model for the assistant.

For the assistant, choose a provider in Settings: the built-in offline model, a local server (Ollama or LM Studio), or any cloud provider with your own API key.

## Build from source

Requires [Rust](https://rustup.rs/) and [Bun](https://bun.sh/).

```bash
git clone https://github.com/AbhishekBarali/SpeakoFlow.git
cd SpeakoFlow
bun install
mkdir -p src-tauri/resources/models
curl -o src-tauri/resources/models/silero_vad_v4.onnx https://blob.handy.computer/silero_vad_v4.onnx
bun run tauri dev
```

See [BUILD.md](BUILD.md) for platform-specific setup.

## Privacy

Your voice is transcribed on your device and never uploaded. The assistant only contacts the model provider you choose, which can be fully local. No telemetry, no account.

## Roadmap

- Code signing for Windows and macOS
- A wider model catalog and more one-click local models
- More community translations
- Voice-to-text tuned for agentic coding
- Prompt-engineering help: describe what you want to build and get a solid prompt back
- Voice commands: trigger actions and complete tasks by voice

Have an idea? Open a [Discussion](https://github.com/AbhishekBarali/SpeakoFlow/discussions).

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md), and [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md) to help translate the app.

## License

[MIT](LICENSE)

## Credits

SpeakoFlow started as a fork of [Handy](https://github.com/cjpais/Handy) by CJ Pais, which provides the local dictation core, under the MIT license. Thanks also to [Tauri](https://tauri.app), whisper.cpp, llama.cpp, Silero VAD, and [Kokoro](https://github.com/hexgrad/kokoro).

<div align="center">

Made by [Abhishek Barali](https://github.com/AbhishekBarali) · [speakoflow.com](https://www.speakoflow.com)

</div>
