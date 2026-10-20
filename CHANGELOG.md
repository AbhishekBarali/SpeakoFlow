# Changelog

What changed in each SpeakoFlow release. The release page on GitHub carries the
highlights; this file has the full list. Versions before 2.0.0 are described on
their [GitHub releases](https://github.com/AbhishekBarali/SpeakoFlow/releases).

## Unreleased

One speech engine under every local model, and all 69 models in Handy's
catalog to choose from, with filters for streaming, translation and language.
The old Parakeet V2 and V3 and the other ONNX models move to the new engine
with one download. Speech now has one Speech processing setting, long
recordings are split at pauses, and Chinese transcripts can be written in
Simplified or Traditional characters even with the language on Auto.

**Using Parakeet V2 or V3, Canary, Moonshine, SenseVoice, GigaAM or Cohere?**
Those models ran on an engine SpeakoFlow no longer includes. After updating, a
pop-up says so when the app opens: Download new version gets the same model for
the new engine, faster and more stable, and switches to it when it lands, or
you can choose another model. Nothing downloads until you choose. If you close
it, the first dictation tells you again and opens the Models page. Until then a
meeting or a History retranscription says which model
needs its new version instead of starting. Your old files stay until you
delete them, which Delete old files on the Models page does in one click.

### New

**Every model in the catalog.** The Speech to text tab lists all 69 models
instead of five. Recommended ones still come first.

- Filter by Streaming, Translate, or the language you speak. Filters combine,
  and when nothing matches, Clear filters puts the full list back.
- Each row says how many languages the model hears. Click the count for the
  list.
- Every download is pinned to one version of the model and checked against its
  checksum when it finishes. If the download from Hugging Face fails, a copy
  is fetched from Handy's mirror and checked the same way.

**Speech processing**, in Settings → Advanced → Performance: Auto, a
specific graphics card, or Processor (CPU). It replaces the Whisper and ONNX
acceleration settings and is no longer hidden behind Experimental features.
Auto uses the dedicated graphics card when a computer has two. Processor and
Auto carry over from the old Whisper setting. A graphics card picked there
goes back to Auto, because the new engine numbers its devices differently;
pick it again from the list.

**Split long recordings**, on by default in Settings → Advanced → Performance.
A dictation longer than 40 seconds is cut at pauses and transcribed in pieces
of about 30 seconds, which keeps models that aren't built for long audio
accurate. Whisper and streaming models are never cut.

**Chinese script.** Choose Simplified, Traditional, or As transcribed, under
the language picker on the Models page and in the cloud settings. It converts
Chinese only: with Auto language, an English or Japanese dictation is left as
it was. Picking Simplified or Traditional Chinese as the language converts
every dictation, as before.

**Custom words reach more models.** Models that accept a vocabulary now get
your custom words while they transcribe, as Whisper always has. Other models
still get the correction pass afterwards.

**Lower other audio while you dictate.** Settings → Audio → Other audio
replaces the Mute while recording switch with Unchanged, Lower or Mute. Lower
fades your music, videos and calls down while you dictate or ask, and back up
when you stop, by an amount you choose (10–90%, 70% by default). Drag the
slider while something is playing to hear it. It works on Windows, macOS and
Linux (PipeWire, PulseAudio or ALSA). If you had Mute while recording on, it
is now set to Mute.

- If you change the volume yourself during a recording, it stays where you put
  it.
- Plugging in headphones mid-recording doesn't turn them up to your speakers'
  volume when the recording ends.
- Audio that was already muted stays muted. The old switch unmuted it.

**Insights covers the assistant and meetings.** Beside dictation, Insights
now shows questions asked, calls and time on calls, and meetings recorded with
their length and words. The streak counts a day when you used any of them, and
the activity grid opens on all of them so it matches, with each one a click
away. Home shows the same streak.
These totals are kept on their own, so deleting conversations or meetings
does not lower them, and what you already had is counted from the first launch.

**Share your insights.** Share on the Insights page turns your numbers into a
set of cards, one insight each: highlights, words spoken, speaking speed, time
saved, your streak, the assistant, meetings and six months of activity. Flip
through them and copy or save the one you like, or post it to X or LinkedIn:
the share window says how the card goes into the post before the site opens
(X takes it pasted; for LinkedIn it is saved to your Downloads folder for the
photo button). On Windows and macOS, More apps opens the system's share
sheet, which hands the image to WhatsApp, Mail or your phone. Square, Story and Wide
sizes; stories keep clear of Instagram's and Facebook's bars. The cards are
made on your computer; nothing is uploaded.

### Changed

- **A mouse button sits next to a shortcut's keys, not in place of them.**
  With mouse buttons on, pressing one in the shortcut editor used to replace
  the keys it had, so Back turned Ctrl + Win into Back alone. Now the shortcut
  keeps its keys and the button is added as its other keys, and either one
  works. In the editor, "Add another" reads "Add mouse button or keys".
  Turning mouse buttons off no longer moves any shortcut back to its default
  keys: the button just stops working until it is turned back on. A shortcut
  that had a button as its only keys gets its keys back on update, with the
  button kept next to them.
- **Streaming follows the model, not the overlay style.** A streaming model
  such as Parakeet or Nemotron now streams with the Minimal pill too, so its
  text is ready the moment you stop. The overlay style only decides whether
  you see the running text.
- **Your language choice is kept** when you switch models. An English-only
  model uses English, and switching back to a multilingual model brings your
  choice back. Auto is only offered for models that can detect the language.
- **Streaming models keep the pauses between sentences** (up to 1.65 seconds
  of each pause instead of 0.45), so they no longer run sentences together.
  Stop after silence still counts from the same moment.
- **Whisper `.bin` files** you already downloaded run on the new engine and
  are marked Older. Their newer versions are in the catalog.
- **The x64 build on a Windows on ARM PC** keeps speech on the processor and
  offers only Processor under Speech processing, because the graphics path
  is not reliable under emulation.

### Fixed

- **Mute while recording could leave the computer muted** after a very short
  dictation, after Esc with the always-on microphone, when a recording failed
  to start, or when you quit mid-dictation. Nothing unmuted it until the next
  dictation ended.
- **A graphics driver crash could take the whole app down** (#82), or leave a
  dictation stuck on "Transcribing…". Speech models now run in a separate
  process: if it crashes or stops responding, SpeakoFlow switches speech to
  the processor for the rest of the session, finishes the dictation there, and
  stays open. If even that fails, the pill offers Try again. The old ONNX
  models, which ran on DirectML, also went with their engine.
- **Esc during "Transcribing…"** now stops the transcription and offers Undo,
  instead of reporting a failure. The model stays loaded for the next
  dictation.
- **A paste could insert what you had copied before** instead of the
  dictation, in a busy browser tab or editor. On Windows and macOS your
  clipboard now goes back only after the app you dictated into has read the
  dictation, and auto-submit's Enter waits for the same moment.
- **Words could repeat** when you started speaking again right after a short
  pause.
- **Esc could stay taken from every app** after a very short recording, until
  the next one.
- **While the speech model was still loading, the pill said "Transcribing…"**
  and its progress bar ran for the whole load. It now says "Loading model…".
- **A model set to unload immediately stayed loaded** after a recording with
  no speech in it.
- **History could list a dictation twice.**
- **A cleanup cut off at the model's output limit was pasted half-finished.**
  The raw transcript is pasted instead.
- **Removing a filler at the start of a sentence lowercased the next word.**
  "Um, so I think" came out as "so I think".
- **Linux: memory grew with every dictation.**
- **Switching models or Speech processing during a dictation broke it.** The
  dictation lost its live text and then failed with "Model is not loaded". The
  switch now waits until the recording ends. The same goes for a model set to
  unload immediately after another transcription.
- **Live text could stop partway through a dictation** that started right
  after another one was cancelled, and the recording was then transcribed a
  second time from the start.
- **Esc pressed just after letting go of the keys** still transcribed the
  whole recording before dismissing it.
- **AI cleanup never ran again for some reasoning models** (LM Studio without
  its reasoning parser, older Ollama) once one answer was cut off while the
  model was still thinking.
- **A call opened from History or Meetings with an old ONNX model selected**
  failed on every turn with "Model is not loaded". It now says the model needs
  its new version.
- **"Loading model…" could stay on the pill**, and a "Transcribing…" pill could
  stay up until the next dictation.
- **Windows: auto-submit came a second late** on computers with a clipboard
  manager, remote desktop or VM client, and pasting after copying a large
  Office selection could stall.
- **An ask started right after a dictation** could send that dictation's
  Enter while the ask keys were held (Ctrl+Enter), or on macOS leave the
  dictation on the clipboard in place of yours.
- **Quitting right after a dictation** could leave the dictation on the
  clipboard instead of what you had copied.
- **A spoken reply could stop with an error** when a sentence held only an
  ElevenLabs audio tag such as `[laughs]`.
- **The tray's model menu listed models that cannot be used**: retired ONNX
  models and the assistant's, cleanup's and voice models.
- **Removing "Um," before a name like iPhone** capitalised it to "IPhone".
- **A processor without FMA3** crashed its first speech worker every session
  before falling back to the processor.
- **macOS: `pkill -USR2 -n speakoflow` did nothing** while a speech model was
  loaded, because it reached the speech worker instead of the app.
- **The main window had no title** in the taskbar preview, Alt+Tab, Task
  Manager and screen readers.

## 2.1.0

Hands-free dictation, Hold or Tap set per shortcut, smart spacing between
dictations, Amazon Transcribe, and optional meeting notes in the meeting's own
language. Local dictation no longer stalls when the computer is busy, and
updates from here on download several times faster.

**On 2.0.0 or 2.0.1?** The app offers this update by itself. On 1.4 or
earlier, download 2.1.0 once and install it over the old version.

### New

**Hands-free dictation.** While you hold the dictation keys, press Space and
let go. The dictation keeps listening until you press the dictation keys again,
click ✓ on the pill, or cancel with Esc or ✕.

- The default is Left Ctrl + Left Win + Space on Windows and Fn + Space on a
  Mac. Pick another key, or turn it off, under Hands-free in Settings →
  Shortcuts.
- A hands-free dictation has a ring around the pill and the live card. Hover
  it for ✕ and ✓.
- The first three times you hold the dictation keys, the pill shows the Space
  key as a reminder.
- It needs the SpeakoFlow Keys engine and dictation keys that are modifiers
  only. Linux's default Ctrl + Space has a main key of its own, so on Linux
  turn off Hold to talk instead, which makes every dictation hands-free.
- Space no longer reaches Windows while you hold Ctrl + Win, so a dictation
  can no longer switch your input language by accident.

**Hold or Tap, set per shortcut.** Dictation and the assistant each have their
own switch in Settings → Shortcuts: Hold to talk and Hold to ask. You can now
hold to dictate and tap to ask. Existing installs keep what they had, because
the ask starts with the old shared setting.

**Smart spacing**, on by default. Two dictations into the same field no longer
run together. SpeakoFlow reads the few characters around the cursor and adds a
space only where one is needed.

- It lowercases the first word when you continue a sentence (Fix capital
  letters). "I", acronyms, names and dictionary words keep their capitals, and
  so does German and Turkish text.
- It removes the period SpeakoFlow added when you paused mid-sentence, but only
  when the field still ends with exactly that paste, and only in the window it
  was read from (Join sentences you paused in).
- No spaces next to Chinese, Japanese, Thai and other scripts written without
  them. Korean gets spaces.
- Password fields and terminals are never read. Nothing that is read is logged
  or kept, apart from the end of SpeakoFlow's own last paste.
- Where a field can't be read, a dictation that continues your last one in the
  same window still gets its space. Otherwise it pastes exactly as before.
- Chrome's address bar trims pasted spaces, so there the space is typed.
- With Smart spacing on, Append trailing space only applies where a field
  can't be read. Turn Smart spacing off in Settings → Dictation → Typing and
  pasting to get the old paste back.

**Amazon Transcribe** for cloud speech-to-text, streaming as you speak.

- Enter an access key ID and secret access key, plus a session token for
  temporary credentials, and pick the region closest to you.
- Auto language picks among five candidates, starting with the app's language.
  A language Amazon can't stream in your region falls back to your local model
  instead of going to another language's model.
- Amazon has no batch endpoint, so History, calls and meetings send recordings
  again at speaking speed. Finishing takes as long as the recording did.

**More than one set of keys per shortcut.** Click a shortcut's keys, then Add
another. Either set starts the shortcut, and a recording started with one
stops with the other.

**Mouse buttons as shortcuts** (experimental, Settings → Advanced →
Experimental). The middle, Back and Forward buttons can be a shortcut, alone or
with modifier keys. Left and right click are never accepted. A bound button
stops doing its usual job. Works on the SpeakoFlow Keys engine only, and on
Linux only the middle button can be used.

**Stop after silence.** A tapped or hands-free dictation can end itself after
3, 5, 10, 20 or 30 seconds without speech, exactly as a second press would.
Off by default, in Settings → Shortcuts. A held recording, the assistant, a
call and a meeting are never stopped this way. In the last two seconds a thin
line on the pill counts down.

**Skip cleanup for short dictations.** Pick a length, and any dictation shorter
than that is pasted as transcribed, without waiting for the cleanup model. It
starts at 15 words, about one spoken sentence, and is on by default.

- Set your own length (2 to 50 words), or turn it off, on the AI cleanup page.
- Only while cleanup runs on every dictation. The dictate-and-clean-up
  shortcut always cleans, however short.
- Words in Chinese, Japanese and Thai are estimated from the character count.

**Meetings.**

- Write notes when a call ends, on the Meetings page, can be turned off. The
  transcript still gets its speaker labels, and notes for one meeting can still
  be written from its transcript.
- Notes language. Same as the meeting is the default, so a German meeting gets
  German notes. You can also pick one of 37 languages, and regenerate an old
  meeting's notes in another.
- A Copy button for the transcript, on the meeting page, in the live recorder
  and in the floating pill. Renamed speakers are copied under their new names.
- Note headings no longer carry the template's descriptions.

**Search depth that changes the search.** Quick, Standard and Deep now set how
many searches the assistant runs, how much text it reads from each source, and
how long it waits, and Tavily and Exa search deeper on Deep.

**Faster updates.** The app downloads an update on 16 connections and checks
its signature before installing it. From Kathmandu that took a 38 MB update
from about three and a half minutes to 42 seconds. It only helps updates
installed by 2.1.0 or later, so this update itself downloads at the old speed.

- When the fast download can't be used, or what it fetched doesn't match the
  signature, the update downloads the old way on one connection. A
  connection that stops sending for a minute ends with an error you can retry
  instead of a progress bar that never moves.
- Download installer, in Settings → About, uses the same fast download.

**Linux: a white tray icon** that matches the rest of the panel. A Color tray
icon switch in Settings → General brings the colour logo back for a light
panel, with teal recording and transcribing icons instead of pink.

**Shortcut rows** on Home and in Settings show a name, an (i) that says how the
shortcut is pressed, and the keys.

### Fixed

- **Local dictation could sit on "Transcribing…" for 15 seconds or more** while
  the computer was busy, for example during a large build. Parakeet and
  Nemotron on a graphics card now decode on one processor thread, and a
  dictation that is still behind two seconds after you stop is transcribed in
  one pass instead.
- **Parakeet Unified dropped the last word** of a dictation.
- **Web search stopped working on Azure GPT-6** with Thinking above Off. The
  assistant now keeps its tools and turns reasoning off for those requests.
- **Linux AppImage: built-in AI cleanup never started** (#68). Thanks to
  @HananSolves for the fix.
- **Meetings with notes off stayed on "Processing".**
- **Meetings: a long transcript could show rows twice** right after the call,
  with the old speaker labels.
- **Meetings: the notes-language list closed** as soon as you scrolled it.
- **After updating, the Hands-free row had no key** until the next launch.
- **The update card showed a literal `<br />`** in the release notes.
- **A debug log could contain cloud transcription credentials.** With
  `--debug`, the WebSocket library logged every handshake in full.

### Downloads

| Platform             | File                                                                |
| -------------------- | ------------------------------------------------------------------- |
| Windows              | `SpeakoFlow_2.1.0_x64-setup.exe` (or the `.msi`)                    |
| macOS, Apple silicon | `SpeakoFlow_2.1.0_aarch64.dmg`                                      |
| macOS, Intel         | `SpeakoFlow_2.1.0_x64.dmg`                                          |
| Linux x86_64         | `SpeakoFlow_2.1.0_amd64.deb` or `SpeakoFlow_2.1.0_amd64.AppImage`   |
| Linux ARM64          | `SpeakoFlow_2.1.0_arm64.deb` or `SpeakoFlow_2.1.0_aarch64.AppImage` |

Every commit: [v2.0.1...v2.1.0](https://github.com/AbhishekBarali/SpeakoFlow/compare/v2.0.1...v2.1.0)

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
