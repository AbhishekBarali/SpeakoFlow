# SpeakoFlow DEV Article and Publishing Guide

## Will this DEV post work?

It has a real chance, but not as a normal product announcement.

A post called “I launched my open-source voice assistant” asks strangers to care about the product before they care about the problem. A better post gives developers a useful surprise:

> I combined local speech-to-text, screen vision, an LLM, and text-to-speech. The models were the easy part. Making them feel like one instant interaction was the hard part.

That angle has four things a good DEV post needs:

1. **A curiosity gap.** If the models were easy, what caused the trouble?
2. **A real technical story.** You can show actual decisions from the code, not generic AI advice.
3. **Visual proof.** The GIF immediately proves that the project exists and works.
4. **A human reason for building it.** You were studying alone, switching between your work and a chatbot, and got tired of repeating context.

What will hurt the post:

- Opening with a feature list
- Asking readers to download before giving them value
- Using the Product Hunt ranking as the main image
- Explaining every feature in SpeakoFlow
- Sounding like a company press release
- Pretending you built the original dictation core from scratch

The goal of this article is not to squeeze out immediate downloads. The goal is to make developers think, “That is a clever solution,” then inspect the repository, leave a comment, save the article, or share it.

## Recommended title

**I Built a Local Voice Assistant That Can See My Screen. The Models Were the Easy Part.**

This is stronger than the previous title because it creates a question in the reader’s head. It also states the unusual capability before mentioning implementation details.

Alternative titles to test later:

- Making a Local Voice Assistant Feel Instant Nearly Broke My Brain
- I Connected Whisper, Screen Vision, llama.cpp, and TTS. Then the Real Work Started.
- The Weird Engineering Problems Behind My Local Voice Assistant
- How I Made a Local Voice Assistant Feel Like One Instant Interaction

## DEV description

> The strange engineering problems behind combining local speech-to-text, screen vision, llama.cpp, streaming responses, and text-to-speech in one desktop interaction.

## Tags

```text
rust, tauri, ai, opensource
```

DEV allows four tags. These are more useful than spending one on a branded tag nobody follows.

## Canonical URL

If you publish the same article on your own blog first, set that exact article URL as the DEV canonical URL. If DEV is the original publication, leave the canonical field blank.

Do not use the website homepage as the canonical URL.

## Animated cover

Use the real product demo as the DEV cover:

```text
assets/darko.gif
```

It is a 789 by 450 GIF and about 1.41 MB. Upload it through DEV’s **Add a cover image** control. DEV may crop the animation to its cover ratio, so preview the title card before publishing. If the recording overlay is cut off, use `assets/hero.gif` as the alternate cover.

Do not use the generated green illustration. If it is already in your DEV draft, remove it manually before uploading the GIF cover.

Because `darko.gif` is now the cover, the article body uses the separate `hero.gif` product demonstration.

---

# Article draft

I wanted one hotkey.

Press it, talk, let the assistant look at my screen, and get an answer without opening six tabs or explaining what was already in front of me.

Simple, right?

In my head, the architecture looked like this:

```text
microphone → local AI magic → useful answer
```

In the code, it looked more like this:

```text
global shortcut
    → audio recorder
    → voice activity detection
    → speech-to-text model
    → screen capture
    → image compression
    → local LLM server
    → streaming UI
    → text-to-speech
    → interruptible text-to-speech playback
```

The models were the easy part.

Getting all of them to behave like one fast, calm assistant was where things became interesting.

