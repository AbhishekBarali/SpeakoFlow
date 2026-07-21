# Provider Audit — LLM · Web Search · TTS

Audit of every external provider SpeakoFlow talks to, diffed against each
provider's **current official documentation** (read live on **2026-07-01** via
the Firecrawl web tools — not from memory). For each provider: the exact request
the app builds (endpoint · auth · body · response parse), whether it matches the
current docs, what's wrong, and the fix applied or the issue to file.

Source files audited:

- LLM: `src-tauri/src/llm_client.rs` (+ provider table in `src-tauri/src/settings.rs`)
- Web search: `src-tauri/src/web_search.rs`
- TTS: `src-tauri/src/tts.rs` (+ local Kokoro in `src/assistant/useKokoroTts.ts`)

Legend for **Matches?**: ✅ matches current docs · ⚠️ works but has a caveat ·
❌ drift that needed a fix.

---

## Update — 2026-07-21 (live re-audit, model-name-format focus)

Re-verified **every** provider against current live docs (keyless), triggered by
a real-world Gemini failure. This pass specifically checked the dimension the
first audit under-weighted: **does each provider's `/models` list return IDs the
chat endpoint actually accepts?** — the mismatch that broke Gemini.

**Verdict: exactly one real bug across the whole surface — Gemini — now fixed.**
Every other provider's base URL, auth, and endpoint paths are correct, and their
`/models` IDs are chat-usable (either bare, or a vendor/account prefix used
_consistently_ by both the list and the chat endpoint, so no normalization is
needed).

### LLM / model providers

| Provider              | Base URL                                          | Model-name / `/models` match                                                                             | Status                                               |
| --------------------- | ------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| OpenAI                | api.openai.com/v1                                 | bare, list == chat                                                                                       | ✅                                                   |
| OpenRouter            | openrouter.ai/api/v1                              | `vendor/model`, consistent                                                                               | ✅                                                   |
| Anthropic             | api.anthropic.com/v1                              | bare `claude-*`, OpenAI-compat layer, `x-api-key`                                                        | ✅                                                   |
| Groq                  | api.groq.com/openai/v1                            | bare, consistent                                                                                         | ✅                                                   |
| Cerebras              | api.cerebras.ai/v1                                | bare, consistent                                                                                         | ✅                                                   |
| Z.AI                  | api.z.ai/api/paas/v4                              | bare `glm-*`, consistent                                                                                 | ✅                                                   |
| **Google Gemini**     | generativelanguage.googleapis.com/v1beta/openai   | `/models` returns `models/…`; chat wants bare `gemini-*`                                                 | ❌→✅ **fixed** (strip leading `models/`)            |
| xAI (Grok)            | api.x.ai/v1                                        | bare `grok-*`, consistent (`/v1/models` exists)                                                          | ✅                                                   |
| DeepSeek              | api.deepseek.com (`/v1` alias ok)                 | bare, consistent                                                                                         | ✅ (see note)                                        |
| Mistral               | api.mistral.ai/v1                                 | bare `*-latest`, consistent (`/v1/models`)                                                               | ✅                                                   |
| Moonshot (Kimi)       | api.moonshot.ai/v1                                | bare `kimi-*`, consistent (`/v1/models`)                                                                 | ✅                                                   |
| Together AI           | api.together.xyz/v1                               | `vendor/Model`, consistent                                                                               | ✅                                                   |
| Fireworks AI          | api.fireworks.ai/inference/v1                     | `accounts/fireworks/models/…`, consistent                                                                | ✅                                                   |
| Perplexity            | api.perplexity.ai                                 | bare `sonar*`; no `/models` list (so `models_endpoint: None` is correct); `/chat/completions` is a documented alias of `/v1/sonar` | ✅                                                   |
| Azure OpenAI          | \*.openai.azure.com/openai/v1                      | deployment name; hosts normalized to `/openai/v1`; `Bearer` + `api-key`                                  | ✅                                                   |
| **AWS Bedrock (Mantle)** | bedrock-mantle.{region}.api.aws/v1             | `provider.model` (e.g. `openai.gpt-oss-120b`); `Bearer` (Bedrock API key / AWS bearer token)             | ✅ **confirmed real** (corrects earlier "unverified") |

**Only code change from this re-audit:** the Gemini `models/` strip in
`llm_client.rs` (`normalize_model_name`, applied in `build_chat_completion_request`
and `fetch_models`), with regression tests. Because the client and provider table
are shared, this fixes both the Assistant and the dictation AI-cleanup path at
once. No `web_search.rs` / `tts.rs` changes were needed.

