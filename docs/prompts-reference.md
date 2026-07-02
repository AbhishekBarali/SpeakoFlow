# Prompts Reference — where every LLM instruction in SpeakoFlow lives

A single map of **every prompt, system message, and instruction string** the app
feeds to an LLM, so they can be found and tuned quickly (by a human or an agent)
instead of grepping blind. Prompt quality — not the model — is usually what makes
answers feel good or awful, so this is the first place to look when output is off.

> Line numbers are hints (they drift as code changes); the **symbol name** is the
> stable anchor. Search for the symbol if the line moved.

---

## 0. The big picture: how an assistant turn is assembled

When you ask the assistant something, `run_assistant_turn`
(`src-tauri/src/assistant.rs`) builds **two** messages whose content is stitched
together from several prompt fragments. Understanding this order is the key to
debugging "why did it say that":

**System message** (built ~`assistant.rs:780–815`), concatenated in this order:

1. **Assistant system prompt** — your editable base persona.
2. `TIME_AWARENESS_NOTE` — always added.
3. `WEB_SEARCH_CAPABILITY_NOTE` — added whenever web search is _enabled_ (every turn, search or not).
4. **Response-length directive** — added unless length = Default.
5. **Web-search grounding directive** — added _only_ on turns that actually found web results.

**User message** (built ~`assistant.rs:860–895`), prepended to your text:

1. `current_datetime_line()` — the live local date/time.
2. **Web results block** (`format_results_for_prompt`) — only when results were found.
3. Your actual transcribed/typed text.
4. `SCREENSHOT_MARKER` is stripped/handled for vision turns.

So a single reply is shaped by **up to 5 system fragments + 2 user-message
fragments**. If the assistant says something weird ("the results you sent me",
hedging, wrong length), it's almost always one of these fragments — not the model.

---

## 1. Assistant chat prompts

| Prompt                        | Location (symbol)                                                                               | Editable?                                                                                           | When it's used                                        | Purpose                                                                                     |
| ----------------------------- | ----------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- | ----------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| **Assistant system prompt**   | `settings.rs` → `default_assistant_system_prompt()` (~929); stored in `assistant_system_prompt` | ✅ **User** — Settings → Assistant → System Prompt                                                  | Every assistant turn (the base of the system message) | Defines the persona ("helpful voice assistant", concise, plain text, use screenshots).      |
| **Time awareness note**       | `assistant.rs` → `TIME_AWARENESS_NOTE` (~495)                                                   | ❌ Hardcoded                                                                                        | Every turn                                            | Tells the model the live date/time is in the user message and to treat it as "now".         |
| **Response-length directive** | `settings.rs` → `AssistantResponseLength::directive()` (~217)                                   | ⚙️ User picks Short/Medium/Long/Default (Settings → Assistant → Response length); text is hardcoded | When length ≠ Default                                 | Controls reply length. Also shapes the **spoken** reply, since TTS reads the answer itself. |
| **Live date/time line**       | `assistant.rs` → `current_datetime_line()` (~497)                                               | ❌ Hardcoded                                                                                        | Every turn (prepended to the user message)            | Injects the actual timestamp; kept out of the system prompt so prompt-caching stays stable. |
| **Screenshot marker**         | `assistant.rs` → `SCREENSHOT_MARKER` = `"[screenshot attached]"` (~63)                          | ❌ Hardcoded                                                                                        | Vision turns                                          | Marks that an image accompanied the message; stripped from the panel display.               |

The default system prompt in code includes a screenshot sentence that your
**saved** prompt (in `settings_store.json`) currently lacks — your saved value
overrides the default, so edit it in Settings, not in code.

---

## 2. Web-search prompts (`src-tauri/src/web_search.rs`)

