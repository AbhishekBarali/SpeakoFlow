<div align="center">

**English** · [繁體中文](README.zh-TW.md)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="Logo/final-v2/png/lockup-dark-h256.png" />
  <img src="Logo/final-v2/png/lockup-h256.png" alt="SpeakoFlow" width="340" />
</picture>

# SpeakoFlow: free voice dictation and an AI assistant for Windows, macOS, and Linux

### You think faster than you type.

**Talk and it types, in any app. Ask and it answers. Call it and talk things through. It also writes up your meetings. Free, open source, and local by default.**

[![Latest release](https://img.shields.io/github/v/release/AbhishekBarali/SpeakoFlow?label=release&color=0A7A70)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-2ea44f.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-informational)](#install)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)

<img src="assets/readme/demo.webp" width="760" alt="SpeakoFlow in action: dictating an email that types itself out, asking the assistant to translate selected text and replacing it with the answer, then a spoken call with the assistant" />

### Download

[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D4?logo=windows&logoColor=white&style=for-the-badge)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)
[![Download for macOS](https://img.shields.io/badge/Download-macOS-000000?logo=apple&logoColor=white&style=for-the-badge)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)
[![Download for Linux](https://img.shields.io/badge/Download-Linux-FCC624?logo=linux&logoColor=black&style=for-the-badge)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)

[All releases](https://github.com/AbhishekBarali/SpeakoFlow/releases) &nbsp;·&nbsp; [Website](https://www.speakoflow.com) &nbsp;·&nbsp; [Documentation](https://www.speakoflow.com/docs)

</div>

> **Staying up to date:** SpeakoFlow checks for new versions itself and installs
> them from Settings → About. To hear about releases on GitHub too, click
> **Watch → Custom → Releases** at the top of this page.

---

## Contents

- [What is SpeakoFlow?](#what-is-speakoflow)
- [Why SpeakoFlow](#why-speakoflow)
- [Features](#features)
- [A look around](#a-look-around)
- [Default hotkeys](#default-hotkeys)
- [Models and providers](#models-and-providers)
- [Install](#install)
- [Build from source](#build-from-source)
- [Tech stack](#tech-stack)
- [Privacy](#privacy)
- [Troubleshooting](#troubleshooting)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [License](#license)
- [Credits](#credits)

## What is SpeakoFlow?

SpeakoFlow is a desktop app that gives your voice three hotkeys. One types
what you say into whatever app you're in. One asks a question and puts the
answer where you need it. One starts a spoken conversation with an AI that
answers out loud.

<table>
<tr>
<td width="33%" valign="top">

**Dictate**

<kbd>Left Ctrl</kbd> + <kbd>Left Win</kbd>

Hold the keys, talk, let go. What you said is typed at your cursor, in any app
that takes text.

</td>
<td width="33%" valign="top">

**Ask**

<kbd>Left Ctrl</kbd> + <kbd>Left Alt</kbd>

Select some text if you want, then ask out loud: translate this, reply to
this, explain this. Copy the answer, insert it, or replace the selection.

</td>
<td width="33%" valign="top">

**Call**

<kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>C</kbd>

A hands-free conversation. Talk a problem through and it answers out loud.
Press the keys again to hang up.

</td>
</tr>
</table>

<sub>Windows defaults shown. The macOS and Linux keys are under
[Default hotkeys](#default-hotkeys), and every one of them is rebindable.</sub>

Speech-to-text runs on your own machine unless you choose a cloud service. The
assistant runs on whichever model you pick: a built-in model that works fully
offline, your own Ollama or LM Studio server, or a cloud provider with your
own API key. You decide how much leaves your computer.

I built it while studying alone for exams. I was paying for dictation software
that stopped at typing: it could hear me, but it couldn't help me.

## Why SpeakoFlow

Most voice tools are dictation tools. SpeakoFlow is a dictation tool too, and
then it keeps going: an assistant you can ask from any app, a voice you can
talk to, and notes from your meetings. It's free, MIT licensed, and the same
app runs on Windows, macOS, and Linux.

|                     | SpeakoFlow | Wispr Flow                                               | Superwhisper                                  | Handy      |
| ------------------- | ---------- | -------------------------------------------------------- | --------------------------------------------- | ---------- |
| Price               | Free       | Free up to 2,000 words a week on desktop, then $15/month | Free tier, Pro at $8.49/month or $249.99 once | Free       |
| Source code         | Open (MIT) | Closed                                                   | Closed                                        | Open (MIT) |
| Linux               | Yes        | No                                                       | No                                            | Yes        |
| Transcribes offline | Yes        | No                                                       | Yes                                           | Yes        |

<sub>Prices and platforms from each product's own site, checked October 2026.</sub>

- **Compared with Wispr Flow.** Wispr Flow is polished, but it is closed
  source, needs an internet connection to transcribe, and has no Linux build.
  SpeakoFlow transcribes on your machine with no word cap. Full breakdown:
  [SpeakoFlow vs Wispr Flow](https://www.speakoflow.com/blog/speakoflow-vs-wispr-flow).
- **Compared with Superwhisper.** Superwhisper is a capable closed-source app
  on macOS, Windows, and mobile. SpeakoFlow is free, MIT licensed, and also
  runs on Linux.
- **Built on Handy.** SpeakoFlow's dictation core comes from
  [Handy](https://github.com/cjpais/Handy), the more established project and a
  genuinely good pure-dictation tool. If all you need is dictation, Handy is a
  solid choice. If you want your computer to answer you, keep reading.

See also:
[the best free and open-source Wispr Flow alternatives](https://www.speakoflow.com/blog/best-free-open-source-wispr-flow-alternatives).

## Features

### Dictation: type into any app with your voice

Hold the hotkey, talk, and let go. The words are typed wherever your cursor is,
and the recording pill can show them as you speak. Transcription runs on your
GPU or CPU with a local model: Parakeet by default (English), Nemotron (28
languages, and it works out which one you're speaking), Canary, Cohere
Transcribe, Whisper (99 languages), or any of the 65 speech models in the
catalog. Prefer the cloud? ElevenLabs and Deepgram stream text while you talk,
and OpenAI, Groq, Mistral, Azure AI Speech, and OpenRouter work too.

The details that make it usable every day:

- **Undo a dismissed dictation.** Cancelled a recording by accident? The pill
  offers Undo for a few seconds, and History keeps it so you can recover it
  later. A failed transcription offers Try again.
- **Your words, spelled your way.** Add names and jargon to the Dictionary and
  they're spelled right, or write replacement rules. On Windows it can learn
  the words you correct.
- **Speak another language, get English.** Whisper models (and OpenAI or Groq
  in the cloud) can translate to English as they transcribe.
- **Hold or tap.** Hold the keys while you talk, or switch to tap so one press
  starts and the next press stops, for longer hands-free dictation.

### Ask: one question, one answer, from any app

<div align="center">
<img src="assets/readme/ask.webp" width="720" alt="The quick ask in three examples: translating a selected message into Spanish and replacing it, writing a reply that is inserted at the cursor, and explaining a selected sentence" />
</div>

Select some text, or don't. Hold <kbd>Left Ctrl</kbd> + <kbd>Left Alt</kbd> and
say what you want: "translate this to Spanish", "write a polite reply saying I
can't make Thursday", "explain this". The answer streams into a card at the
top of your screen. Copy it, insert it at your cursor, or have it replace the
text you selected.

It can do more when you let it:

- **Screen vision.** Ask about the error in your terminal or the chart in your
  spreadsheet. It's off until you turn it on, and even then the model decides
  per question whether it needs to look. The capture goes only to the provider
  you chose, a screenshot it doesn't use never leaves your machine, and only a
  small thumbnail is kept.
- **Web search** for current answers, through Serper, Brave, Tavily, Exa,
  SerpAPI, or TinyFish.
- **Reminders.** "Remind me to send the invoice in twenty minutes." The
  reminder survives a restart and pops up without stealing your keyboard.
- **Profiles and memory.** Switch personas, each with its own reply length, and
  let it learn how you like to work. Memory is off by default, stored on your
  device, and you can edit or erase it at any time.

### Call: talk it through, out loud

Press <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>C</kbd> and just talk. The
assistant answers out loud, you can cut in while it's speaking, and pressing
the keys again hangs up. Need to type something mid-call? Dictate as usual: the
call holds while you do, then picks up again.

Voices run on your computer (Kokoro on your graphics card or processor, plus
Kitten, Pocket TTS, and Supertonic) or in the cloud with OpenAI, ElevenLabs,
Deepgram, Cartesia, Google, Azure, and others.

### Meetings: every call, written down

<table>
<tr>
<td width="50%"><img src="assets/readme/screens/meetings-live.webp" alt="A meeting recording in SpeakoFlow, with a live transcript that separates what you said from what the other side said" /></td>
<td width="50%"><img src="assets/readme/screens/meeting-notes.webp" alt="Notes SpeakoFlow wrote after a meeting: a summary, key takeaways, topics, decisions, and next steps with owners" /></td>
</tr>
</table>

Press Start recording before a call. SpeakoFlow records your microphone and
your computer's audio as two separate streams and transcribes as people speak,
so the transcript always knows which words were yours. No bot joins the
meeting, and it works with any meeting app.

When the call ends, it writes the notes: a summary, decisions, and next steps
with owners, in the shape you pick (General, Standup, One-on-one, Interview, or
Action items). Speaker labels split the other side into Speaker 1, Speaker 2,
and so on. Ask questions about the meeting afterwards, or open it in a call and
talk it over. Windows can offer to start recording when it notices a call.

The transcript uses the speech model you already chose, and the notes use the
assistant's model. On macOS, recording the other side of a call needs a virtual
audio device such as BlackHole; see [Troubleshooting](#troubleshooting).

### AI cleanup, by a model we trained for it

SpeakoFlow Mini is our own model, and it does one thing, turning what you said
into clean text. It strips filler, fixes grammar and punctuation, and applies
spoken edits, so saying "new paragraph" or "scratch that" mid-dictation does
what you meant instead of getting typed out. It is a 795 MB download, it runs on
your machine, and it handles English only for now.

Layer a writing style on top: Professional, Friendly, Concise, Formal, Casual,
or your own instruction. Cleanup is off until you turn it on, and then runs on
its own hotkey or on every dictation, your choice. Any other local or cloud
model can do the job instead, including Apple Intelligence on Apple silicon
Macs.

### Insights and history

Insights counts your dictation on this computer: words dictated, your speaking
speed, the time saved against typing at 40 words a minute, and a six-month
activity map with streaks. History keeps your dictations and assistant chats.
Play a recording back, transcribe it again with a different model, recover a
dismissed one, or reopen a chat as a call. Old recordings can delete
themselves after a set number, a few days, or a few months.

### Use the models you already have

If a model is already on your disk, SpeakoFlow will use it where it sits. Add a
`.gguf` or a Whisper `.bin`, or link a folder and every model inside it appears,
subfolders included. Nothing is copied, nothing is moved, and removing an entry
only takes it off the list. Downloads that do happen fetch eight chunks at once
and resume where they stopped.

### Generate with Flow

Start a dictation with "Hey Flow" and SpeakoFlow writes instead of
transcribing: describe the email or reply you want and the finished text is
pasted at your cursor. It's still here, off by default, under Settings →
Dictation. Ask does the same job with less ceremony, and it can see what you
selected.

Full documentation for each:
[dictation](https://www.speakoflow.com/docs/dictation/basics),
[live transcription](https://www.speakoflow.com/docs/dictation/live-transcription),
[custom words](https://www.speakoflow.com/docs/dictation/custom-words),
[languages and translation](https://www.speakoflow.com/docs/models/languages),
[the assistant](https://www.speakoflow.com/docs/assistant/panel),
[screen vision](https://www.speakoflow.com/docs/assistant/screen-vision),
[web search](https://www.speakoflow.com/docs/assistant/web-search),
[voices](https://www.speakoflow.com/docs/assistant/voice-output),
[profiles](https://www.speakoflow.com/docs/personalize/profiles),
[memory](https://www.speakoflow.com/docs/personalize/memory),
[history](https://www.speakoflow.com/docs/personalize/history),
[AI cleanup](https://www.speakoflow.com/docs/writing/ai-cleanup),
[writing styles](https://www.speakoflow.com/docs/writing/writing-styles), and
[Generate with Flow](https://www.speakoflow.com/docs/writing/generate-with-flow).

## A look around

<table>
<tr>
<td width="50%" valign="top"><img src="assets/readme/screens/home.webp" alt="SpeakoFlow's home page: the dictation hotkey, every shortcut, the models powering each job, and recent dictations" /><br /><sub><b>Home.</b> Your hotkeys, the models doing each job, and what you dictated last.</sub></td>
<td width="50%" valign="top"><img src="assets/readme/screens/assistant.webp" alt="The Assistant page: the ask and call hotkeys, the model it thinks with, its voice, and switches for screen vision and web search" /><br /><sub><b>Assistant.</b> Pick its model and voice, and what it's allowed to do.</sub></td>
</tr>
<tr>
<td width="50%" valign="top"><img src="assets/readme/screens/cleanup.webp" alt="The AI cleanup page, showing a messy spoken sentence and the clean text it becomes, with writing styles to choose from" /><br /><sub><b>AI cleanup.</b> Say it messy, get it clean, in the style you pick.</sub></td>
<td width="50%" valign="top"><img src="assets/readme/screens/insights.webp" alt="The Insights page: words dictated, words per minute, time saved, number of dictations, and a six-month activity map" /><br /><sub><b>Insights.</b> How much you dictate, and how much typing it saved.</sub></td>
</tr>
<tr>
<td width="50%" valign="top"><img src="assets/readme/screens/models.webp" alt="The speech-to-text model settings, with cloud providers such as ElevenLabs, OpenRouter, OpenAI, Deepgram, and Azure AI Speech" /><br /><sub><b>Models.</b> On this computer or in the cloud, one job at a time.</sub></td>
<td width="50%" valign="top"><img src="assets/readme/screens/models-voice.webp" alt="The voice settings, with on-device voices such as Kokoro, Kitten, Pocket TTS, and Supertonic, and cloud voices from a dozen providers" /><br /><sub><b>Voices.</b> Local voices or a dozen cloud ones for spoken replies.</sub></td>
</tr>
</table>

## Default hotkeys

| Action                            | Windows                        | macOS                          | Linux                          |
| --------------------------------- | ------------------------------ | ------------------------------ | ------------------------------ |
| Dictate                           | `Left Ctrl + Left Win`         | `Option + Space`               | `Ctrl + Space`                 |
| Ask the assistant                 | `Left Ctrl + Left Alt`         | `Option + Ctrl + Space`        | `Ctrl + Alt + Space`           |
| Start or end a call               | `Ctrl + Shift + C`             | `Option + Ctrl + C`            | `Ctrl + Alt + C`               |
| Dictate and clean up <sup>1</sup> | `Ctrl + Shift + Space`         | `Option + Shift + Space`       | `Ctrl + Shift + Space`         |
| Cancel                            | Not set. Bind one in Settings. | Not set. Bind one in Settings. | Not set. Bind one in Settings. |

<sup>1</sup> Only while AI cleanup is on and set to its own hotkey.

Hold a recording hotkey to talk and release it to finish, or switch the home
page's **Hold to talk** to **Tap to toggle** so one press starts and the next
press stops. The choice applies to every recording hotkey. Cancel is unset
because a global Esc would swallow Esc presses meant for other apps.

Every shortcut and its default, on all three platforms:
[Keyboard shortcuts](https://www.speakoflow.com/docs/start/keyboard-shortcuts).
To trigger SpeakoFlow from a script or your window manager, see the
[command-line flags](https://www.speakoflow.com/docs/settings/cli).

## Models and providers

Every job in SpeakoFlow can run on your computer or on a provider you choose.
Cloud providers use your own API key, which is stored in your system keychain.

| Job                   | On your computer                                                                                                                                        | In the cloud, with your key                                                                                                                                                                               |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Speech to text        | Parakeet, Nemotron, Canary, Cohere Transcribe, Whisper, Moonshine, Voxtral, Qwen3-ASR, GigaAM, Granite Speech, SenseVoice, and more (65 in the catalog) | ElevenLabs, Deepgram, OpenAI, Groq, Mistral (Voxtral), Azure AI Speech, OpenRouter, or any OpenAI-compatible server                                                                                       |
| Assistant and cleanup | Built-in engine (llama.cpp, fully offline), Ollama, LM Studio, SpeakoFlow Mini for cleanup, Apple Intelligence for cleanup on Apple silicon             | OpenAI, Anthropic, Google Gemini, Azure OpenAI, AWS Bedrock, OpenRouter, Groq, Cerebras, xAI, DeepSeek, Mistral, Moonshot, Together AI, Fireworks AI, Perplexity, Z.AI, or any OpenAI-compatible endpoint |
| Spoken replies        | Kokoro, Kitten, Pocket TTS, Supertonic                                                                                                                  | OpenAI, ElevenLabs, OpenRouter, Deepgram, Cartesia, Google Cloud, Azure AI Speech, Groq, xAI, Mistral, Inworld, or your own server                                                                        |
| Web search (optional) |                                                                                                                                                         | Serper, Brave, Tavily, Exa, SerpAPI, TinyFish                                                                                                                                                             |

The app itself is translated into 20 languages.

## Install

Download the latest build for Windows, macOS, or Linux from the
[Releases](https://github.com/AbhishekBarali/SpeakoFlow/releases) page. The
first launch asks you to pick a speech model and starts downloading it, then a
short tour shows the three hotkeys while the download finishes in the
background.

### Windows

Download the `.exe` installer and run it. Windows may show a SmartScreen notice
because the installer isn't signed by a known publisher yet. Choose **More
info → Run anyway**.

### Linux

- **Arch Linux.** Install from the AUR:
  ```bash
  yay -S speakoflow-bin
  # or
  paru -S speakoflow-bin
  ```
- **Debian 13+, Ubuntu 24.04+, Mint 22+, Pop!\_OS, Tuxedo OS.** Download the
  `.deb` and install it. This registers the app icon and menu entry properly,
  which the AppImage can't do on its own:
  ```bash
  sudo apt install ./SpeakoFlow_*_amd64.deb
  ```
- **Any other distribution, including Fedora and openSUSE.** Download the
  AppImage, make it executable (`chmod +x`), and run it. Note that an AppImage
  doesn't integrate with your desktop by itself, so it won't show an icon in your
  file manager or app menu; tools like Gear Lever or AppImageLauncher add that if
  you want it.

Both packages are built on Ubuntu 24.04, so both need glibc 2.39 or newer. That
rules out Ubuntu 22.04, Debian 12, Mint 21 and RHEL/Alma/Rocky 9, which ship older
glibc and can't start either package.

The AppImage and `.deb` are both built for x86_64 and ARM64. There's no `.rpm`
yet, because the packaging doesn't bundle the speech engine correctly, and
shipping one that installs but can't transcribe would be worse than not shipping
it.

### macOS

Download the `.dmg` and drag **SpeakoFlow** into Applications. macOS then needs
**Microphone** and **Accessibility** permissions (_System Settings → Privacy &
Security_) so SpeakoFlow can hear you and type into other apps.

Because the app isn't Apple-signed yet, macOS blocks the first launch and needs
one Terminal command to clear it. Full explanation below, or in the
[install docs](https://www.speakoflow.com/docs/start/install#macos).

<details>
<summary><b>Why macOS says "SpeakoFlow is damaged", and the one-line fix</b></summary>

<br />

SpeakoFlow works fully on macOS, but it isn't signed by Apple yet, so macOS
blocks it on first launch with a message that says **"SpeakoFlow is damaged and
can't be opened."**

**The app is not damaged.** That wording is what macOS shows for any app it
can't trace to a paid Apple Developer account. Signing costs $99/year, which
this project doesn't have yet, so the block is expected and harmless.

Install it in three steps:

1. Download `SpeakoFlow_<version>_aarch64.dmg` and drag **SpeakoFlow** into your
   Applications folder.
2. Open **Terminal** (press `Cmd + Space`, type `Terminal`) and paste this,
   then press Return:
   ```bash
   xattr -dr com.apple.quarantine /Applications/SpeakoFlow.app
   ```
3. Open SpeakoFlow normally, from Launchpad, Spotlight, or Applications.

**You only do this once per version you download.** The command removes the
"downloaded from the internet" tag that macOS puts on the file; after that the
app opens like any other. Updates installed from inside the app (Settings →
About) aren't tagged, so they don't need it. If you download a new `.dmg` by
hand, run the command again for that copy. Never per launch.

After an update, macOS can sometimes keep showing SpeakoFlow as allowed under
Accessibility, Microphone or Screen Recording while no longer honouring it. If a
permission screen keeps waiting, use its **Reset permission** button, then switch
SpeakoFlow on again in System Settings.

If you're wondering why there's no button to click instead: macOS 15 and later
removed the old right-click → **Open** bypass, and the "damaged" message is the
one case where no **Open Anyway** button appears in _System Settings → Privacy &
Security_. Terminal is the only route left. Proper Apple signing and
notarization is on the [roadmap](#roadmap) and removes this step entirely.

**Intel Macs, from 1.3.0 onward.** Download
`SpeakoFlow_<version>_x64.dmg` for Intel and
`SpeakoFlow_<version>_aarch64.dmg` for M1 and newer. The Intel build needs
**macOS 14 Sonoma or later**. It is CPU only, since the GPU backend targets
Apple Silicon, so transcription is slower than on Apple Silicon but fully
functional. Every Intel build is checked in CI
on a real Intel machine: the app's own libraries are the only ones left in
place, then the binary is launched, so a bundle that could not start on your Mac
fails the build instead of reaching the release page. You can also
[build from source](#build-from-source); see [BUILD.md](BUILD.md) for the extra
Intel step.

> An earlier version of this section said GitHub had retired its Intel build
> machines, leaving no way to produce or test an Intel build. That was wrong.
> GitHub retired the old `macos-13` runner in December 2025 but replaced it with
> `macos-15-intel`, which is available until August 2027. Thanks to
> [@hellosimplerick](https://github.com/AbhishekBarali/SpeakoFlow/issues/19) for
> catching it, which is why the Intel build now exists.

</details>

### Updates

From 1.5.0, SpeakoFlow updates itself. It checks in the background and
installs the new version from Settings → About. Every update is verified
against the project's signing key before it's installed. The AUR package
updates through your package manager instead.

### Choosing the assistant's model

Pick the assistant's model on its page in the app:

- **Built-in (offline).** Download a small local model and run it fully on your
  machine, no key needed.
- **Local server.** Point SpeakoFlow at Ollama or LM Studio.
- **Cloud.** Bring your own API key for any of the providers in
  [Models and providers](#models-and-providers).

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

On Arch Linux and Arch-based distributions, build and install the current
checkout with:

```bash
bun run install:arch
speak
```

This installs the app for the current user under `~/.local`, including its
speech-engine libraries, desktop entry, and `speak` terminal command.

See [BUILD.md](BUILD.md) for platform-specific setup.

## Tech stack

- **App:** [Tauri 2](https://tauri.app) with a Rust backend and a React and TypeScript frontend.
- **Speech-to-text:** transcribe.cpp for GGUF speech models, whisper.cpp, and Parakeet on ONNX Runtime, with GPU acceleration and Silero VAD for voice detection.
- **Assistant and cleanup:** a built-in llama.cpp engine, or any OpenAI-compatible provider you configure.
- **Voices:** [Kokoro](https://github.com/hexgrad/kokoro) in the app's own window on your graphics card, and Kokoro, Kitten, Pocket TTS, and Supertonic on your processor through [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx).
- **Meetings:** separate microphone and system-audio capture, with WeSpeaker voice embeddings for speaker labels.

## Privacy

By default your voice is transcribed on your device and never uploaded. There
is no telemetry, no analytics, and no account.

Data only leaves your computer for things you set up yourself:

- **A cloud speech service**, if you pick one instead of a local model. It
  receives your recordings.
- **The assistant's provider**, if it isn't a local one. It receives your
  questions, any text you selected, a screenshot when screen vision is on and
  the model asks for one, and a meeting's transcript when you generate notes or
  ask about it.
- **Web search**, if you turn it on. The search provider receives the query.
- **Feedback**, if you send it from the app. It sends exactly what the dialog
  shows you, and nothing else.

API keys are kept in your system keychain. Personal memory is off until you
turn it on, and it lives on your device where you can view, edit, or erase it.

Full detail on what is stored and where:
[the privacy page](https://www.speakoflow.com/docs/reference/privacy).

## Troubleshooting

Common issues are collapsed below. For anything not covered here, see
[the troubleshooting docs](https://www.speakoflow.com/docs/reference/troubleshooting) or
[open an issue](https://github.com/AbhishekBarali/SpeakoFlow/issues).

<details>
<summary><b>macOS: a meeting only records my side of the call</b></summary>

<br />

macOS gives apps no direct way to record the sound your computer plays. On
Windows SpeakoFlow captures it through WASAPI loopback, and on Linux through
your PulseAudio or PipeWire monitor source, but a Mac needs a virtual audio
device in between.

Install a loopback driver such as [BlackHole](https://github.com/ExistentialAudio/BlackHole),
create a Multi-Output Device in Audio MIDI Setup that sends sound to both your
speakers and BlackHole, and make it your output. SpeakoFlow can then record the
other side of the call. Your microphone is recorded either way.

</details>

<details>
<summary><b>Linux: the recording overlay won't stay on top of other apps</b></summary>

<br />

The recording overlay has to float above every other window. On Linux that is only possible two ways: the `wlr-layer-shell` protocol (used by wlroots compositors like Sway and Hyprland, and by KDE Plasma) or classic X11 "keep above" stacking.

**A native GNOME/Wayland session supports neither.** Mutter does not implement `wlr-layer-shell`, and Wayland gives apps no way to raise themselves above others. So under native GNOME/Wayland the overlay can't stay on top.

SpeakoFlow handles this automatically: when it detects GNOME on Wayland it runs under **XWayland**, where "keep above" works and the overlay floats normally. This is on by default and needs no setup. X11 sessions and KDE/wlroots Wayland already work out of the box.

- Force native Wayland anyway (the overlay may not stay on top): launch with `SPEAKOFLOW_ALLOW_WAYLAND=1`.
- If the overlay misbehaves under a layer-shell compositor, disable layer shell with `SPEAKOFLOW_NO_GTK_LAYER_SHELL=1`.

</details>

<details>
<summary><b>Linux: hotkeys do nothing and the logs repeat "Permission denied"</b></summary>

<br />

If dictation and the assistant hotkeys don't respond on Linux and you see the log
repeating `rdev grab error: ... PermissionDenied` (errno 13), the app can't read
your input devices. This affects the **handy-keys** keyboard engine, which reads
`/dev/input/event*` and needs your user to be in the `input` group.

Two ways to fix it:

- **Grant access.** Add your user to the `input` group, then log out and back in:

  ```bash
  sudo usermod -aG input $USER
  ```

- **Or switch engines.** Set the keyboard engine to **Tauri** in Settings. It
  registers hotkeys through X11 (the `global-hotkey` crate behind Tauri's
  global-shortcut plugin supports only X11 on Linux) and needs no special
  permissions. It works in an X11 session and under XWayland, but on Wayland it
  only sees a hotkey while an X11 window has focus, so in a native Wayland app it
  can miss the press. (Tauri is already the default engine on Linux, so this only
  affects you if you switched to handy-keys.)

On Wayland, the reliable route is a shortcut owned by your desktop itself: add a
custom keyboard shortcut in your desktop's settings (GNOME, KDE, or a `bind` line
in Sway/Hyprland) that runs `speakoflow --toggle-transcription` (with an
AppImage, the path to the AppImage followed by the same flag). It reaches the
running app from any window, on any compositor. `--toggle-post-process`,
`--toggle-assistant`, and `--cancel` work the same way.

</details>

<details>
<summary><b>Linux: the app crashes when you pinch-to-zoom on a touchpad</b></summary>

<br />

On some Linux setups a trackpad pinch-to-zoom gesture crashes the window, with
`Received invalid message: 'DrawingArea_CommitTransientZoom'` in the logs. This
is a bug in **WebKitGTK** (the Linux web engine Tauri/wry uses), not in
SpeakoFlow itself, and it affects many WebKitGTK-based apps. It is tracked
upstream in [tauri#13115](https://github.com/tauri-apps/tauri/issues/13115) and
[wry#544](https://github.com/tauri-apps/wry/issues/544).

Until there's an upstream fix, avoid the pinch-to-zoom gesture inside the app
window. Updating your system's WebKitGTK packages (`webkit2gtk-4.1`) to the
latest version can also help, since newer releases handle the gesture more
gracefully.

</details>

## Roadmap

- Code signing for Windows and macOS
- A wider model catalog and more one-click local models
- More community translations
- Voice-to-text tuned for agentic coding
- Prompt-engineering help: describe what you want to build and get a solid prompt back
- Voice commands: trigger actions and complete tasks by voice

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) to get started, and [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md) if you'd like to help translate the app.

Found a bug or have an idea? Use **Send feedback** in the app (the **?** next to
Settings), or [open an issue](https://github.com/AbhishekBarali/SpeakoFlow/issues).

## License

Released under the [MIT License](LICENSE).

## Credits

SpeakoFlow builds on the dictation core from [Handy](https://github.com/cjpais/Handy)
by CJ Pais, used under the MIT licence. Thanks to CJ for making it open. The
assistant, calls, meetings, screen vision, Generate with Flow, translation,
text-to-speech, and memory layers are SpeakoFlow's own.

Thanks also to [Tauri](https://tauri.app), whisper.cpp, llama.cpp, ONNX Runtime,
[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx), Silero VAD, WeSpeaker,
[Kokoro](https://github.com/hexgrad/kokoro), and Kyutai's Pocket TTS.

<div align="center">

Made by [Abhishek Barali](https://github.com/AbhishekBarali) · [speakoflow.com](https://www.speakoflow.com)

</div>