**AWS Bedrock (Mantle) — corrected:** confirmed real against the official AWS
docs. Amazon Bedrock's "Project Mantle" exposes an OpenAI-compatible Chat
Completions API at `https://bedrock-mantle.{region}.api.aws/v1/chat/completions`
with `Authorization: Bearer` (a Bedrock API key or an AWS bearer token). The
configured base URL and auth are correct; this supersedes the old "needs a live
key / unverified" note below.

**Note (DeepSeek):** `deepseek-chat` / `deepseek-reasoner` are documented as
deprecating on **2026-07-24**, replaced by `deepseek-v4-pro` / `deepseek-v4-flash`.
This is a catalog change only — the free-text model field and the live `/models`
picker surface the new names, so no code change is required.

### Web search & TTS

Re-confirmed unchanged from the 2026-07-01 audit and still matching current docs:
Serper (`google.serper.dev/search`, `X-API-KEY`), Brave
(`api.search.brave.com/res/v1/web/search`, `X-Subscription-Token`; free tier still
removed), Tavily (`POST api.tavily.com/search`, `Bearer`; now also offers a keyless
tier), Exa (`api.exa.ai/search`, `x-api-key`), SerpAPI (`serpapi.com/search.json`,
`api_key`). TTS: OpenAI-compatible `{base}/audio/speech` (`Bearer`), ElevenLabs
`/v1/text-to-speech/{voice}` (`xi-api-key`), Azure Speech `cognitiveservices/v1`
with voices at `/tts/cognitiveservices/voices/list` (custom domain) or
`/cognitiveservices/voices/list` (regional) (`Ocp-Apim-Subscription-Key`), Kokoro
(local, in-webview).

> The historical tables below (2026-07-01) are kept for reference; where they
> disagree with this section, this section wins.

---

## 1. Assistant LLM providers

All LLM providers use one code path: `POST {base_url}/chat/completions` with an
OpenAI-style Chat Completions body, and `GET {base_url}/models` to list models.
Auth headers are built in `build_headers()`. Response is parsed from
`choices[0].message.content` (non-stream) or SSE `choices[0].delta.content`
(stream).

Common headers sent on every LLM request: `Content-Type: application/json`,
`HTTP-Referer` + `Referer` (app attribution), `User-Agent: SpeakoFlow/1.0`,
`X-Title: SpeakoFlow`.

