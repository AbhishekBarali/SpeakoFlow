# Translation brief — SpeakoFlow UI strings

SpeakoFlow is a desktop voice app: dictation (speak, text is typed into any
app), AI cleanup of dictated text, an AI assistant panel with voice calls and
screen vision, meeting transcription, a personal dictionary, usage insights,
and local/cloud model management. Users are everyday knowledge workers, not
developers, so copy should read like a polished consumer app.

## Your input and output

- Input: `.i18n-work/<lang>/chunk-N.source.json` — a flat JSON object of
  `dotted.key → English text`. The dotted key tells you where the string
  appears (e.g. `settings.sound.microphone.title`, `onboarding.tour...`).
- Output: `.i18n-work/<lang>/chunk-N.<lang>.json` — the SAME keys in the SAME
  order, every value translated. Valid JSON, UTF-8, 2-space indent, string
  values only. Do not add, drop, or rename keys. Never leave a value empty.
- Write only those output files. Do not edit anything under `src/` and do not
  run `--apply`.

## Consistency with what already exists

Before translating, skim `src/i18n/locales/<lang>/translation.json`. It holds
the strings already translated for your language. Reuse its terminology (how
"dictation", "AI cleanup", "assistant", "shortcut", "model", "history",
"transcript" etc. are rendered) and its form of address (e.g. German du/Sie,
French tu/vous, Spanish tú/usted; formal vs casual in ja/ko). Keep one term
per concept across all chunks.

## Must be preserved exactly

- `{{placeholders}}` — keep every one, spelled identically (you may move it
  within the sentence). Exception: see plurals below.
- Markup: `<em>…</em>`, `<code>…</code>`, `<strong>…</strong>` — keep the
  tags, translate the text inside them. `${output}` stays literal.
- `\n` line breaks.
- Product, brand, company and model names: SpeakoFlow, Flow (the feature
  "Generate with Flow" — translate "Generate with", keep "Flow"), Handy,
  OpenAI, Anthropic, Claude, Gemini, Groq, Mistral, OpenRouter, Ollama,
  LM Studio, Azure, ElevenLabs, Deepgram, Cartesia, Kokoro, Kitten, Pocket TTS,
  Supertonic, Whisper, Parakeet, Moonshine, Canary, SenseVoice, Hugging Face,
  GitHub, llama.cpp, GGUF, WebGPU, CUDA, Vulkan, Metal, etc.
- Keyboard keys and combos (Ctrl, Alt, Shift, Cmd, Fn, Esc, Enter, Space,
  Tab, Insert…), file names/extensions, paths, URLs, code, units (MB, GB,
  ms, tok/s), sample API keys and technical identifiers.

## Plural keys

Keys ending in `_zero`, `_one`, `_two`, `_few`, `_many`, `_other` are CLDR
plural forms chosen by `Intl.PluralRules` for YOUR language. The English text
given is only the singular (`_one`) or plural (`_other`) English form; write
the grammatically correct form for that category in your language (e.g.
Russian `_few` = 2–4 form, `_many` = 5+ form; Arabic has all six). `_zero`,
`_one` and `_two` may drop `{{count}}` if the language naturally does
("no images", "one image"); every other form must keep `{{count}}`.
`{{count}}` may also be _added_ to any plural form. Do that when your `_one`
category covers more than the number 1 (Russian/Ukrainian `_one` also means
21, 31, …; French/Portuguese `_one` also means 0), so "Snoozed once" must
become a form with `{{count}}` there.

## Style

- Natural, idiomatic UI language — not word-for-word. Keep buttons and labels
  short; a label should not become much longer than the English.
- Keep sentence case / capitalization conventions of your language (German
  nouns capitalized, no English Title Case).
- Use the target language's punctuation and quotation marks; keep "…" as the
  single ellipsis character.
- Example/sample sentences (e.g. messy dictation samples, sample questions)
  should be rewritten as equally natural samples in your language, keeping
  their purpose (a messy sample stays messy, a cleaned one stays clean).
- If a value is genuinely identical in your language (a brand name, "OK",
  a model name like "Whisper Large"), output it unchanged.

## Check your work

Run `node scripts/i18n-merge.mjs --check <lang>` from the repo root. Fix any
line mentioning your chunks: invalid JSON, missing/unknown keys, empty
translation, placeholder mismatch. "chunk files not produced yet" only refers
to chunks assigned to someone else and can be ignored.
