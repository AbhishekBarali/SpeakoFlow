# Privacy Policy

**Effective date:** October 3, 2026

SpeakoFlow is a free, open-source desktop voice assistant for Windows, macOS, and
Linux. It is designed to be **local-first**: your voice is processed on your own
computer, and the app works without an account, sign-in, or any tracking.

This policy explains, in plain language, exactly what does and does not leave
your device. If anything here is unclear, please open an issue on our
[GitHub repository](https://github.com/AbhishekBarali/SpeakoFlow).

---

## The short version

- **Nothing is collected automatically.** SpeakoFlow has no analytics, no
  telemetry, no tracking, no advertising, and no user accounts. The only thing
  that ever reaches the developer is feedback you choose to send.
- **Your voice is transcribed on your device** and is never uploaded to us. It
  only leaves your computer if you pick a cloud speech service yourself.
- **The only server we run is the feedback service**, and it receives only what
  you send from the feedback dialog. There is no "SpeakoFlow cloud" for your
  voice, text, or conversations.
- The app only makes a network connection in a few clearly-defined situations,
  described below — and the features that send your content to a third party are
  **off by default** and only ever use a provider **you** choose and configure.

---

## Information SpeakoFlow does **not** collect

We want to be explicit, because "we don't collect data" is easy to say and
harder to prove. SpeakoFlow contains **no** analytics or telemetry libraries and
sends **no** usage data, crash reports, device identifiers, or behavioral data to
the developer. Specifically, we never collect or transmit to ourselves:

- Your voice recordings or transcripts
- The text you dictate or paste
- Your conversations with the AI assistant
- Screenshots or screen contents, unless you attach one to feedback yourself
- Your API keys or credentials
- Your settings, personal memory, or history
- Any identifier, IP-based profile, location, or usage statistics

Apart from feedback you send on purpose, none of this ever reaches us, so there
is nothing to sell or share. Feedback is used only to fix and improve
SpeakoFlow, and it is never sold.

---

## Data that stays on your device

The following is stored **locally on your computer** and never sent to us:

- **Voice recordings and transcripts.** Recordings are saved in the app's local
  recordings folder so you can review or replay them, subject to your retention
  settings. You can delete them at any time from the History screen.
- **Speech-to-text processing.** By default, transcription runs entirely on
  your machine (on your CPU or GPU) using local models, and your audio never
  leaves the device. The exception is a cloud speech service you choose
  yourself (see below).
- **Transcription history** and any "Flow" generations.
- **Personal memory** (an optional feature that is **off by default**): notes the
  assistant learns about how you like to work. It is stored on-device, fully
  viewable and editable by you, and can be exported or erased in
  Settings → Memory. It is never uploaded to us.
- **Settings and preferences.**
- **Feedback drafts.** A message you are writing in the feedback dialog, and the
  email you typed there, are remembered on this device so closing the dialog
  loses nothing. They are only sent when you press Send.
- **API keys and secrets.** Any keys you enter for third-party providers are
  stored in your operating system's secure credential store (the OS keychain /
  Credential Manager / Secret Service), not in plain-text config files, and never
  in logs.

---

## Optional network connections

SpeakoFlow connects to the internet only in the situations below. Some are
standard app maintenance; the ones that send your content are **optional,
off by default, and routed to a provider you choose**, never through us. The one
exception is feedback (section 8), which comes to us, and only when you send
it.

### 1. Automatic update check

To tell you when a new version is available, the app checks a file published on
our GitHub Releases page. This is a normal file download; as with any web
request, the server (GitHub) can see your IP address and general request
metadata. No personal data is sent by SpeakoFlow. You can ignore or disable
update checks in Settings.

### 2. Downloading models and engines

When you choose to download a speech, language, or voice model, SpeakoFlow
fetches the model file from its host — typically Hugging Face, or a mirror on
GitHub. The local AI engine (llama.cpp) may likewise be fetched from GitHub. These
are plain file downloads of publicly available software; no personal data or
content is sent, though the host can see your IP address like any download.

### 3. Cloud speech-to-text (optional, off by default)

If you switch transcription from "On my device" to a cloud provider (for
example ElevenLabs, Deepgram, OpenAI, Groq, or OpenRouter), your recordings are
sent to **that provider**, using **your own API key**, so it can return the
text. It goes directly from your machine to the provider, never through us, and
the provider's own privacy policy applies.

### 4. The AI assistant and AI cleanup (optional)

If you use the assistant or the AI text-cleanup feature, your message (and
conversation context) is sent to the **model provider you have configured**. You
control which provider that is:

