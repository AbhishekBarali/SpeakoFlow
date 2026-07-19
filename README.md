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

Press a hotkey and talk, and your words are typed into whatever app you're using. Speech-to-text runs locally on your machine. The AI assistant runs on any model you choose, including a fully offline one, so you can keep everything on your device if you want to.

## Features

- **Dictation.** Talk into any app. Words appear live as you speak or all at once when you stop.
- **Generate with Flow.** Start a dictation with "Hey Flow" and it writes the reply, email, or draft and pastes it for you.
- **Translate.** Speak another language and get clean English (with a Whisper model).
- **AI cleanup.** Remove filler and fix grammar in a tone you choose.
- **Assistant panel.** A floating chat you open with a hotkey. It can talk back and read your screen when you ask.
- **Optional extras.** Web search, spoken answers, assistant profiles, and on-device memory. All off until you turn them on.

Everything lives in Settings, and every hotkey is rebindable.

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

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md), and [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md) to help translate the app.

## License

[MIT](LICENSE)

## Credits

SpeakoFlow builds on [Handy](https://github.com/cjpais/Handy) by CJ Pais, which provides the local dictation core, under the MIT license. Thanks also to [Tauri](https://tauri.app), whisper.cpp, llama.cpp, Silero VAD, and [Kokoro](https://github.com/hexgrad/kokoro).

<div align="center">

Made by [Abhishek Barali](https://github.com/AbhishekBarali) · [speakoflow.com](https://www.speakoflow.com)

</div>