| Prompt                   | Location (symbol)                                                      | Editable?                        | When it's used                                                               | Purpose                                                                                                                                                                                                                                                         |
| ------------------------ | ---------------------------------------------------------------------- | -------------------------------- | ---------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Search planner**       | `PLANNER_SYSTEM_PROMPT` (~172), used by `plan_search()`                | ❌ Hardcoded                     | Before a search, on cloud/custom providers (and built-in when "smart" is on) | Decides _whether_ to search and rewrites the messy voice transcript into 1–4 clean queries + a freshness window + a news flag. `plan_search` also appends "Today's date is …" and, for providers without structured output, a strict JSON-shape instruction.    |
| **Capability note**      | `WEB_SEARCH_CAPABILITY_NOTE` (~165)                                    | ❌ Hardcoded                     | Every turn while web search is **enabled**                                   | Tells the model it _has_ a web tool and must not claim "I can't browse"; trust the current date over its training year.                                                                                                                                         |
| **Grounding directive**  | `web_search_system_directive(tts_enabled)` (~125)                      | ❌ Hardcoded (TTS-aware builder) | Only on turns where results were **found**                                   | The big one for answer quality: frames results as the assistant's _own_ findings (never "the results you sent"), demands a direct BLUF answer, bans hedging/asking-to-clarify, and switches formatting (prose+bullets when TTS is on; tables allowed when off). |
| **Results block header** | `format_results_for_prompt()` (~“[Web search results you retrieved…]”) | ❌ Hardcoded                     | When results are injected into the user message                              | Labels the block as the assistant's own retrieval, not user-provided.                                                                                                                                                                                           |

> **History note:** the "assistant keeps asking the user for more information"
> behavior came from the _old_ grounding directive (it invited "say what's
> unclear", which models escalated into clarifying questions). The current
> `web_search_system_directive` explicitly bans asking-to-clarify when results
> are present. There is **no** "ask the user for more info" instruction left in
> code or in the saved system prompt.

---

## 3. Dictation post-processing prompt (`src-tauri/src/settings.rs`)

| Prompt                       | Location (symbol)                                                         | Editable?                                        | When it's used                                                                        | Purpose                                                                                                                                                |
| ---------------------------- | ------------------------------------------------------------------------- | ------------------------------------------------ | ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **"Improve Transcriptions"** | `default_post_process_prompts()` (~907); stored in `post_process_prompts` | ✅ **User** — Settings (post-processing prompts) | When dictation post-processing is enabled (the `transcribe_with_post_process` hotkey) | Cleans a raw transcript (spelling, numbers, punctuation, filler). Uses the `${output}` placeholder for the transcript. Not used by the assistant chat. |

---

## 4. Text-to-speech (TTS)

There is **no separate "summary" prompt in the current code.** `spawn_tts_speak`
(`assistant.rs`) speaks the **full assistant reply** after stripping Markdown via
`sanitize_for_speech` (`tts.rs`). Reply length is controlled by the
response-length directive (section 1), not a summarizer.

> ⚠️ **Stale references to clean up:** `src/i18n/locales/en/translation.json`
> still has `settings.assistant.tts.promptLabel` = "Summary Prompt" /
> `promptDescription`, and `docs/PROGRESS.md` still says a "1–3 sentence spoken
> recap (prompt configurable)" is generated. Both describe an earlier design that
> was replaced; there is no summary-prompt setting or command in the code today.

---

## 5. Not prompts (but easy to mistake for them)

- **Vision error messages** — `assistant.rs` → `vision_unsupported_message()` /
  the `Some(Err(e))` branch. These are user-facing strings, not model input.
- **`should_search` / `looks_time_sensitive` / `is_explicit_search_request`** —
  `web_search.rs` keyword heuristics (Rust logic, not LLM prompts) that gate
  whether the planner runs at all.
- **Screen-vision keyword list** — `assistant.rs` phrases that auto-attach a
  screenshot ("what's on my screen", etc.). Logic, not a prompt.

---

## Quick "the assistant is doing X wrong" → look here

| Symptom                                           | Most likely prompt                                                                 |
| ------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Says "the results you sent / your search results" | `web_search_system_directive` + `format_results_for_prompt` (§2)                   |
| Hedges, under-delivers, or asks you to clarify    | `web_search_system_directive` (§2)                                                 |
| Replies too long / too short                      | Response-length directive (§1) + your saved system prompt                          |
| Claims it can't browse the internet               | `WEB_SEARCH_CAPABILITY_NOTE` (§2)                                                  |
| Searches when it shouldn't (or vice-versa)        | `PLANNER_SYSTEM_PROMPT` + the heuristics in §5                                     |
| Wrong/old date assumptions                        | `TIME_AWARENESS_NOTE` + `current_datetime_line()` (§1)                             |
| Dictation cleanup is wrong                        | "Improve Transcriptions" post-process prompt (§3)                                  |
| Spoken reply reads symbols/markdown aloud         | `sanitize_for_speech` + TTS-aware branch of `web_search_system_directive` (§4, §2) |