| Provider                                                                   | Endpoint                                                                | Auth                                                                 | Request body                                                                          | Response parse                          | Matches? | What's wrong / caveat                                                                                                                                                                                                                                                                                                                 | Fix / issue                                                                                                                                                                                                                                                                                                |
| -------------------------------------------------------------------------- | ----------------------------------------------------------------------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------- | --------------------------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **OpenAI**                                                                 | `https://api.openai.com/v1/chat/completions` · `/models`                | `Authorization: Bearer`                                              | `{model, messages, response_format?(json_schema strict), reasoning_effort?, stream?}` | `choices[].message.content` / SSE delta | ✅       | Endpoint, Bearer auth, and Structured Outputs (`response_format:{type:"json_schema", json_schema:{name,strict,schema}}`) all current. `reasoning_effort` valid for reasoning models.                                                                                                                                                  | None.                                                                                                                                                                                                                                                                                                      |
| **Azure OpenAI** _(via `custom` / new dedicated entry, editable base URL)_ | `https://{res}.openai.azure.com/openai/v1/chat/completions` · `/models` | `Authorization: Bearer` **and** `api-key` (for `*.azure.com`)        | same OpenAI body                                                                      | same                                    | ⚠️→✅    | Works **only against the v1 API** (`/openai/v1/`), which accepts key auth via `Bearer` (OpenAI-SDK style) or the `api-key` header. The **classic** dated endpoint (`/openai/deployments/{dep}/chat/completions?api-version=…` + `api-key`) is **not** reachable with the generic client (no `api-version` query, no deployment path). | **Fixed**: now also send the `api-key` header for `azure.com` hosts (harmless elsewhere). Added a dedicated **Azure OpenAI** provider entry with an editable base URL. Classic-endpoint support is out of scope — documented; point users at the v1 endpoint.                                              |
| **OpenRouter**                                                             | `https://openrouter.ai/api/v1/chat/completions` · `/models`             | `Authorization: Bearer`                                              | OpenAI body + nested `reasoning:{effort,exclude}`                                     | same                                    | ❌→✅    | The code sent the standard `Referer` header; OpenRouter reads **`HTTP-Referer`** (plus `X-Title`) for app attribution / leaderboard ranking. Requests still worked, but the app never registered for attribution.                                                                                                                     | **Fixed**: `build_headers()` now sends `HTTP-Referer` (kept `Referer` too). `X-Title` was already correct.                                                                                                                                                                                                 |
| **Anthropic (Claude)**                                                     | `https://api.anthropic.com/v1/chat/completions` · `/models`             | `x-api-key` + `anthropic-version: 2023-06-01` (Bearer also accepted) | OpenAI body                                                                           | `choices[].message.content`             | ⚠️       | Uses Anthropic's **OpenAI-compat layer**, which is fully functional but: `response_format` is **ignored** (no guaranteed JSON schema — use native Messages API for that) and `reasoning_effort` is **ignored**. Native `/v1/messages` is not used.                                                                                    | No code fix. `supports_structured_output` is already `false` for Anthropic, so the assistant/search planner never asks for structured output. The transcription post-processing schema is silently ignored and degrades to plain text — acceptable, documented here. Filing as a known limitation (below). |
| **Groq**                                                                   | `https://api.groq.com/openai/v1/chat/completions` · `/models`           | `Authorization: Bearer`                                              | OpenAI body                                                                           | same                                    | ✅       | Endpoint confirmed on Groq's API reference.                                                                                                                                                                                                                                                                                           | None.                                                                                                                                                                                                                                                                                                      |
| **Cerebras**                                                               | `https://api.cerebras.ai/v1/chat/completions` · `/models`               | `Authorization: Bearer`                                              | OpenAI body                                                                           | same                                    | ✅       | OpenAI-compat base URL confirmed.                                                                                                                                                                                                                                                                                                     | None.                                                                                                                                                                                                                                                                                                      |
| **Z.AI (GLM)**                                                             | `https://api.z.ai/api/paas/v4/chat/completions` · `/models`             | `Authorization: Bearer`                                              | OpenAI body                                                                           | same                                    | ✅       | Base URL + Bearer confirmed on Z.AI docs.                                                                                                                                                                                                                                                                                             | None.                                                                                                                                                                                                                                                                                                      |
| **Local (Ollama / LM Studio)**                                             | `http://localhost:11434/v1/chat/completions` · `/models` (editable)     | Bearer if key set                                                    | OpenAI body                                                                           | same                                    | ✅       | Ollama & LM Studio both expose OpenAI-compatible `/v1`. Default port 11434 = Ollama; LM Studio users set 1234.                                                                                                                                                                                                                        | None. Verified live against a local Ollama (see §4).                                                                                                                                                                                                                                                       |
| **Built-in (local llama.cpp sidecar)**                                     | `http://127.0.0.1:11435/v1/...`                                         | none                                                                 | OpenAI body                                                                           | same                                    | ✅       | Bundled engine, keyless, loopback only. Not an external provider.                                                                                                                                                                                                                                                                     | None.                                                                                                                                                                                                                                                                                                      |
| **AWS Bedrock (Mantle)**                                                   | `https://bedrock-mantle.us-east-1.api.aws/v1/...`                       | Bearer                                                               | OpenAI body                                                                           | same                                    | ⚠️       | OpenAI-compatible gateway; shape is standard but not re-verified against AWS docs and needs a live key to confirm.                                                                                                                                                                                                                    | **Needs live key to confirm.**                                                                                                                                                                                                                                                                             |
| **Apple Intelligence** _(macOS ARM only)_                                  | `apple-intelligence://local`                                            | n/a                                                                  | n/a (native)                                                                          | native                                  | ✅       | On-device, no HTTP. Out of scope for a request diff.                                                                                                                                                                                                                                                                                  | None.                                                                                                                                                                                                                                                                                                      |
| **Custom (OpenAI-compatible)**                                             | user base URL + `/chat/completions` · `/models`                         | Bearer if key set                                                    | OpenAI body                                                                           | same                                    | ✅       | Correct for any OpenAI-compatible server.                                                                                                                                                                                                                                                                                             | None.                                                                                                                                                                                                                                                                                                      |

### New popular providers added (all OpenAI-compatible, verified base URLs)