- a **fully offline, built-in model** that runs on your machine (nothing leaves
  your device), or
- a **local server** you run (e.g. Ollama or LM Studio), or
- a **cloud provider you choose** (e.g. OpenAI, Anthropic, Azure, OpenRouter,
  and others) using **your own API key**.

SpeakoFlow does not proxy or copy this traffic — it goes directly from your
machine to the provider you selected. What that provider does with the data is
governed by **their** privacy policy.

### 5. Screen vision (optional, off by default)

Screen vision has to be switched on first, separately for quick questions and
for conversations. Once it is on, the assistant may take a screenshot when a
question needs the screen (for example "what's this error?") or when you ask it
to look, and it includes that screenshot with the request to your chosen AI
provider. Every screenshot sent appears in the conversation, so you can always
see when it looked. Like all assistant traffic it goes only to the provider you
configured, and with the offline built-in model it never leaves your device.

### 6. Text-to-speech (optional)

If you have spoken replies enabled, the assistant's answer text is converted to
audio either **locally** (the built-in Kokoro voice) or by a **TTS provider you
choose** (e.g. an OpenAI-compatible service, ElevenLabs, or Azure) using your own
key. Only the answer text needed to synthesize speech is sent to that provider.

### 7. Web search (optional, off by default)

If you enable web search, the assistant can send a search query to the **search
provider you configure** (e.g. Serper, Brave, Tavily, Exa, SerpAPI, or TinyFish) using your
own key, to fetch current information. This is off until you turn it on.

### 8. Feedback (only when you press Send)

The **Send feedback** dialog is the one way information reaches the developer.
Nothing is sent until you press Send, and what is sent is what the dialog shows
you:

- the kind of feedback (bug, idea, or question) and your message;
- your email address, only if you type one, used only to reply to you;
- up to three screenshots, only if you attach them. The app shrinks and
  re-encodes each one first, which also strips hidden metadata such as a
  photo's location;
- the app version, operating system, processor type, and install type, only if
  "Include app version and OS" is ticked. The dialog shows the exact line
  before you send it.

No logs, recordings, transcripts, settings, or identifiers are included.

**Where it goes.** The app sends the report over HTTPS to
`feedback.speakoflow.com`, a small service the developer runs on Cloudflare
Workers. It files the report as an issue in a private GitHub repository that
only the developer can read, and stores attached screenshots as image files in
that repository.

**Your IP address.** Cloudflare sees it, as it would for any website. The
feedback service uses it only to limit how many reports one address can send a
minute. It is not saved with your report or in the service's logs, and it is
never passed to GitHub. Cloudflare's own handling of connections is covered by
its privacy policy.

**How long it is kept, and deleting it.** Reports are kept until the developer
deletes them. To have yours deleted, including its screenshots, send another
message through the dialog asking for it. Use the same email address if you
left one, or describe the report so it can be found.

---

## Third-party providers

The optional cloud features above rely on services you choose and authenticate
with your own credentials. SpeakoFlow is not affiliated with these providers and
has no visibility into the data you exchange with them. When you use such a
service, its own terms and privacy policy apply.

The feedback service is the exception: the developer runs it, on Cloudflare,
and keeps reports in a private GitHub repository. Cloudflare and GitHub process
those reports on the developer's behalf. If you prefer that nothing ever
leaves your machine, you can run SpeakoFlow with only local models and keep every
optional cloud feature disabled.

---

## Children's privacy

SpeakoFlow is a general-purpose productivity tool and is not directed at
children. We do not knowingly collect information from anyone, including
children.

## Security

API keys are stored in your operating system's secure credential store rather
than in plain-text files, and are never written to logs. Because your content is
processed locally and not stored on any server we control, there is no central
database of user data that could be breached. The one exception is feedback you
send, which is kept in a private repository that only the developer can access. As with any software, keep your
operating system and SpeakoFlow up to date.

## Open source

SpeakoFlow is open source under the MIT license. You can inspect exactly what the
app does — including every network call — in the source code at
<https://github.com/AbhishekBarali/SpeakoFlow>.

## Changes to this policy

If this policy changes, we will update the "Effective date" above and publish the
updated version in the repository. Because the project is versioned in Git, the
full history of this document is publicly visible.

## Contact

Questions about privacy? Please open an issue on the
[SpeakoFlow GitHub repository](https://github.com/AbhishekBarali/SpeakoFlow),
or use **Send feedback** in the app for anything you'd rather not post in
public.

---

_SpeakoFlow is a local-first, open-source project. It began as a fork of
[Handy](https://github.com/cjpais/Handy) (MIT)._
