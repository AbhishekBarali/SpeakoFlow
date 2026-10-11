# Changelog

What changed in each SpeakoFlow release. The release page on GitHub carries the
highlights; this file has the full list. Versions before 2.0.0 are described on
their [GitHub releases](https://github.com/AbhishekBarali/SpeakoFlow/releases).

## 2.0.1

A smaller update to 2.0: a connection test for AI cleanup, a Thinking setting
for reasoning models, and fixes for writing styles, settings that reset after a
restart, the on-device AI engine, the macOS menu bar, and the assistant on KDE
Wayland.

**On 2.0.0?** The app offers this update by itself. On 1.4 or earlier,
download 2.0.1 once and install it over the old version.

### New

**Test connection for AI cleanup.** The AI cleanup card in Models now ends with
a Test connection row, the same as the Assistant's. It sends one short request
to the provider and model that clean up your dictations, so a wrong key, an
empty balance, or a model the server does not offer shows up here instead of as
a dictation pasted without cleanup.

- A missing setting is named, for example "No API key is set for Azure OpenAI".
- When cleanup borrows the assistant's model, the result says so.
- A key you have just pasted is saved before the test runs, so the new key is
  the one tested.
- With a model on this computer, the first test starts the model, so it can
  take a moment.

**Thinking.** Choose how much a reasoning model thinks before it answers: Off,
Low, Medium, or High. It is set separately for AI cleanup (Models, AI cleanup,
Performance) and for the assistant (under its model picker).

- Off is the default and works exactly as 2.0.0 did.
- When a provider refuses a level, the request is retried at a lower one
  instead of failing.
- Anthropic, Apple Intelligence, and models on this computer have no such
  control, so the setting is disabled for them and says why.

### Fixed

- **Writing styles made no difference.** Formal, Professional, Concise, and the
  other styles now change the cleaned text, while keeping every fact, name,
  number, and request. With no style picked, cleanup is exactly as before.
  SpeakoFlow Mini and other fine-tuned models mostly ignore styles, so the
  Cleanup page now says so when one is in use and offers Change model.
- **Settings could reset to defaults after a restart or shutdown** (reported on
  macOS). Every save now writes a new file and swaps it in, so an interrupted
  save can no longer damage your settings. If the file is ever damaged anyway,
  SpeakoFlow keeps the damaged copy aside, restores your settings from a
  backup, and tells you that recent changes may be missing. A new install gets
  its first backup on its first save.
- **The on-device AI could crash while loading Gemma 4** on computers with two
  graphics cards, or with the model only partly on the graphics card. The
  built-in engine is updated to llama.cpp b11429, which fixes this.
  - The first time you use on-device AI after updating, the new engine
    downloads once in the background, with a progress bar.
  - The new engine is downloaded and checked before the old one is removed.
    If you are offline or the download fails, the current engine keeps working
    and the update is tried again next time.
  - The assistant and cleanup can no longer install the engine over each other
    at the same moment.
- **AI cleanup with gpt-oss pasted the transcript unchanged.** It now cleans up
  as expected on Azure, Groq, and OpenRouter.
- **Text you never said could appear after a short dictation.** With the
  built-in engine, some models answered short dictations such as "Thank you."
  with the cleanup prompt's own examples. The prompt is now sent as a separate
  instruction, and output that copies the prompt is discarded so your own words
  are pasted instead. A reply that is only punctuation, such as "...", is no
  longer pasted either.

### Linux

- **The assistant on KDE Plasma Wayland** can see the text you selected, and
  its panel and the conversation bar respond to clicks again. The selection is
  read from the desktop's own selection on Wayland desktops that support it,
  and falls back to the old way on those that do not. On Wayland, the clear
  margin around the panel also catches clicks now.

### macOS

- **The menu bar icon** shows the SpeakoFlow logo instead of a plain rounded
  square. The recording and transcribing icons still follow a light or dark
  menu bar.

### Downloads

| Platform             | File                                                                |
| -------------------- | ------------------------------------------------------------------- |
| Windows              | `SpeakoFlow_2.0.1_x64-setup.exe` (or the `.msi`)                    |
| macOS, Apple silicon | `SpeakoFlow_2.0.1_aarch64.dmg`                                      |
| macOS, Intel         | `SpeakoFlow_2.0.1_x64.dmg`                                          |
| Linux x86_64         | `SpeakoFlow_2.0.1_amd64.deb` or `SpeakoFlow_2.0.1_amd64.AppImage`   |
| Linux ARM64          | `SpeakoFlow_2.0.1_arm64.deb` or `SpeakoFlow_2.0.1_aarch64.AppImage` |

Every commit: [v2.0.0...v2.0.1](https://github.com/AbhishekBarali/SpeakoFlow/compare/v2.0.0...v2.0.1)

## 2.0.0

The largest release so far: 115 commits since 1.4.0. The app was rebuilt
around three things you do with your voice (dictate, ask, talk it through),
and it gained meetings, cloud transcription, undo, reminders, and updates that
install themselves.

**Coming from 1.4 or earlier?** Those versions cannot update themselves to 2.0.
Download 2.0 once and install it over the old one. Settings, history, models,
and API keys are kept. The new first-run setup shows once, for everyone.

### New

**Conversations.** A hands-free, spoken back-and-forth with the assistant, on
its own shortcut (Left Ctrl + Left Alt + C on Windows). It answers out loud,
and you can talk over it to cut in.

- Esc stops a reply without ending the conversation. Pressing the shortcut
  again, closing the panel, or End hangs up, and hanging up closes the panel.
- Dictating during a conversation pauses it instead of ending it. The
  conversation listens again once your text is pasted, and a muted
  conversation stays muted.
- One thought spoken with a pause in the middle stays one question. The words
  before the pause are no longer dropped.
- The microphone and the assistant's voice have separate mute switches.
- History's Continue opens a past chat as a conversation.

**Ask about what you selected.** Select text in any app, hold the ask keys, and
say what you want done with it. The answer streams into a card from the first
word. Copy it, insert it at the cursor, or replace the selection with it.

- Every quick ask starts fresh and closes when you are done with it.
- The card opens in the same place every time, on the display under your
  cursor, and never resizes or jumps while it writes.
- On Windows the card does not take the keyboard from the app you were in, so
  Replace and Insert paste back into the same field.

**Meetings** (beta). Record your microphone and the computer's audio as two
streams, transcribed live, so the transcript knows which words were yours.
No bot joins the call, and it works with any meeting app.

- Notes are written when the call ends, from a template: General, Standup,
  One-on-one, Interview, or Action items. Other voices are labelled
  Speaker 1, Speaker 2, and so on.
- Ask questions about a meeting afterwards, or choose Discuss this meeting to
  talk it over in a conversation.
- On Windows it can offer to record when it notices a call.
- Quitting during a meeting finishes the recording, and audio left behind by a
  crash is recovered on the next launch.

**Cloud speech-to-text.** Transcribe on a hosted service instead of a local
model: ElevenLabs, Deepgram, OpenAI, Groq, Mistral (Voxtral), Azure AI Speech,
OpenRouter, or any OpenAI-compatible server.

- ElevenLabs Scribe v2 Realtime and Deepgram show text while you speak.
- Your Dictionary words are sent to the provider as recognition hints.
- An incomplete cloud setup falls back to the local model instead of failing.
- Recordings upload as compressed MP3, about five times smaller than WAV, so a
  long dictation on slow Wi-Fi no longer times out. The timeout also grows
  with the length of the recording.
- The first dictation after a quiet spell is about 1.4 seconds faster: the app
  warms the provider's route while you are still talking.
- OpenAI and Groq can translate speech into English text.

**Undo.** Cancelled a recording by accident? The pill offers Undo for a few
seconds. A failed transcription offers Try again. History keeps dismissed
recordings, and Recover finishes one later.

**Reminders.** "Remind me to send the invoice in twenty minutes." The assistant
sets it. Reminders survive a restart, anything due while the app was closed
fires on the next launch, and the popup appears without taking your keyboard.
Done or Snooze from the popup.

**More voices.** Kitten (three sizes), Pocket TTS (14 voices), and Supertonic
run on the processor, and Kokoro can too, for machines whose graphics card
cannot run it. Nothing ships in the installer. The engine downloads with the
first voice you pick.

- ElevenLabs audio tags. Eleven v3 and v4 voices can laugh, whisper, or sigh
  when the reply calls for it, at the intensity you choose.
- An expressiveness setting for ElevenLabs voices.
- The assistant's voice has its own volume control.

**Insights.** Words dictated, speaking speed, time saved compared with typing
at 40 words a minute, and six months of activity with streaks.

**Updates install themselves.** SpeakoFlow checks in the background, shows new
versions at the bottom of the sidebar, and installs them from Settings → About
after verifying each one against the project's signing key. Installs that
cannot replace themselves (AUR, a build from source) get a download instead.

**Send feedback from the app.** The ? beside Settings, or the tray menu. Attach
up to three screenshots. It sends exactly what the dialog shows and nothing
else, to a private tracker only the developer reads.

**Learn words I correct** (Windows). Fix a misheard word after it is pasted and
SpeakoFlow adds it to the Dictionary.

**AI cleanup on the dictation shortcut.** Cleanup can run on every dictation, or
keep its own shortcut (the dictation keys plus Shift).

**Azure AI Speech** is a cloud transcription provider. **OpenRouter** reaches
about 20 transcription models with one key.

**Command line.** `--toggle-assistant` asks the assistant and `--toggle-call`
starts or ends a conversation, for window managers and desktop shortcuts.

### Changed

**The whole app is redesigned.** Pages are now the things you do: Home,
History, Assistant, Meetings, AI cleanup, Dictionary, and Models. Settings is a
separate dialog for app-wide preferences. Models are grouped by the job they
do (speech to text, cleanup, assistant, voice), and every page agrees on which
model is doing each job.

- A new first-run setup: pick one of five speech models, then a short tour
  shows the three shortcuts in action while the model downloads.
- A light theme that reads as paper, and a quieter dark theme.
- Assistant answers are easier to read: a capped line length, softer body
  text, and proper styling for lists, code, and links.
- Larger text size options for the assistant.
- "Calls" are now called conversations.

**One shortcut scheme on every platform.** Two keys to dictate, two to ask.
Add Shift to dictate and clean up, add C to start a conversation, Esc to
cancel.

| Action               | Windows                      | macOS         | Linux                |
| -------------------- | ---------------------------- | ------------- | -------------------- |
| Dictate              | Left Ctrl + Left Win         | Fn            | Ctrl + Space         |
| Ask                  | Left Ctrl + Left Alt         | Fn + Ctrl     | Ctrl + Alt + Space   |
| Conversation         | Left Ctrl + Left Alt + C     | Fn + Ctrl + C | Ctrl + Alt + C       |
| Dictate and clean up | Left Ctrl + Left Win + Shift | Fn + Shift    | Ctrl + Shift + Space |

- Shortcuts that share keys no longer trip each other. Pressing Ctrl + Alt + C
  starts a conversation without flashing the ask on the way, however slowly
  you press the C.
- On Windows the keys you hold are replayed to other apps when they turn out
  not to be a SpeakoFlow shortcut, so Ctrl + Alt shortcuts in VS Code and
  AltGr characters on a German layout still work.
- Any shortcut still on an old default moves to the new one. On a Mac, an
  install on the old Option-based defaults keeps them, because the Fn key
  needs **Press 🌐 key to** set to **Do Nothing** first. Home says when it is
  not.
- Cancel and the conversation shortcut can be turned off.
- The shortcut editor says which shortcut already uses the keys you pressed.

**Screen vision is two switches,** one for the quick ask and one for
conversations, both off by default. When on, the model decides per question
whether it needs to look, and a selection counts as the "this" in "what does
this mean", so it no longer screenshots the desktop to answer about three
selected words. The old Manual mode and its snip tool are gone.

**The recording pill.** The live card shows text as you speak, scrolls back,
lets you select a phrase, and has a copy button, all without ever taking focus
from the app you are dictating into. It follows your cursor between displays.
The working indicator is one forward-only fill timed from your own past
dictations, instead of a looping sweep. Finished text stays on screen for one
second.

**History** separates quick asks from conversations and shows less noise per
row. Recording retention now tells you how many recordings a new setting will
delete before it deletes them, counts months as calendar months, and applies
while the app stays open instead of only at launch.

**All 20 interface languages are complete,** each with its own plural forms.

### Faster and lighter

- Local models are released when nothing uses them. Each local engine is
  warmed up once rather than on every recording, and its prompt cache is
  capped.
- The microphone stream is parked between recordings, and slow device checks
  no longer delay the start of a recording.
- Speech engine backends load on first use instead of at launch.
- Cloud transcription reuses its connection between dictations, measured at
  about 27% faster.
- History pages through assistant conversations instead of loading them all.
- Settings sliders save when you let go rather than on every tick.

### Fixed

- **Pasting into the app's own windows did nothing.** Dictating into a field in
  the main window now inserts the text at the caret.
- **The paste could land in the wrong window** when a SpeakoFlow window took
  focus during the recording. The text now goes to the window you started in.
- **The recording pill could hide behind other windows** mid-recording on
  Windows. It re-asserts itself while it is up.
- **A microphone dropout ended the recording.** It now recovers.
- **Cleanup silently did nothing on some cloud models,** and the log now says
  why a provider refused.
- **The local assistant spent its answer on hidden thinking.**
- **A model that cannot read images** no longer fails the turn when a
  screenshot is attached.
- **An assistant chat could break permanently** after a reply was stopped
  before its first word. Strict chat templates rejected two questions in a
  row, and every later message failed the same way. Chats already stuck this
  way work again.
- **An error sent inside a streaming reply** (how AWS Bedrock reports one) was
  shown as silence. It is now shown as an error.
- **Memory** keeps your own edits when it learns something new.
- **A crash could leave an engine process running.** The next launch cleans it
  up.
- **The app no longer needs a Vulkan runtime to start,** and transient cloud
  failures are retried.
- **Filler removal** no longer leaves a lowercase sentence start or a doubled
  comma behind.
- **Transcripts and model output are kept out of release logs.**

### Linux

- The app no longer aborts at launch.
- The assistant panel stays clickable on native Wayland.
- Conversations can open the microphone.
- Home says when shortcuts cannot work and gives the exact fix, and on Wayland
  a Set up button lists the command for each action to bind in your desktop's
  own settings.
- A user-set `WEBKIT_DISABLE_DMABUF_RENDERER` is respected.
- An Arch Linux build is tested in CI.

### macOS

- Unsigned builds are signed with one stable identity, so macOS remembers
  permissions across updates more reliably.
- Permission screens have a Reset permission button for when macOS shows
  SpeakoFlow as allowed but no longer honours it.

### Downloads

| Platform             | File                                                                |
| -------------------- | ------------------------------------------------------------------- |
| Windows              | `SpeakoFlow_2.0.0_x64-setup.exe` (or the `.msi`)                    |
| macOS, Apple silicon | `SpeakoFlow_2.0.0_aarch64.dmg`                                      |
| macOS, Intel         | `SpeakoFlow_2.0.0_x64.dmg`                                          |
| Linux x86_64         | `SpeakoFlow_2.0.0_amd64.deb` or `SpeakoFlow_2.0.0_amd64.AppImage`   |
| Linux ARM64          | `SpeakoFlow_2.0.0_arm64.deb` or `SpeakoFlow_2.0.0_aarch64.AppImage` |

Every commit: [v1.4.0...v2.0.0](https://github.com/AbhishekBarali/SpeakoFlow/compare/v1.4.0...v2.0.0)