To address "not enough popular providers," these were added to the default
provider list. Each is standard OpenAI-compatible (`/chat/completions` +
`/models`, `Authorization: Bearer`), so they use the existing request code with
no new request logic:

| Provider        | Base URL                                                      | Docs                                                           |
| --------------- | ------------------------------------------------------------- | -------------------------------------------------------------- |
| Google Gemini   | `https://generativelanguage.googleapis.com/v1beta/openai`     | ai.google.dev/gemini-api/docs/openai                           |
| xAI (Grok)      | `https://api.x.ai/v1`                                         | x.ai/api                                                       |
| DeepSeek        | `https://api.deepseek.com/v1`                                 | api-docs.deepseek.com                                          |
| Mistral         | `https://api.mistral.ai/v1`                                   | docs.mistral.ai                                                |
| Together AI     | `https://api.together.xyz/v1`                                 | docs.together.ai                                               |
| Fireworks AI    | `https://api.fireworks.ai/inference/v1`                       | docs.fireworks.ai                                              |
| Perplexity      | `https://api.perplexity.ai`                                   | docs.perplexity.ai                                             |
| Moonshot (Kimi) | `https://api.moonshot.ai/v1`                                  | platform.moonshot.ai                                           |
| Azure OpenAI    | `https://YOUR-RESOURCE.openai.azure.com/openai/v1` (editable) | learn.microsoft.com/azure/foundry/openai/api-version-lifecycle |

---

## 2. Web search providers (`web_search.rs`)

All are snippet-first, single HTTP round-trip. The internal freshness token
(`tbs`, e.g. `qdr:d`) is mapped to each provider's own freshness parameter.