![SpeakoFlow showing live dictation in meeting notes](https://raw.githubusercontent.com/AbhishekBarali/SpeakoFlow/main/assets/hero.gif)

I built this while studying alone for exams. I kept bouncing between my work and a chatbot tab, copying context, asking a question, copying the answer back, and slowly losing the will to continue.

I was also paying for dictation software because talking is faster than typing. The dictation worked, but it stopped at text. It could hear me. It could not help me.

So I started building the thing I actually wanted: a voice layer over my desktop.

SpeakoFlow began as a fork of [Handy](https://github.com/cjpais/Handy), which gave me a solid local dictation foundation. I did not reinvent that part and I want to be clear about the credit. I built the assistant, screen vision, spoken answers, translation, memory, and the orchestration that turns those pieces into one experience.

Here are the problems that surprised me most.

## Why a fully local pipeline still felt slow

My first mental model was completely sequential:

1. Record the user.
2. Stop recording.
3. Load the transcription model.
4. Transcribe the audio.
5. Capture the screen.
6. Start the LLM.
7. Generate an answer.
8. Start text-to-speech.

Nothing about that pipeline was technically wrong.

It just felt terrible.

[![A skeleton sitting at a table and waiting for the pipeline to finish](https://media.giphy.com/media/QZyBvNVaMbIZ9yadec/giphy.gif)](https://giphy.com/gifs/waiting-loud-polish-QZyBvNVaMbIZ9yadec)

A fully local pipeline can still feel slow when every stage runs sequentially. The critical path matters more than the number of components running on-device.

The biggest latency improvement did not come from switching models. It came from refusing to wait.

## Hide cold starts inside the recording window

A person is going to spend a few seconds speaking. That time is free latency budget.

As soon as assistant recording begins, SpeakoFlow can start doing useful work in parallel:

```rust
// Simplified version of the real flow
fn recording_started() {
    initiate_transcription_model_load();
    preload_vad();

    spawn(prewarm_local_llm());
    spawn(capture_screen_if_armed());

    stop_previous_spoken_answer();
    show_listening_state();
}
```

While the user says, “Can you explain the error in this terminal?”, the app is already loading the local model and preparing the visual context.

By the time speech-to-text finishes, much of the cold-start work may already be gone.

The “Hey Flow” prewarm path is narrower. It runs only when Flow is enabled, a streaming transcription model produces live text, the activation phrase leads the committed transcript, and the selected assistant provider is the built-in engine. Batch transcription models emit no live text, and cloud providers have no local model to load.

This avoids waking a multi-gigabyte LLM during ordinary dictation while still overlapping startup with genuine assistant requests.

The lesson was simple:

> In a voice interface, the user’s speaking time is part of your latency budget.

### The critical-path timeline

I avoid quoting one universal latency number because cold starts vary by model, hardware, and accelerator. The more useful measurement is which work still blocks the first visible token.

| Phase                      | Work started                                                                                                                                      | Why it matters                                                  |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| Assistant recording begins | Start the transcription model load, preload VAD, stop old TTS, and prewarm the built-in LLM when selected unless its unload policy is Immediately | Removes setup work from the post-recording path                 |
| While the user speaks      | Continue model loading and optionally capture the screen immediately                                                                              | Overlaps cold starts with time the user already spends speaking |
| Recording stops            | Finalize audio, transcribe, and capture on send if that timing is selected                                                                        | Blocks only on work that could not safely start earlier         |
| Generation begins          | Stream SSE deltas from Rust to the React panel                                                                                                    | Shows the first text before the complete response exists        |
| Generation completes       | Sanitize a separate speech copy and start playback with a cancellation epoch                                                                      | Keeps formatted text on screen and prevents stale audio         |

This does not make model inference free. It removes avoidable idle gaps between stages.

## Global hotkeys need a serialized state machine

A normal button usually emits one event. A global shortcut can emit repeated press events, delayed releases, or no release event at all.

It can repeat. A release event can arrive late. The user can press it again while transcription is still running. A hands-free recording can outlive the key that started it. A stale timeout can wake up and try to stop a completely different recording.

[![Donald Glover walks into a room and discovers complete chaos](https://media.giphy.com/media/137TKgM3d2XQjK/giphy.gif)](https://giphy.com/gifs/community-troy-chaos-137TKgM3d2XQjK)

I eventually routed recording through one coordinator with explicit states:

```text
Idle → Recording → Processing → Idle
```

The coordinator owns start, stop, cancel, commit, and hands-free transitions. It rejects duplicate triggers and tags safety timers so an old timer cannot stop a newer recording.

Serializing these transitions is the important part. Every lifecycle change goes through one coordinator, so duplicate key events cannot race the asynchronous transcription and paste pipeline.

Voice software has very little room for ambiguity. If the app starts twice, ignores a sentence, or keeps recording after the user stops, confidence disappears immediately.

## Screen vision is an image-budgeting problem

Screen vision sounds easy:

```rust
let screenshot = capture_screen();
send_to_model(screenshot);
```

That works beautifully until the screenshot hits a strict API gateway, a small local context window, or a multi-monitor setup.

SpeakoFlow captures the monitor under the mouse cursor, with the primary monitor as a fallback. The cursor is usually the best clue about where the user is actually working.

Then the image goes through a provider-specific compression ladder.

The current code has different profiles for:

- Strict gateways such as Azure
- Local llama.cpp vision models
- Cloud models with larger payload limits

Each profile tries combinations of image dimensions and JPEG quality until the base64 payload fits its target.

```rust
for (max_dimension, quality) in profile.ladder {
    let jpeg = resize_and_encode(&image, max_dimension, quality);

    if base64_size(&jpeg) <= profile.target_bytes {
        return as_data_url(jpeg);
    }
}
```

If no rung fits the target, the encoder keeps the smallest attempt rather than failing the capture.

The rough targets currently range from about 48 KB for strict gateways to 200 KB for local vision and 384 KB for more generous cloud providers.

For local models, this is not only about transfer speed. A screenshot consumes vision tokens. Make it too large and it crowds the conversation out of the context window. Make it too small and the model cannot read the error message you wanted help with.

That makes screen capture a constrained encoding problem, not a simple screenshot call.

<!-- OPTIONAL IMAGE: Add any product screenshot you choose here. A real screen-aware assistant example would fit this section best. -->

## Screen access and capture timing are separate decisions

SpeakoFlow separates permission from timing. They are not the same setting.

**1. Who decides whether a screen capture is allowed**

- **Off:** screen capture is disabled.
- **Manual:** the user explicitly arms screen sharing for the session, or attaches an image for a specific turn.
- **Agent decides:** the request starts without an image and the model receives a `capture_screen` tool. It may call that tool only when the question genuinely depends on visible context, and it can call it at most once per message.

**2. When a manually armed voice turn captures**

- **Immediate:** capture when recording begins, preserving what the user saw when they started the question.
- **On send:** capture after transcription, using what is visible when the request is sent.

Typed messages capture on send because they have no recording-start event. In Agent decides mode, capture happens when the model calls the tool, so the Immediate and On send setting does not control that path.

Immediate capture runs in a background thread while the user speaks. The frame carries a generation token. If the user cancels, starts another turn, or changes screen permission, the token becomes invalid and the old frame cannot be attached to a later request.

The agent-decided path adds a visible screenshot marker and compact thumbnail to the conversation when the tool succeeds. This preserves an audit trail even though the model chose when to look.

> Cancellation is not a button. It is a rule that every background task must understand.

## Send the full screenshot once, persist only a thumbnail

Keeping full-resolution screenshots in conversation history sounded convenient until I considered what it meant.

Every later request could resend the same image. Context usage would grow. History files would become heavy. The app would retain more visual data than it needed.

So the full image belongs to one model turn only.

SpeakoFlow creates a smaller thumbnail for the visible chat history. The user can still see what was shared, but the original frame is not repeatedly sent back to the model.

This preserves a visible audit trail while preventing repeated image payloads from consuming context and growing the persisted conversation history.

## Treating llama.cpp as a managed local service

The built-in assistant runs llama.cpp as a local loopback service.

The application has to care for it:

1. Find or download a compatible engine.
2. Pick the right build for the operating system.
3. Start it on `127.0.0.1`.
4. Load the selected GGUF model.
5. Attach the vision projector when needed.
6. Wait for the health check.
7. Keep it alive during active requests.
8. Unload it after the configured idle period.
9. Make sure it dies when the app dies.

Two rapid requests also cannot be allowed to start two copies of the server. Startup is serialized, and model switches wait for the previous process to release the port.

One tiny flag caused a surprisingly large improvement:

```text
--parallel 1
```

llama-server normally supports multiple generation slots. That is sensible for a shared server. SpeakoFlow is a single-user desktop app.

Multiple slots divide the available context between concurrent requests. A screenshot may already consume a meaningful part of a small local model’s context window, so splitting the remainder can cause early truncation or KV-cache allocation failures.

One slot gives the active desktop conversation the complete configured context. Rare overlapping requests queue instead of competing for fragmented cache space.

The context failures came from a server default optimized for multi-user workloads, not from the model itself.

[![Robert Redford gives a restrained nod of approval](https://media.giphy.com/media/xSM46ernAUN3y/giphy.gif)](https://giphy.com/gifs/stoner-sees-isopropyl-xSM46ernAUN3y)

## One API client talks to both local and cloud models

The local engine exposes an OpenAI-compatible endpoint. The same Rust client can talk to:

- The built-in llama.cpp engine
- Ollama
- LM Studio
- OpenAI-compatible cloud services
- Other providers with small authentication adaptations

Responses arrive as SSE events. The Rust backend forwards text to the React assistant panel while generation is in progress, but coalesces deltas to at most one emit about every 40 ms. Each emit becomes an `evaluate_script` call in the panel WebView, and that upstream path leaks memory per invocation, so batching preserves the streaming effect without issuing one WebView call per token. When the turn finishes, the authoritative conversation snapshot replaces the temporary streamed text.

The assistant can also run a small, bounded tool loop. Depending on the user’s settings, the model can decide to search the web, get the current date, or capture the screen.

The round cap prevents a model from repeatedly calling the same tool without producing a final response.

## TTS interruption is a concurrency problem

Text-to-speech sits after generation, but it has two separate responsibilities: normalize the text for speech and make playback cancellable.

[![A cat putting one paw to its mouth to ask for quiet](https://media.giphy.com/media/n1qrZkwr4Fcj51XAQi/giphy.gif)](https://giphy.com/gifs/lips-finger-on-n1qrZkwr4Fcj51XAQi)

The displayed answer may contain Markdown, code blocks, links, and emoji. SpeakoFlow keeps that original response in the panel while sending a sanitized copy to the speech engine.

The local voice runs Kokoro in the assistant panel through WebGPU where available, with a WASM fallback for unsupported or failed GPU paths. Remote OpenAI-compatible, ElevenLabs, and Azure voices are optional.

Interruption was the harder problem.

If the user starts a new question, the previous answer must stop immediately. SpeakoFlow uses a monotonically increasing playback epoch:

```text
Reply A starts with epoch 12
User begins a new recording
Current epoch becomes 13
Reply A notices that 12 is stale and stops
```

That same mechanism prevents a slow, old speech request from suddenly playing after the conversation has already moved on.

Right now, speech begins after the complete text answer is ready. I considered speaking partial sentences as tokens arrive, but complete replies make cleanup, cancellation, and pronunciation more predictable.

Sentence-level streaming is a possible future optimization, but the current design favors predictable cleanup and cancellation.

## Local-first works better as explicit boundaries

Speech-to-text always runs on the user’s machine. The other stages are choices:

- Built-in local LLM
- Ollama or LM Studio
- A cloud model with the user’s own key
- Local Kokoro text-to-speech
- A configured remote voice provider

This matters on real hardware.

A laptop user may want private local transcription but use a cloud LLM to protect battery life. A desktop user with a GPU may want the entire pipeline offline.

Local-first works better when each boundary is explicit: transcription location, assistant provider, vision permission, memory storage, and speech engine can be configured independently.

## Source map for the pipeline

The implementation is split by lifecycle responsibility rather than by model vendor:

| Source file                                  | Responsibility                                                                                                                    |
| -------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `src-tauri/src/actions.rs`                   | Starts and stops recording, initiates transcription model loading, prewarms the built-in LLM, and starts immediate vision capture |
| `src-tauri/src/transcription_coordinator.rs` | Serializes recording, processing, cancel, commit, lock, and timeout transitions                                                   |
| `src-tauri/src/managers/transcription.rs`    | Loads Whisper, transcribe.cpp, and ONNX engines including Parakeet, Moonshine, SenseVoice, Canary, and others                     |
| `src-tauri/src/screenshot.rs`                | Selects the monitor, applies provider-specific JPEG ladders, and creates persisted thumbnails                                     |
| `src-tauri/src/assistant.rs`                 | Builds turns, enforces screen authorization, runs the bounded tool loop, streams state, and stores conversation history           |
| `src-tauri/src/managers/local_llm.rs`        | Installs, starts, health-checks, switches, and unloads the local llama.cpp server                                                 |
| `src-tauri/src/llm_client.rs`                | Normalizes provider requests and parses streamed SSE responses                                                                    |
| `src-tauri/src/tts.rs`                       | Sanitizes speech text and cancels stale or interrupted playback                                                                   |
| `src/assistant/useKokoroTts.ts`              | Runs local Kokoro speech with WebGPU preference and a WASM fallback                                                               |

Keeping these boundaries separate made failures easier to locate. A slow first response, stale screenshot, duplicate hotkey event, and late TTS playback each belong to a different lifecycle owner.

## Engineering rules I would reuse

If I built another voice interface tomorrow, I would keep these rules:

1. **Hide cold starts inside actions the user is already taking.** Speaking time is useful time.
2. **Make recording a state machine.** Hotkeys are not ordinary buttons.
3. **Give every background task an identity.** Old work must be able to become invalid.
4. **Treat visual context as a budget.** Resolution, payload size, and vision tokens compete.
5. **Send full images once.** Keep small thumbnails for history.
6. **Make spoken output easy to interrupt.** The assistant should never fight the user for the floor.
7. **Optimize the loop, not the benchmark.** A faster model cannot rescue a sequential pipeline.

The project is open source, so the modules and lifecycle boundaries above can be inspected directly:

{% embed https://github.com/AbhishekBarali/SpeakoFlow %}

I am curious how other developers would handle two choices:

1. Would you start text-to-speech from partial sentences, or wait for the complete answer?
2. For screen vision, would you capture when the user starts speaking or when they finish?

If there is interest, I can write a deeper follow-up about the screenshot compression ladder or managing llama.cpp as a desktop sidecar.

---

_Disclosure: I built and personally reviewed the SpeakoFlow implementation described here. I used an AI assistant to inspect the current code, organize the article, and edit the language. I checked the technical claims against the current source before publication._

---

# Media plan

## 1. Animated cover

Upload the real product demo through DEV’s **Add a cover image** control:

```text
assets/darko.gif
```

The GIF is 789 by 450 and about 1.41 MB. Preview DEV’s crop before publishing. If the recording overlay is cut off, use `assets/hero.gif` instead.

Remove the generated green illustration from any existing DEV draft. It is no longer referenced by this document.

## 2. Product proof after the hook

Because `darko.gif` is the cover, the article uses the separate meeting-notes demo after the opening:

```text
https://raw.githubusercontent.com/AbhishekBarali/SpeakoFlow/main/assets/hero.gif
```

## 3. Waiting reaction GIF

The skeleton appears after the sequential pipeline to illustrate avoidable waiting.

GIPHY page:

```text
https://giphy.com/gifs/waiting-loud-polish-QZyBvNVaMbIZ9yadec
```

Direct GIF:

```text
https://media.giphy.com/media/QZyBvNVaMbIZ9yadec/giphy.gif
```

## 4. Hotkey chaos reaction GIF

The Community GIF appears after the list of repeated press, delayed release, and stale timeout events.

GIPHY page:

```text
https://giphy.com/gifs/community-troy-chaos-137TKgM3d2XQjK
```

Direct GIF:

```text
https://media.giphy.com/media/137TKgM3d2XQjK/giphy.gif
```

## 5. Local-service approval GIF

The restrained Robert Redford nod appears after the `--parallel 1` context-slot explanation.

GIPHY page:

```text
https://giphy.com/gifs/stoner-sees-isopropyl-xSM46ernAUN3y
```

Direct GIF:

```text
https://media.giphy.com/media/xSM46ernAUN3y/giphy.gif
```

## 6. TTS interruption reaction GIF

The shushing cat appears under the TTS concurrency heading.

GIPHY page:

```text
https://giphy.com/gifs/lips-finger-on-n1qrZkwr4Fcj51XAQi
```

Direct GIF:

```text
https://media.giphy.com/media/n1qrZkwr4Fcj51XAQi/giphy.gif
```

## 7. Your screenshots

Add one or two real product screenshots where they support the surrounding explanation. An optional HTML-comment slot remains in the screen-vision section, so leaving it empty will not create a broken image.

The body has one product demonstration and four reaction GIFs. The animated cover is additional, so avoid adding more motion inside the article.

## 8. Product Hunt image

Keep the Product Hunt ranking screenshot for a separate launch or social post. It changes the DEV article from an engineering story into a promotion.

# Publishing checklist

1. Open <https://dev.to/new>.
2. Use the Rich + Markdown editor.
3. Add the recommended title and description.
4. Add `rust`, `tauri`, `ai`, and `opensource`.
5. Remove the generated illustration from any existing draft, then upload `assets/darko.gif` as the animated cover.
6. Paste the updated article. The separate `hero.gif` demo and all four reaction GIFs are embedded in the body.
7. Add one or two screenshots wherever they support the surrounding section.
8. Preview the post and confirm the animated cover and all five body GIFs render correctly.
9. If DEV cannot fetch a reaction GIF, download it from its linked GIPHY page, upload it through DEV’s editor, and replace only the direct media URL.
10. Check every code block on a narrow window.
11. Keep the AI assistance disclosure.
12. Send the unpublished draft link to two technical readers.
13. Publish only after personally checking every implementation claim.
14. Write comment replies yourself. Do not use generated comments.

## The first hour after publishing

Do not drop the link and disappear.

- Reply to every thoughtful comment.
- Ask follow-up questions instead of replying only with “thanks.”
- If someone challenges a tradeoff, explain why you chose it.
- If someone finds a mistake, correct the article and thank them.
- Share the article with a short story, not “please support my post.”

A useful social caption would be:

> I thought the hard part of a local voice assistant would be choosing the models. It was not. It was cancellation, screenshot budgets, hotkey state, model warm-up, and teaching the assistant when to stop talking. I wrote down the weirdest lessons.

## Follow-up posts if this one performs well

- How I Compress Desktop Screenshots for Local Vision Models
- Why My Local llama.cpp Server Uses One Generation Slot
- Stopping Stale Text-to-Speech with Playback Epochs
- Why Voice Shortcuts Need a State Machine

# SourceForge appendix

SourceForge can still be useful for distribution and another credible project listing. Use it as a release mirror and discovery page, not as a replacement for GitHub.

## Recommended setup

1. Open <https://sourceforge.net/p/import_project/github/>.
2. Authorize SourceForge to access GitHub.
3. Select `AbhishekBarali/SpeakoFlow`.
4. Use project name `SpeakoFlow` and URL name `speakoflow`.
5. Import metadata and releases.
6. Import source code only if you want a mirror.
7. Leave issues and the wiki on GitHub to avoid splitting the community.
8. In SourceForge, open the gear beside **Files** and enable **GitHub Integration** for ongoing release synchronization.
9. Add the project logo and six static screenshots.
10. Check the default operating system assigned to every installer.

## Copy-ready SourceForge summary

This is a reader-facing product tagline, not a keyword dump. It uses 62 of the 70 available characters.

```text
Free open-source voice dictation and screen-aware AI assistant
```

## Copy-ready SourceForge description

```text
SpeakoFlow is free, open-source voice dictation and AI assistant software for Windows, macOS, and Linux. Press a global hotkey and speak to type in any app, including email, editors, chat, browsers, and terminals. Voice-to-text runs on your device with Whisper or Parakeet, keeping audio private.

Say “Hey Flow” to turn spoken instructions into polished emails, replies, and drafts, or open the floating screen-aware assistant to ask about the document, app, or error in front of you. SpeakoFlow also provides live transcription, filler-word removal, tone controls, offline translation, text-to-speech, profiles, and editable personal memory. Run AI with the built-in local engine, Ollama, LM Studio, or an optional cloud provider. No account, subscription, word limit, or telemetry is required.
```

## Copy-ready SourceForge features

SourceForge displays these as public bullets. Add these six complete capability statements one at a time:

1. `Dictate into any desktop app with fast, private speech recognition on your device`
2. `Watch words appear live or automatically paste the finished transcript when you stop`
3. `Turn spoken instructions into polished emails, replies, and drafts with Hey Flow`
4. `Ask a screen-aware assistant about the document, app, or error in front of you`
5. `Run the assistant locally or connect Ollama, LM Studio, and cloud AI providers`
6. `Clean up filler words, adjust the tone, translate speech, and hear answers aloud`

Do not put the license, supported operating systems, account policy, telemetry policy, or model names into this field. SourceForge has dedicated metadata for those details.

For **Preferred Support Page**, select **URL** and enter:

```text
https://github.com/AbhishekBarali/SpeakoFlow/issues
```

## SourceForge fields

- Homepage: `https://www.speakoflow.com/`
- Support: `https://github.com/AbhishekBarali/SpeakoFlow/issues`
- Repository: `https://github.com/AbhishekBarali/SpeakoFlow`
- License: `MIT License`
- Languages: `Rust, TypeScript, JavaScript`
- Platforms: `Windows, macOS, Linux`
- Logo: `Logo/final/png/icon-512.png`

## Suggested SourceForge screenshots

1. Dictation inside an email or editor
2. “Hey Flow” producing a finished reply
3. Screen-aware assistant response
4. Built-in local model settings
5. AI cleanup before and after
6. Main settings window

# Verified documentation

- DEV Editor Guide: <https://dev.to/p/editor_guide>
- DEV publishing and canonical URLs: <https://dev.to/help/writing-editing-scheduling>
- DEV rules for AI-assisted articles: <https://dev.to/guidelines-for-ai-assisted-articles-on-dev>
- SourceForge GitHub Importer: <https://sourceforge.net/p/forge/documentation/GitHub%20Importer/>
- SourceForge project creation: <https://sourceforge.net/p/forge/documentation/Create%20a%20New%20Project/>
- SourceForge release files: <https://sourceforge.net/p/forge/documentation/Release%20Files%20for%20Download/>
- SourceForge screenshots: <https://sourceforge.net/p/forge/documentation/Adding%20Screenshots/>