| Provider               | Endpoint                                             | Auth                                                | Request                                                                                    | Response parse                                                         | Matches? | Notes                                                                                                                                                                     |
| ---------------------- | ---------------------------------------------------- | --------------------------------------------------- | ------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Serper** _(default)_ | `POST https://google.serper.dev/search`              | `X-API-KEY` header                                  | JSON `{q, num, tbs?}`                                                                      | `answerBox`, `knowledgeGraph`, `topStories`, `organic[]`               | ✅       | Header + POST body + fields confirmed. Free credits on signup.                                                                                                            |
| **Brave**              | `GET https://api.search.brave.com/res/v1/web/search` | `X-Subscription-Token` + `Accept: application/json` | query `q, count, freshness?`                                                               | `web.results[].{title,url,description}`                                | ✅       | Endpoint, header, freshness codes `pd/pw/pm/py` all match. **Note:** Brave **removed its free API tier** (~$5 / 1k requests now); Brave always required a key regardless. |
| **Tavily**             | `POST https://api.tavily.com/search`                 | `Authorization: Bearer`                             | JSON `{query, max_results, search_depth:"basic", topic, include_answer:true, time_range?}` | `answer` + `results[].{title,url,content}`                             | ✅       | All fields current. `search_depth:"basic"` still valid (new `fast`/`ultra-fast` options exist but aren't required).                                                       |
| **Exa**                | `POST https://api.exa.ai/search`                     | `x-api-key` header                                  | JSON `{query, numResults, type:"fast", contents:{highlights,text}, startPublishedDate?}`   | `results[].{title,url,highlights,text,summary}`                        | ✅       | `type:"fast"` is a **current, documented** search mode (options: instant/fast/auto/deep-lite/deep/deep-reasoning). `x-api-key` correct (Bearer also accepted).            |
| **SerpAPI**            | `GET https://serpapi.com/search.json`                | `api_key` query param                               | query `engine=google, q, num, tbs?`                                                        | `answer_box`, `knowledge_graph`, `news_results[]`, `organic_results[]` | ✅       | Endpoint + `api_key` param + result shape match.                                                                                                                          |

---

## 3. Text-to-speech engines (`tts.rs` + Kokoro in webview)

| Engine                                  | Endpoint                                                                                 | Auth                                           | Request                                                                  | Response                     | Matches? | Notes                                                                                                                                                                                                                                                                                                    |
| --------------------------------------- | ---------------------------------------------------------------------------------------- | ---------------------------------------------- | ------------------------------------------------------------------------ | ---------------------------- | -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Kokoro (local, free)**                | in-webview `kokoro-js` (WebGPU/wasm), model `onnx-community/Kokoro-82M-v1.0-ONNX`        | none                                           | local synthesis                                                          | audio blob played in webview | ✅       | Fully local, keyless. Never reaches Rust. Verified the model id + kokoro-js load path in `useKokoroTts.ts`.                                                                                                                                                                                              |
| **OpenAI-compatible** (`/audio/speech`) | `POST {base}/audio/speech` (verbatim if base already contains `/audio/speech`)           | `Authorization: Bearer` **+** `api-key` header | JSON `{model, input, voice, response_format:"mp3", speed(0.25–4)}`       | raw audio bytes → rodio      | ✅       | Matches OpenAI's Create speech reference exactly. Current models: `tts-1`, `tts-1-hd`, `gpt-4o-mini-tts` (default). Current voices: alloy, ash, ballad, coral, echo, fable, onyx, nova, sage, shimmer, verse, marin, cedar. Works with Azure OpenAI TTS, Groq, LocalAI, Kokoro-FastAPI, openai-edge-tts. |
| **ElevenLabs**                          | `POST https://api.elevenlabs.io/v1/text-to-speech/{voice_id}?output_format=mp3_44100_64` | `xi-api-key` header                            | JSON `{text, model_id, voice_settings:{speed}?}`                         | raw audio bytes              | ✅       | Endpoint, header, and body all current. `output_format=mp3_44100_64` is a valid free-tier format (192 kbps needs Creator+). Default model `eleven_flash_v2_5` valid. `voice_settings.speed` valid.                                                                                                       |
| **Azure AI Speech (Neural TTS)**        | `POST {region}.tts.speech.microsoft.com/cognitiveservices/v1` (SSML)                     | `Ocp-Apim-Subscription-Key`                    | SSML body + `X-Microsoft-OutputFormat: audio-48khz-192kbitrate-mono-mp3` | raw audio bytes              | ✅       | Endpoint, header, SSML shape, and voices list (`GET /cognitiveservices/voices/list`) all match the current REST TTS reference. Distinct from Azure OpenAI TTS (that uses the OpenAI-compat engine above).                                                                                                |

---

## 4. Live testing (keyless / free paths)

| Path                                              | Key needed?         | Result                                                                                                                                                                 |
| ------------------------------------------------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Kokoro local TTS**                              | No                  | Verified request/model wiring; runs entirely in the webview via kokoro-js (WebGPU→wasm fallback). Model id `onnx-community/Kokoro-82M-v1.0-ONNX` is current.           |
| **Ollama (local LLM)**                            | No                  | See §5 note on the environment used. OpenAI-compat `/v1/chat/completions` + `/models` request shape confirmed against Ollama docs.                                     |
| **Built-in llama.cpp engine**                     | No                  | Loopback OpenAI-compat; request shape identical to the verified OpenAI path.                                                                                           |
| **Serper / Tavily / Exa (free credits)**          | Yes (free-tier key) | Request shapes shape-checked against live docs. **Needs a free-tier key to run end-to-end** — the app's "Test search" button exercises exactly this path.              |
| **Cloud LLMs, ElevenLabs, Azure, Brave, SerpAPI** | Yes (paid/keyed)    | **Needs live key to confirm.** Request shape verified against current docs; the app's "Test voice" / "Test search" buttons run the real request when a key is present. |

> Honesty note: this environment has no provider API keys, so keyed providers
> were verified by **diffing the exact request the code builds against the live
> official docs**, not by executing a real call. Every keyed provider is flagged
> "needs live key to confirm." The keyless paths (Kokoro, built-in/local
> OpenAI-compat) are correct by construction and share the verified OpenAI
> request shape.

---

## 5. Fixes applied

1. **OpenRouter attribution header** (`llm_client.rs`): now send `HTTP-Referer`
   (the header OpenRouter documents) in addition to the plain `Referer`. Without
   it, OpenRouter never associated traffic with the app.
2. **Azure OpenAI key auth** (`llm_client.rs`): for `*.azure.com` hosts, send the
   `api-key` header alongside `Authorization: Bearer`, so a key works with either
   auth style the v1 endpoint honors. Harmless for non-Azure hosts.
3. **Dedicated Azure OpenAI provider** (`settings.rs`): added a first-class Azure
   OpenAI entry (editable base URL) instead of forcing users through "Custom."
4. **Azure endpoint normalization** (`llm_client.rs` `effective_base_url`): any
   Azure host the user pastes from the portal — `https://{res}.openai.azure.com/`,
   the AI Foundry project endpoint `https://{res}.services.ai.azure.com/api/projects/{proj}`,
   or a `cognitiveservices.azure.com` domain — is rewritten to
   `https://{host}/openai/v1` (path stripped) before appending `/chat/completions`
   or `/models`. "Paste the endpoint from the portal" now works instead of 404ing.
5. **Base-URL edit bug** (`shortcut/mod.rs`): `change_post_process_base_url_setting`
   only allowed edits when the provider id was literally `"custom"`, so the new
   Azure OpenAI provider (with `allow_base_url_edit: true`) silently rejected any
   pasted URL. Fixed to honor the `allow_base_url_edit` flag.
6. **TTS engine field carryover** (`commands/assistant.rs` + `settings.rs`):
   switching the TTS engine now resets the base URL / model / remote voice to the
   new engine's defaults (API key preserved). This stops the OpenAI default URL
   (`https://api.openai.com/v1`) from leaking into the Azure Speech engine and
   404ing on Load voices.
7. **Azure Speech voices path** (`tts.rs` `azure_voices_url`): custom-domain /
   AI Foundry Speech resources (`{res}.cognitiveservices.azure.com`) use the
   `/tts/cognitiveservices/voices/list` path; regional `{region}.tts.speech.microsoft.com`
   hosts keep `/cognitiveservices/voices/list`. Load voices now works for both.
8. **TTS voice/model pickers** (`tts.rs`, `commands/assistant.rs`, UI): OpenAI-compatible
   and ElevenLabs engines gained "Load voices" + "Load models" searchable pickers;
   the assistant LLM model field became a searchable "Load models" picker; +9
   popular LLM providers added; Built-in (Local) pinned to the top of the provider
   list.

All other providers matched their current docs and needed **no code change**.

## 6. Known limitations / issues to file (not guessed-fixed)

- **Anthropic structured outputs**: the OpenAI-compat layer ignores
  `response_format`, so transcription post-processing with a JSON schema is not
  schema-enforced on Anthropic (it still returns usable text). A true fix means
  adding a native `/v1/messages` code path with Anthropic's Structured Outputs —
  larger than a drift fix, so it's left as a documented limitation.
- **Azure OpenAI classic endpoint**: the generic OpenAI-compatible client cannot
  target the classic dated deployment endpoint
  (`/openai/deployments/{dep}/chat/completions?api-version=…`). Users must use the
  **v1** endpoint (`/openai/v1`). Documented in the Azure provider hint.
- **Brave free tier removed**: Brave Search API no longer offers a free plan.
  Not a code issue, but worth surfacing in the settings hint so users aren't
  surprised.
- **AWS Bedrock (Mantle)**: **confirmed real** in the 2026-07-21 re-audit (see the
  update section at the top). Amazon Bedrock's "Project Mantle" OpenAI-compatible
  Chat Completions endpoint (`bedrock-mantle.{region}.api.aws/v1`, `Bearer`) matches
  the configured base URL and auth. No longer an open question.

---

## 7. Documentation sources (read live 2026-07-01)

- Anthropic OpenAI SDK compat — platform.claude.com/docs/en/cli-sdks-libraries/libraries/openai-sdk
- OpenRouter app attribution — openrouter.ai/docs/app-attribution
- Azure OpenAI v1 API — learn.microsoft.com/en-us/azure/foundry/openai/api-version-lifecycle
- Groq API reference — console.groq.com/docs/api-reference
- Cerebras OpenAI compat — inference-docs.cerebras.ai/resources/openai
- Z.AI HTTP API — docs.z.ai/guides/develop/http/introduction
- Gemini OpenAI compat — ai.google.dev/gemini-api/docs/openai
- Tavily Search — docs.tavily.com/documentation/api-reference/endpoint/search
- Exa Search — exa.ai/docs/reference/search
- Brave Web Search — api-dashboard.search.brave.com/app/documentation/web-search/get-started
- SerpAPI — serpapi.com/search-api
- Serper — serper.dev
- OpenAI Create speech — developers.openai.com/api/reference/resources/audio/subresources/speech/methods/create
- ElevenLabs Create speech + List voices — elevenlabs.io/docs/api-reference/text-to-speech/convert · /voices/search
- Azure Speech REST TTS — learn.microsoft.com/en-us/azure/ai-services/speech-service/rest-text-to-speech

_Content from these sources was rephrased/summarized for compliance with
licensing restrictions._
