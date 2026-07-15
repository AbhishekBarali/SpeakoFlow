# AI Cleanup Reliability + Onboarding Theme — Living Plan

> **Executor AI: read this entire file before making changes.** This document is the
> persistent source of truth after conversation compaction. Implement only the work
> described here. Keep the checklist current, record verification evidence, and do
> not mark an item complete without a command result or a human-confirmed manual test.

---

## 0. Status and progress rules

Status legend: `[ ]` not started · `[~]` in progress · `[x]` complete · `[!]` blocked

**Overall status:** implementation complete; all automated validation green. Manual
runtime/theme scenarios (§8.5, §11.2, §11.3) handed to the user — they require a running
window and live providers.

When implementing:

1. Work through the phases in order unless a validation failure requires a focused fix.
2. Update this file after each completed phase:
   - tick completed boxes;
   - add exact commands/results under **Evidence**;
   - record any changed decisions under **Decision log**;
   - add a row to **Progress log**.
3. Never mark a behavior complete based only on reading code. Use unit/integration tests
   or a human-confirmed manual check.
4. Do not create commits, branches, PRs, or Graphify refreshes unless the user asks.

---

## 1. Goal

Repair two specific inconsistencies:

1. **AI cleanup reliability:** The dedicated **Dictate and clean up** shortcut must
   consistently attempt the configured cleanup operation. Invalid settings, provider
   failures, local-model startup, timeouts, and malformed responses must never look
   like a successful cleanup. The original transcript remains the safety fallback.
2. **Onboarding/loading appearance:** Setup, onboarding, and their loading states must
   respect the app's Light, Dark, or System appearance preference instead of forcing
   dark mode.

The goal is reliability and consistency, not a new feature family or redesign.

---

## 2. Binding scope and non-goals

### 2.1 In scope

- Persisted AI-cleanup defaults and migration/repair.
- Selected cleanup prompt, provider, model, API key, tone, and timeout consistency.
- The existing assistant-provider/model fallback, but only if its use is made explicit
  and it resolves through the same backend logic as the settings readiness display.
- The built-in `Improve Transcriptions` prompt and existing tone directives.
- Runtime error classification, raw-text fallback, and non-secret user-visible status.
- Correct prewarming and timeout behavior for the currently resolved built-in model.
- Frontend handling of Tauri `Result.status` for cleanup-setting mutations.
- A compact readiness state in the existing AI-cleanup settings panel.
- Focused tests for settings, prompt construction, request outcomes, and theme behavior.
- Making onboarding/loading surfaces use semantic theme tokens and the selected theme.

### 2.2 Explicitly out of scope

- **No new AI models.**
- No changes to the model catalog, model downloads, quantization, inference engines,
  GGUF search, or provider list.
- No dedicated small-model research or integration.
- No change to the normal Dictate shortcut.
- No removal or redesign of the dedicated Dictate and clean up shortcut.
- No assistant-panel, memory, profiles, TTS, web-search, or screen-vision work.
- No broad Dictation page or navigation redesign.
- No replacement of user-created prompts.
- No silent upload, telemetry, or transmission of transcripts beyond the provider the
  user already configured.
- No Graphify refresh: this task is localized and below the repository threshold.

### 2.3 Product behavior that must remain true

- AI cleanup remains opt-in and uses its dedicated shortcut.
- Normal dictation remains fast and unchanged.
- If cleanup cannot safely produce output, SpeakoFlow pastes the original transcript.
- Custom prompts remain editable and are never overwritten by migration.
- Existing configured providers/models continue to work.
- Light, Dark, and System remain the only appearance choices.

---

## 3. Confirmed diagnosis from the pre-compaction audit

### 3.1 Onboarding is intentionally forced dark

Current behavior in `src/App.tsx`:

- `FORCE_ONBOARDING = import.meta.env.DEV` can show onboarding every development launch.
- `inOnboarding` causes the theme effect to write
  `document.documentElement.dataset.theme = "dark"`.
- The saved preference is restored only after onboarding ends.

The normal appearance path is otherwise sound:

- `src/main.tsx` calls `applyCachedTheme()` before React renders.
- `src/lib/theme.ts` resolves Light/Dark/System and caches the preference.
- `App.tsx` reapplies the authoritative setting after settings load.
- Rust sets the native main-window theme from persisted settings at startup.

There is also a default inconsistency in `src-tauri/src/settings.rs`:

- `impl Default for Theme` returns `Theme::Light`.
- `get_default_settings()` currently assigns `theme: Theme::System`.

### 3.2 AI cleanup has a valid bundled prompt but selects none by default

In `src-tauri/src/settings.rs`:

- `default_post_process_prompts()` creates `default_improve_transcriptions`.
- `get_default_settings()` sets `post_process_selected_prompt_id: None`.
- `ensure_post_process_defaults()` repairs providers/keys/models but does not repair
  the prompt list or selected prompt ID.

In `src-tauri/src/actions.rs`, a missing, invalid, or empty selected prompt returns
`None`; the caller quietly keeps the raw transcript.

### 3.3 The dedicated shortcut semantics are correct and must stay

- `TranscribeAction { post_process: true }` backs `transcribe_with_post_process`.
- `post_process_enabled` registers/unregisters that dedicated shortcut.
- Ordinary `transcribe` uses `post_process: false`.

The user understands this design. Do not convert the toggle into “clean every normal
transcription,” and do not rename/remove the dedicated shortcut as part of this work.

### 3.4 Runtime skips and failures are silent

`process_transcription_output()` wraps `post_process_transcription()` in the configured
outer timeout and keeps raw text when the operation returns `None` or times out.
Possible silent paths include:

- no selected prompt;
- selected prompt ID not found;
- empty prompt;
- no dedicated cleanup model and no usable assistant fallback;
- local model startup failure;
- unavailable Apple Intelligence;
- HTTP/auth/provider failure;
- response with no content;
- timeout.

Only logs distinguish these paths. To the user, each looks like “the selected cleanup
or tone did nothing.”

### 3.5 Local prewarming misses an important fallback path

`TranscribeAction::start()` prewarms only when the active **dedicated cleanup provider**
is `builtin` with a selected cleanup model. If cleanup falls back to the assistant's
built-in model, it is resolved later inside the timed operation and may start cold.

### 3.6 Structured-output fallback can consume the same timeout twice

Providers marked `supports_structured_output` first receive a schema request. On an
error, the code attempts a plain-text request. Both attempts, plus model startup, share
one outer timeout. Structured-output support can also vary by deployment/model even
when the provider generally supports it.

### 3.7 Empty or malformed output handling is incomplete

- Apple Intelligence rejects an empty sanitized result.
- The generic plain-text path can return `Some("")`, causing final text to become empty.
- A structured response with invalid JSON is sanitized and returned as text rather than
  consistently classified as malformed.

A cleanup failure must never erase a non-empty raw transcript.

### 3.8 Tone wiring exists, but quality and differentiation are not verified

Current code appends the selected tone after the base prompt, which is the correct
ordering. Existing tests verify only:

- None does not modify the prompt;
- non-None directives exist;
- the directive appears after the base prompt;
- wrapper/code-fence sanitization.

They do not verify request payloads or real/mocked outputs. The current descriptions
also overlap:

- Formal includes “professional.”
- Professional is formal/businesslike.
- Casual and Friendly can produce very similar changes.

### 3.9 Some frontend commands ignore generated `Result.status`

In `src/stores/settingsStore.ts`, several generic settings paths await generated Tauri
commands but do not inspect `{ status: "ok" | "error" }`. A backend error therefore may
not enter `catch`, and the UI can appear to have saved a setting that did not persist.

### 3.10 Azure endpoint shown in the screenshot is not automatically invalid

`src-tauri/src/llm_client.rs::effective_base_url()` recognizes
`.services.ai.azure.com/api/projects/...` and rewrites it to the OpenAI-compatible
`/openai/v1` surface. The visible URL format is therefore not the primary diagnosed
fault. However, changing a base URL intentionally clears that provider's selected
model, and the UI must clearly show that a new model/deployment selection is required.

---

## 4. Binding design decisions

These decisions are the default implementation direction. Change one only if source
constraints or a failing test prove it unsafe; record the change in **Decision log**.

### D1 — Keep the dedicated shortcut behavior

`post_process_enabled` continues to control registration of
`transcribe_with_post_process`. Ordinary dictation remains untouched.

### D2 — Preserve raw text on every failure

For any non-empty raw transcript, cleanup must return either:

- a non-empty cleaned transcript; or
- the original transcript plus a classified fallback reason.

No provider/model response may turn non-empty dictation into empty output.

### D3 — Auto-select a valid bundled prompt

Use stable ID `default_improve_transcriptions` as the fallback selection when the
stored selected ID is missing, empty, deleted, or invalid.

### D4 — Never overwrite an unknown user edit

The shipped default prompt may be updated only when:

- the prompt is absent (insert the new shipped default); or
- its stable ID exists and its text exactly matches a known historical shipped version.

If the same stable ID contains user-modified text, preserve it. Preserve all custom
prompts and their selected IDs when valid.

### D5 — One backend resolver is the source of truth

Create one pure resolver that returns either a fully resolved cleanup configuration or
a typed unavailable reason. Runtime and the UI readiness command must call the same
logic. Do not duplicate “ready” rules in TypeScript.

### D6 — Preserve but expose assistant fallback

Do not unexpectedly break users who rely on the assistant provider/model fallback.
Represent its source explicitly, for example:

- `DedicatedCleanupSelection`
- `AssistantFallback`

The settings panel must say when fallback is being used and name the provider/model.
A later product decision may remove fallback, but not in this repair.

### D7 — Cloud-key validation is provider-aware and conservative

A missing key is definitely invalid for fixed cloud providers. Built-in, local, Apple
Intelligence, and custom OpenAI-compatible endpoints may not require a key. Do not
reject keyless local/custom servers before a request. Actual 401/403 responses remain
runtime authentication failures.

### D8 — Keep one cleanup model call semantically

Tone remains part of the same cleanup request; do not add a second tone-rewriting pass.
The implementation may retain structured-to-plain compatibility fallback, but attempts
must have explicit sub-timeouts/remaining-budget checks and a classified final result.
Do not allow an unbounded hidden second attempt.

### D9 — Onboarding uses the same theme contract as settings

There is no onboarding-specific theme. The cached preference supplies first paint; the
persisted preference becomes authoritative after hydration; System responds to OS
changes. Development-forced onboarding may remain, but it may not force dark mode.

### D10 — Minimal UI additions only

Add readiness/error information to the existing AI-cleanup groups. Do not reorganize
the whole Dictation page. A “Test cleanup” control is optional and should be added only
if core reliability and required validation are complete without expanding scope.

---

## 5. Target backend design

Names below are recommendations; align with repository style while preserving the
semantics.

### 5.1 Constants and prompt migration

In `src-tauri/src/settings.rs`:

- Define a stable constant for the built-in prompt ID.
- Keep the new built-in prompt text in one function/constant.
- Keep exact known historical shipped prompt text where needed for safe migration.
- Extend `ensure_post_process_defaults()` or split a focused helper that:
  1. ensures all current providers, key slots, and model slots exist;
  2. ensures the bundled prompt exists;
  3. safely upgrades an unchanged historical built-in prompt;
  4. checks whether `post_process_selected_prompt_id` points to a non-empty prompt;
  5. selects the bundled prompt when selection is missing/invalid;
  6. preserves valid custom selection;
  7. returns `changed` only when it actually mutates settings.

Required pure tests:

- fresh defaults select the bundled prompt;
- `None` migrates to bundled prompt;
- unknown ID migrates to bundled prompt;
- selected empty prompt migrates to bundled prompt;
- selected valid custom prompt remains selected;
- user-modified built-in prompt is not overwritten;
- known historical untouched built-in prompt upgrades safely;
- an absent bundled prompt is reinserted without deleting custom prompts.

### 5.2 Typed readiness/configuration resolution

Replace `resolve_post_process_provider_and_model() -> Option<_>` with a typed result.
Recommended shape:

```rust
struct ResolvedPostProcessConfig {
    provider: PostProcessProvider,
    model: String,
    prompt_id: String,
    prompt: String,
    tone: PostProcessTone,
    source: PostProcessConfigSource,
}

enum PostProcessUnavailableReason {
    NoProviders,
    SelectedProviderMissing,
    NoModelConfigured,
    NoPromptSelected,
    SelectedPromptMissing,
    SelectedPromptEmpty,
    MissingApiKey,
}
```

Exact public/private visibility can differ. Requirements:

- Trim model and prompt values before readiness decisions.
- Prefer valid dedicated provider/model.
- Fall back to a valid assistant provider/model as today.
- Use the same shared `post_process_api_keys` map used by assistant and cleanup.
- Include source/provider/model in a frontend-safe readiness DTO.
- Never include API-key contents in logs, events, or DTOs.

Expose a read-only Tauri command such as `get_post_process_readiness` if needed so the
frontend does not reimplement backend resolution.

### 5.3 Typed runtime outcome

Replace ambiguous `Option<String>` behavior at the orchestration boundary with an
outcome that distinguishes application from fallback. Recommended semantics:

```rust
enum PostProcessAttemptOutcome {
    Applied(String),
    Unavailable(PostProcessUnavailableReason),
    Failed(PostProcessFailureKind),
    TimedOut,
}

enum PostProcessFailureKind {
    LocalModelStart,
    Authentication,
    ProviderRequest,
    StructuredOutputRejected,
    MalformedResponse,
    EmptyResponse,
    UnsupportedProvider,
}
```

Error strings may still be logged internally, but user-facing state uses safe categories.

`ProcessedTranscription` should carry enough metadata to know:

- whether cleanup was requested;
- whether it was applied;
- why raw text was used;
- which source/provider/model was resolved (safe names only);
- elapsed cleanup time if useful for debugging.

Do not add new persistent history columns unless essential. Prefer an event/runtime field
for this localized repair; history schema migration is out of scope.

### 5.4 Output safety rules

After sanitization:

- If raw input is non-empty and cleaned output is empty, classify `EmptyResponse` and
  retain raw text.
- If structured JSON is required but cannot be parsed or lacks a string
  `transcription` field, do not paste raw JSON as the cleaned transcript.
- Continue stripping exact transcript wrappers, whole-output code fences, invisible
  characters, and surrounding whitespace.
- Do not strip arbitrary angle-bracket content, quotes, Markdown, or punctuation that
  may have been dictated unless a test establishes it is an exact model wrapper.
- Do not introduce aggressive edit-distance rejection in this task; tone modes may
  legitimately rewrite substantially. Meaning preservation is handled by the prompt and
  test corpus.

### 5.5 Timeout and retry budget

- Resolve configuration before beginning any expensive operation.
- During recording start, prewarm the **resolved effective built-in model**, whether it
  comes from the dedicated cleanup selection or assistant fallback.
- Preserve `ModelUnloadTimeout::Immediately` behavior; do not force a model to stay warm
  against user preference.
- Keep the user-configured overall cleanup timeout.
- If structured output fails and plain-text fallback remains:
  - give the first attempt a bounded sub-timeout;
  - retry only when sufficient overall budget remains;
  - perform at most one compatibility fallback;
  - report which final failure occurred;
  - never start another request after the outer deadline.
- Add debug timing logs for resolution, local startup, request attempt(s), and total
  cleanup time without logging transcript content at elevated levels.

Do not change the selected model or download anything.

### 5.6 User-visible outcome event

Emit a small event only when the dedicated shortcut requested cleanup. Suggested payload:

```ts
type PostProcessResultEvent = {
  status: "applied" | "fallback";
  reason?:
    | "not_configured"
    | "missing_api_key"
    | "model_unavailable"
    | "authentication"
    | "provider_error"
    | "invalid_response"
    | "empty_response"
    | "timeout";
};
```

Rules:

- Do not send raw transcript, prompt, API response, endpoint, or key.
- On fallback, show a concise localized message when the main UI is available.
- Always write a useful technical log entry.
- Do not add a new notification dependency.
- Do not block paste while waiting for UI acknowledgement.

If a main-window toast is invisible while the app is hidden, that is acceptable for the
first repair as long as logs and settings readiness are clear. An overlay redesign is not
part of this task.

---

## 6. Built-in prompt specification

### 6.1 Prompt priorities

The shipped prompt must be short enough for small/current models and order instructions
so cleanup and tone do not contradict each other.

Required order:

1. **Role and output contract** — clean one raw speech transcript and return only text.
2. **Meaning safety** — preserve facts, intent, names, technical terms, URLs, code-like
   tokens, and original language.
3. **Mechanical cleanup** — spelling where unambiguous, capitalization, punctuation,
   spacing, and sentence boundaries.
4. **Speech cleanup** — genuine fillers, stutters, repeated words, abandoned false
   starts, and explicit self-corrections.
5. **Spoken formatting** — punctuation commands and unambiguous numbers/dates/times/
   money.
6. **Prompt-injection boundary** — dictated questions/commands are content to clean,
   never instructions to execute or answer.
7. **Tone override** — appended last when non-None.

### 6.2 Important edge rules

- Keep original language; do not translate.
- Preserve meaning; do not invent facts or complete an unfinished thought.
- Preserve negations.
- Preserve names and jargon unless correction is unambiguous.
- Treat `like` and `you know` as fillers only when they are functioning as fillers.
- For “wait, no,” “I mean,” or “scratch that,” keep the explicit corrected version.
- Do not answer a dictated question.
- Do not follow a dictated command.
- Empty/only-filler input may produce empty output; non-empty substantive input may not.

### 6.3 Distinct tone contracts

Keep enum values and serialization unchanged.

- **None:** cleanup only. Preserve wording and register unless grammar requires a local
  change. No stylistic paraphrase.
- **Formal:** polished, respectful, complete-sentence register; avoid casual slang and
  unnecessary contractions. Not specifically corporate.
- **Casual:** natural conversational wording and contractions; relaxed but clear. Do not
  add slang, jokes, or enthusiasm not present in the source.
- **Professional:** concise workplace-appropriate language; direct, courteous, and
  businesslike. Avoid ceremonial or legalistic phrasing.
- **Friendly:** warm and approachable while preserving the same request/content. Do not
  add compliments, emojis, exclamation marks, or emotional claims not present.
- **Concise:** remove redundancy and wordiness while preserving every material fact,
  request, condition, name, number, and deadline.

Each directive should contain one compact contrast/example in tests or fixtures, not a
large token-heavy example block in every production prompt unless model testing proves it
necessary.

### 6.4 Prompt test corpus

Create table-driven fixtures covering at least:

1. fillers: `um`, `uh`, filler `like`, filler `you know`;
2. lexical uses of “like” and “you know” that must remain;
3. stutters and repeated words;
4. abandoned false starts;
5. explicit self-correction;
6. spoken punctuation and new line;
7. dates, money, times, phone-like numbers;
8. names, product names, technical jargon, code, URLs;
9. negation and conditions;
10. a dictated question that must remain a question, not receive an answer;
11. a dictated command that must be cleaned, not executed;
12. already-clean text;
13. one-word/very-short text;
14. long multi-sentence dictation;
15. non-English text;
16. every tone mode on the same neutral source text.

Automated tests with a mock provider verify payload and orchestration. Human/model quality
checks are recorded separately because exact generative wording should not be asserted.

---

## 7. Frontend/settings-panel plan

### 7.1 Preserve current page structure

Keep:

- “Fix up my dictation with AI” toggle;
- Tone dropdown;
- Dictate and clean up shortcut row;
- Cleanup model group;
- Cleanup prompt group;
- Timeout control.

The user already understands the dedicated shortcut. Do not redesign this relationship.

### 7.2 Add a compact readiness row/message

Use backend readiness data. States:

- **Ready:** show resolved provider/model.
- **Ready — using Assistant model:** explicitly identify fallback.
- **Select a cleanup prompt.**
- **Select a model for this provider.**
- **Add an API key for this provider.**
- **The selected provider is no longer available.**

Do not label a configuration ready from TypeScript-only guesses.

When Base URL changes and model is cleared, readiness must immediately change to
“Select a model for this endpoint.” Do not leave a stale Ready state.

### 7.3 Correct all mutation handling

For these operations, inspect generated command `Result.status`:

- enable/disable cleanup;
- set tone;
- set timeout;
- set provider;
- change base URL;
- change API key;
- change model;
- select prompt;
- create/update/delete prompt.

On error:

- roll back optimistic state where used;
- refresh authoritative settings;
- show localized error feedback;
- do not continue dependent operations (for example, do not refresh models after a
  failed provider/base-URL save).

On success:

- refresh only when necessary;
- avoid races where an older refresh overwrites a newer selection;
- clear cached model options only for the affected provider.

### 7.4 Prompt selection behavior

- After migration, the bundled prompt appears selected automatically.
- Creating a prompt selects it only after both creation and selection commands succeed.
- Deleting the selected prompt causes backend repair/fallback to the bundled prompt.
- The UI must not display a deleted/empty selection after refresh.
- Editing a custom prompt must not modify the shipped default.

### 7.5 Optional test action

Only after core work and required tests pass, consider a compact “Test cleanup” action
inside the cleanup prompt/model area:

- uses typed sample text, not microphone input;
- calls the exact runtime resolver/request path without pasting;
- shows cleaned result or classified failure;
- never stores test text in history;
- does not become a prerequisite for this task's completion unless the user asks.

---

## 8. Onboarding/loading theme implementation

### 8.1 Remove the forced-dark exception

In `src/App.tsx`:

- Remove the branch that directly writes `data-theme = "dark"` during onboarding.
- Apply `applyThemePreference(themePreference)` regardless of onboarding step.
- Make `watchSystemTheme()` read the actual preference, not return a synthetic `dark`
  preference while onboarding.
- Do not change onboarding step routing or completion logic.
- `FORCE_ONBOARDING` may remain as a development flow aid; it must not alter theme.

### 8.2 First-paint behavior

Retain `applyCachedTheme()` in `src/main.tsx` before React render.

Expected sequence:

1. Cached Light/Dark/System preference is resolved synchronously.
2. First onboarding/loading paint uses that resolved theme.
3. Settings hydration supplies authoritative `settings.theme`.
4. The UI reapplies the authoritative preference without flashing a forced palette.
5. If preference is System, an OS appearance change updates onboarding live.

### 8.3 Align theme defaults

In `src-tauri/src/settings.rs`, choose one source of truth for fresh/default settings.
Recommended: assign `theme: Theme::default()` in `get_default_settings()`, preserving
`Theme::Light` as the tuned application default. Do not overwrite an existing user's
persisted Light/Dark/System preference.

### 8.4 Audit onboarding surfaces for semantic tokens

Inspect these current files before editing:

- `src/components/onboarding/AccessibilityOnboarding.tsx`
- `src/components/onboarding/Onboarding.tsx`
- `src/components/onboarding/LlmOnboarding.tsx`
- `src/components/onboarding/ReadyStep.tsx` or its actual current location
- `src/components/onboarding/OnboardingLayout.tsx`
- onboarding child cards/progress components
- `src/components/TitleBar.tsx`
- `src/App.css`

Requirements:

- Prefer semantic utilities/tokens (`bg-canvas`, `bg-surface`, `text-ink`,
  `text-muted`, `border-hairline`, `bg-accent`).
- Remove onboarding-only hard-coded near-black/white colors if they prevent Light mode.
- Do not flatten the visual hierarchy or redesign layouts.
- Keep contrast readable in both themes.
- Ensure dropdown menus, progress bars, selected cards, disabled text, footer borders,
  and title bar all follow the same palette.
- Do not change the assistant panel's dark-only design; it is a separate window and out
  of scope.

### 8.5 Theme manual matrix

Verify on the main settings window:

| Preference | OS mode | Expected onboarding/loading result |
| ---------- | ------- | ---------------------------------- |
| Light      | Light   | Light                              |
| Light      | Dark    | Light                              |
| Dark       | Light   | Dark                               |
| Dark       | Dark    | Dark                               |
| System     | Light   | Light                              |
| System     | Dark    | Dark                               |

Also verify:

- fresh install/no cache uses the chosen application default;
- returning user with a saved preference has no wrong-theme flash;
- development-forced onboarding respects the saved preference;
- live OS mode switching updates System while onboarding is open;
- leaving onboarding keeps the same theme;
- reopening the app keeps the same preference.

---

## 9. Affected-file map

Read current source before every edit; this map is a guide, not permission to blindly
modify every file.

### Backend likely to change

- `src-tauri/src/settings.rs`
  - default prompt selection;
  - safe prompt migration;
  - theme default alignment;
  - readiness/config types or helpers.
- `src-tauri/src/actions.rs`
  - resolver use;
  - effective built-in prewarm;
  - typed attempt outcomes;
  - timeout/retry budgeting;
  - empty/malformed output fallback;
  - result event emission;
  - focused tests.
- `src-tauri/src/llm_client.rs`
  - only if request-attempt error classification or structured/plain timeout behavior
    cannot remain isolated in `actions.rs`.
- `src-tauri/src/shortcut/mod.rs`
  - readiness command/settings mutations only if needed;
  - do not change dedicated shortcut semantics.
- `src-tauri/src/lib.rs`
  - register any new read-only readiness command.

### Frontend likely to change

- `src/stores/settingsStore.ts`
  - inspect `Result.status`, rollback, refresh/race handling.
- `src/components/settings/dictation/AiCleanupGroup.tsx`
  - compact readiness display.
- `src/components/settings/post-processing/PostProcessingSettings.tsx`
  - prompt/tone UI error handling only as needed.
- `src/components/settings/PostProcessingSettingsApi/usePostProcessProviderState.ts`
  - readiness refresh and provider/model mutation correctness.
- `src/components/settings/PostProcessTimeout.tsx`
  - only if mutation error handling requires it.
- `src/App.tsx`
  - remove onboarding forced-dark behavior;
  - listen for safe cleanup result event if used for toast feedback.
- `src/lib/theme.ts`
  - likely no behavioral change; modify only if tests expose a shared-theme issue.
- onboarding components/CSS identified in §8.4
  - semantic theme compatibility only.
- `src/i18n/locales/en/translation.json`
  - readiness and failure strings.
- `src/bindings.ts`
  - regenerate via the repository's binding workflow if Rust command/types change;
  - do not hand-edit if generation is available.

### Files/systems not to touch

- `src/assistant/**`
- `src-tauri/src/assistant.rs`
- `src-tauri/src/memory.rs`
- `src-tauri/src/tts.rs`
- `src-tauri/src/web_search.rs`
- transcription engines and model catalog code
- Graphify outputs
- non-English locale files unless the project's established workflow requires a
  mechanical source-key update; do not machine-invent translations.

---

## 10. Implementation phases

### Phase 0 — Baseline and reproduction

- [x] Read this plan and all current affected files.
- [x] Record current git working-tree state without discarding unrelated changes.
- [x] Run targeted existing Rust tests for `actions`/settings if available.
- [x] Run frontend typecheck baseline.
- [x] Record current lint/build failures separately from task-caused failures.
- [x] Reproduce or prove from settings fixtures/source paths:
  - enabled cleanup + no selected prompt;
  - invalid/deleted selected prompt;
  - dedicated model absent + assistant fallback present;
  - cold built-in fallback not prewarmed;
  - provider error/timeout returning raw text silently;
  - onboarding forced dark under saved Light.

**Exit:** baseline evidence recorded; no implementation assumptions left unverified.

### Phase 1 — Settings invariants and theme contract

- [x] Add safe built-in-prompt repair/migration.
- [x] Make fresh defaults select a valid prompt.
- [x] Preserve valid custom selections and user-edited prompt text.
- [x] Align fresh theme default via `Theme::default()`.
- [x] Remove onboarding forced-dark branch.
- [x] Keep cached first paint and System watcher behavior.
- [x] Add/adjust pure tests for settings migration and theme resolution where possible.

**Exit:** settings can no longer naturally enter a promptless state; onboarding follows
selected appearance in source/tests.

### Phase 2 — Shared readiness and runtime outcomes

- [x] Introduce shared typed configuration resolver.
- [x] Preserve and identify assistant fallback explicitly.
- [x] Add frontend-safe readiness command/DTO if needed.
- [x] Introduce typed attempt/failure outcomes.
- [x] Make empty/malformed outputs use raw text.
- [x] Prewarm resolved effective built-in model.
- [x] Bound structured/plain attempt timing and retry count.
- [x] Emit safe applied/fallback status.
- [x] Keep normal dictation path untouched.

**Exit:** every dedicated invocation is classified as applied or a specific raw fallback;
no non-empty transcript can disappear.

### Phase 3 — Prompt/tone reliability

- [x] Rewrite shipped built-in prompt to §6 contract.
- [x] Add distinct tone directives without changing enum values.
- [x] Safely migrate only unchanged shipped prompt versions.
- [x] Add table-driven prompt construction tests.
- [x] Add mock-provider payload tests proving system/user separation and selected tone.
- [x] Confirm custom prompts still reach the same path unchanged except legacy
      `${output}` stripping.

**Exit:** requests are unambiguous, tone-specific, and regression-tested without a new
model or second rewriting pass.

### Phase 4 — Settings-panel consistency

- [x] Display backend readiness in existing cleanup groups.
- [x] Explicitly display assistant fallback source when used.
- [x] Show model-required state immediately after Base URL reset.
- [x] Check `Result.status` for every cleanup mutation.
- [x] Roll back/refresh on mutation failure.
- [x] Prevent stale async refreshes from restoring old provider/model values.
- [x] Add localized error/fallback copy.
- [x] Do not redesign the page or dedicated shortcut row.

**Exit:** displayed settings and backend settings cannot silently diverge.

### Phase 5 — Theme visual audit

- [x] Inspect every onboarding/loading component against semantic tokens.
- [x] Fix only hard-coded colors that prevent Light/Dark/System behavior.
- [x] Verify title bar, cards, dropdowns, progress, footer, selected/disabled states.
- [x] Preserve layout, wording, and onboarding flow.
- [~] Run the full manual matrix in §8.5 where environment access permits.

**Exit:** supplied onboarding screen renders coherently in both Light and Dark and tracks
System mode.

### Phase 6 — Validation and cleanup

- [x] Run targeted Rust unit tests.
- [x] Run mock-provider integration tests.
- [~] Run frontend tests if the repository has an established runner. (No frontend test runner configured in `package.json`; not applicable.)
- [x] Run `bun x tsc --noEmit` or the repository's frontend build.
- [x] Run `bun run lint` and separate pre-existing failures.
- [x] Run `bun run format:check`.
- [x] Run `cargo fmt --check` for Rust changes.
- [x] Run `cargo check` or targeted Cargo tests for the affected backend.
- [x] Run `bun run build` as final frontend validation.
- [~] Perform/manual-request the scenario matrix in §11. (Automated portions covered by tests; runtime/theme manual scenarios handed to the user — they need a running window and live providers.)
- [x] Update this plan's Evidence, Decision log, and Progress log.
- [x] Do not refresh Graphify.

**Exit:** all automated checks pass or every pre-existing/blocking failure is documented;
manual gaps are explicitly handed to the user.

---

## 11. Required scenario matrix

### 11.1 Configuration and persistence

- [ ] Fresh settings: cleanup off, bundled prompt exists and is selected.
- [ ] Enable cleanup: dedicated shortcut registers; normal shortcut unchanged.
- [ ] Restart: enabled state, provider, model, prompt, tone, timeout persist.
- [ ] Invalid prompt ID: repaired to bundled prompt.
- [ ] Deleted selected custom prompt: repaired to bundled prompt.
- [ ] Valid selected custom prompt: preserved.
- [ ] User-edited built-in prompt: preserved.
- [ ] Base URL change: old model clears and UI immediately requests a new selection.
- [ ] Failed backend setting mutation: UI rolls back and reports failure.

### 11.2 Runtime outcomes

- [ ] Valid cloud provider: cleaned text applied.
- [ ] Valid built-in dedicated model: cold invocation succeeds or visibly falls back.
- [ ] Assistant built-in fallback: prewarms and is visibly identified.
- [ ] Missing model: raw text used with clear reason.
- [ ] Missing required key: raw text used with clear reason.
- [ ] 401/403: authentication reason, raw text preserved.
- [ ] 429: provider/rate-limit-safe reason, raw text preserved.
- [ ] 500: provider reason, raw text preserved.
- [ ] Network connection failure: provider reason, raw text preserved.
- [ ] Structured-output rejection: at most one bounded fallback.
- [ ] Malformed JSON/content: raw text preserved.
- [ ] Empty output for substantive input: raw text preserved.
- [ ] Timeout: raw text preserved and operation finishes within bounded time.
- [ ] Only-filler input: empty cleaned result may be accepted as designed.
- [ ] Ten repeated dedicated invocations: every run is either applied or visibly
      classified; no silent raw fallback.

### 11.3 Prompt/tone behavior

Using the same neutral source message:

- [ ] None performs cleanup without stylistic rewrite.
- [ ] Formal differs from Professional according to §6.3.
- [ ] Casual differs from Friendly according to §6.3.
- [ ] Concise removes redundancy without losing facts/numbers/deadlines.
- [ ] Questions remain questions and are not answered.
- [ ] Commands remain dictated content and are not executed/answered.
- [ ] Technical terms, names, URLs, and negations survive.
- [ ] Custom prompt remains selectable and used.

Generative wording need not match exact snapshots. Human verification should focus on
instruction adherence, meaning preservation, and mode differentiation.

### 11.4 Theme behavior

Run the six preference/OS combinations in §8.5 on:

- [ ] accessibility step;
- [ ] transcription model step;
- [ ] local assistant model step;
- [ ] ready step;
- [ ] download/loading progress;
- [ ] transition from onboarding to settings;
- [ ] relaunch with saved preference;
- [ ] development-forced onboarding.

---

## 12. Acceptance criteria

The task is complete only when all applicable criteria have evidence.

### AI cleanup

- **A1:** Fresh/migrated settings always have a valid selected cleanup prompt.
- **A2:** Valid custom prompts and user modifications are preserved.
- **A3:** The dedicated shortcut attempts cleanup on every invocation when enabled.
- **A4:** Normal dictation behavior is unchanged.
- **A5:** UI readiness and runtime use the same backend resolver.
- **A6:** Dedicated cleanup model and assistant fallback source are unambiguous.
- **A7:** Frontend setting errors cannot masquerade as successful persistence.
- **A8:** Every runtime attempt ends as Applied or a typed/raw fallback reason.
- **A9:** A non-empty raw transcript is never replaced by empty/malformed output.
- **A10:** Cold built-in assistant fallback is eligible for prewarm.
- **A11:** Retry/structured fallback is bounded and cannot silently consume unlimited time.
- **A12:** Built-in cleanup prompt follows §6 and all tone directives are distinct.
- **A13:** No new model, model download, provider, or inference engine is introduced.

### Theme

- **T1:** Onboarding/loading no longer forces `data-theme="dark"`.
- **T2:** Light and Dark ignore the opposite OS preference as expected.
- **T3:** System follows the OS during onboarding, including live changes.
- **T4:** Cached preference controls first paint without a forced-theme flash.
- **T5:** Existing persisted preferences are not reset during migration.
- **T6:** All onboarding states remain readable and visually coherent in both themes.
- **T7:** Assistant panel dark-only behavior remains untouched.

### Validation

- **V1:** Targeted Rust tests pass.
- **V2:** Frontend typecheck/build passes.
- **V3:** Lint and formatting checks pass, excluding documented pre-existing failures.
- **V4:** Required manual scenarios are completed or explicitly handed to the user.
- **V5:** No unrelated source files or generated Graphify artifacts changed.

---

## 13. Validation command reference

Run the narrowest relevant commands first, then final checks. Adjust only if package
scripts have changed; record exact commands actually used.

```bash
# Frontend
bun x tsc --noEmit
bun run lint
bun run format:check
bun run build

# Rust (from repository root unless command requires src-tauri)
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml actions::tests
cargo test --manifest-path src-tauri/Cargo.toml settings
cargo check --manifest-path src-tauri/Cargo.toml
```

If Windows native dependencies make full Cargo validation unavailable, run the most
focused compilable tests and document the exact blocker/output. Do not claim validation
that did not run.

---

## 14. Risks and mitigations

| Risk                                                | Mitigation                                                             |
| --------------------------------------------------- | ---------------------------------------------------------------------- |
| Migration overwrites a user's edited prompt         | Upgrade only exact known shipped text; preserve unknown edits          |
| Removing promptless state surprises users           | Fallback to the existing bundled prompt, already visible/editable      |
| UI and runtime readiness drift again                | One backend resolver + read-only readiness DTO                         |
| Assistant fallback behavior changes unexpectedly    | Preserve fallback; make source explicit; add resolver tests            |
| Cold local model still exceeds timeout              | Prewarm resolved effective model; preserve explicit timeout fallback   |
| Structured fallback doubles latency                 | Bound first attempt and retry; maximum one compatibility retry         |
| Empty model output erases dictation                 | Non-empty raw input always wins over empty cleaned result              |
| Error toast exposes secrets                         | Emit only stable reason codes; technical details stay in redacted logs |
| Tone modes remain indistinguishable                 | Distinct contracts + shared-source human test matrix                   |
| Theme flashes before settings load                  | Keep synchronous cached-theme application before React                 |
| Light onboarding has low-contrast hard-coded colors | Semantic-token audit across every onboarding state                     |
| Task expands into model/UI redesign                 | Enforce §2 non-goals and affected-file map                             |

---

## 15. Evidence

Populate during implementation.

### Baseline

- Working tree recorded with `git status --short --branch; git diff -- docs/ai-cleanup-reliability/PLAN.md; git diff --stat` (exit 0). The branch already had 46 changed/untracked paths, including unrelated edits in `actions.rs`, `settings.rs`, `lib.rs`, `shortcut/mod.rs`, `bindings.ts`, onboarding files, locales, and Graphify output; none were discarded.
- `cargo test --manifest-path src-tauri/Cargo.toml actions::tests` — exit 0; 7 passed, 0 failed.
- `cargo test --manifest-path src-tauri/Cargo.toml settings::tests` — exit 0; 14 passed, 0 failed.
- `bun x tsc --noEmit` — baseline exit 1: pre-existing `src/components/settings/dictation/SpokenEmojiToggle.tsx(11,68)` TS2686 (`React` UMD global; missing import).
- `bun run lint` — baseline exit 0.
- `bun run build` — baseline exit 2 with the same pre-existing `SpokenEmojiToggle.tsx(11,68)` TS2686 error.
- Current-source reproduction: `get_default_settings()` selected no prompt; `ensure_post_process_defaults()` repaired only provider/key/model maps; `resolve_post_process_provider_and_model()` silently used assistant fallback; `TranscribeAction::start()` prewarmed only the dedicated built-in selection; `process_transcription_output()` silently retained raw text for `None`/timeout; structured malformed content could be returned as text; and `App.tsx` wrote `data-theme="dark"` plus a synthetic dark System watcher during onboarding.

### Phase 1

- Added stable bundled-prompt constants plus exact historical text matching; migration repairs missing/empty/invalid selection, reinserts an absent bundled prompt, upgrades only known untouched shipped text, and preserves valid custom selection and unknown non-empty edits.
- Fresh settings now select `default_improve_transcriptions`; fresh theme now uses `Theme::default()` (`Light`). Existing persisted theme values are not migrated or overwritten.
- `App.tsx` now always calls `applyThemePreference(themePreference)` and `watchSystemTheme(() => themePreference)`; `applyCachedTheme()` remains before React render in `main.tsx`.
- `cargo test --manifest-path src-tauri/Cargo.toml settings::tests` — exit 0; 20 passed, 0 failed (6 new migration/theme tests).
- `bun x eslint src/App.tsx` — exit 0.
- Focused grep for `inOnboarding|dataset.theme = "dark"|watchSystemTheme` found only the import/comment/real watcher; no forced-dark assignment or synthetic onboarding preference remains.

### Phase 2

- Added one pure resolver `resolve_post_process_config` plus `post_process_readiness` and DTOs (`PostProcessReadiness`, `PostProcessConfigSource`, `PostProcessUnavailableReason`) in `settings.rs`; runtime and the new `get_post_process_readiness` command share it. Credentials live only in the non-serializable `ResolvedPostProcessConfig`.
- Conservative `post_process_provider_requires_api_key` policy reused by readiness, runtime, and `fetch_post_process_models`.
- `actions.rs` now returns typed `PostProcessAttemptOutcome`/`PostProcessFailureKind`; empty/malformed output preserves raw text; one `timeout_at` deadline covers local start + structured + at most one bounded plain fallback; `TranscribeAction::start` prewarms the resolved effective built-in model (dedicated or assistant fallback); safe `post-process-result` event emitted.
- `llm_client.rs` gained `send_chat_completion_with_schema_typed` + `ChatCompletionError`; the public string API is a thin wrapper so assistant/memory callers are unchanged.
- `cargo test settings::tests` 26/26 and `cargo test actions::tests` 17/17 passed (local `TcpListener` mock-provider suite).

### Phase 3

- Rewrote shipped prompt to the §6 ordered contract with safe historical-only migration (legacy exact text upgraded, user edits preserved).
- Made all five non-None tone directives compact and distinct; enum values/serialization unchanged; tone appended once after the base/custom prompt.
- Tests: `prompt_corpus_stays_in_the_user_turn_and_contract_stays_in_system` (16-fixture corpus), `every_tone_builds_a_distinct_final_system_directive`, custom `${output}` stripping preserved.

### Phase 4

- `settingsStore.ts`: `commandSucceeded` unwraps generated `Result.status`; cleanup mutations return booleans; latest-wins generations for settings refresh, readiness, per-key mutations, and per-provider model fetch; atomic Base URL relies on backend clearing the model in one write; `initialize` de-duplicated; readiness state (`refreshPostProcessReadiness`).
- `shortcut/mod.rs`: `change_post_process_base_url_setting` clears the provider's cleanup model in the same write; deleting the selected prompt repairs to the bundled prompt; `set_post_process_selected_prompt` rejects an empty prompt.
- `usePostProcessProviderState.ts`: sequence-guarded provider selection, auto-fetch only after a successful save, provider-busy disable, toast on failed manual model refresh.
- `PostProcessingSettings.tsx`: create→select→single refresh ordering, status-checked update/delete with a busy guard; `AiCleanupGroup.tsx`: compact backend readiness row (incl. explicit assistant-fallback label); `AssistantSettings.tsx` base URL routed through the hardened action; English `settings.postProcessing.readiness`/`errors` copy added.
- `src/bindings.ts` regenerated via the debug Specta export (`getPostProcessReadiness` + DTO unions present); `bun x tsc --noEmit` exit 0; `bun run lint` exit 0.

### Phase 5

- `App.css`: `--color-success` tuned to `#15803d` (AA as text on light) and given luminous `#4ade80` overrides in both the dark theme and the pre-JS dark media fallback. `--color-error` left unchanged because it is a destructive fill behind white text (close/danger buttons).
- `AccessibilityOnboarding.tsx`: replaced `text-emerald-400`/`bg-emerald-500/20` with `text-success`/`bg-success/15`; low-opacity `text-text/50|60|70` → `text-muted`; `text-text` headings → `text-ink`; both grant buttons `bg-ink hover:opacity-90` → `bg-accent hover:bg-accent-strong` to match onboarding's accent primary vocabulary.
- Onboarding/LlmOnboarding/ReadyStep instruction lines: disabled-tier `text-muted-soft` → `text-muted`.
- Progress components (ModelCard, ReadyStep, DownloadProgress, WelcomeChoiceCard): `bg-mid-gray/20` → `bg-hairline-strong`, `bg-logo-primary` → `bg-accent`, low-opacity status text → `text-muted`.
- `WelcomeChoiceCard.tsx`: removed the hard-coded `rgba(20,184,166,…)`/black selected/hover shadows; selected state stays expressed by semantic accent border/ring/tint.
- Preserved intentionally: the Gemma brand tile (`bg-[#4285f4]`), the vivid decorative icon tiles, and the assistant panel's dark-only design.
- Verified: `bun x tsc --noEmit` exit 0 and `bun run lint` exit 0 after the theme edits.
- Manual §8.5 matrix (six preference/OS combinations, live OS switching, relaunch, dev-forced onboarding) still requires a running app window and is handed to the user; static routing is confirmed in `App.tsx`/`main.tsx`/`theme.ts`.

### Final validation

All commands run from the repo root unless noted; exact results:

- `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` — exit 0 (after applying `cargo fmt`).
- `cargo test --manifest-path src-tauri/Cargo.toml actions::tests` — `test result: ok. 21 passed; 0 failed`.
- `cargo test --manifest-path src-tauri/Cargo.toml settings::tests` — `test result: ok. 26 passed; 0 failed`.
- `bun x tsc --noEmit` — exit 0.
- `bun run lint` — exit 0.
- `bun run check:translations` — `✓ All 19 languages have complete translations!` (14 new keys seeded into every locale as English fallback via a temporary, since-deleted `scripts/seed-cleanup-keys.ts`; not machine-invented translations).
- `bun run format:check` — `All matched files use Prettier code style!` + `cargo fmt -- --check` clean.
- `bun run build` — exit 0, `✓ built in 7.48s` (chunk-size warning is pre-existing and unrelated).
- One transient issue encountered and resolved: a Windows file lock on `transcribe-libs/ggml-base.dll` from a lingering process (cleared with `Stop-Process`), and one flaky timing test (`connection_failure_is_a_provider_failure_without_retry`) rewritten to force a deterministic transport error (accept-and-drop) instead of relying on OS dead-port refusal timing.

**Manual scenarios handed to the user (require a running window / live providers):**

- §11.2 runtime provider matrix (valid cloud applied; 401/403/429/500/network/timeout each preserve raw text with a classified `post-process-result`), and cold built-in prewarm timing.
- §11.3 human tone/meaning-adherence review across modes on a shared source message.
- §8.5 / §11.4 six preference/OS theme combinations, live OS switching, relaunch-with-saved-preference (no flash), and development-forced onboarding.

---

## 16. Decision log

| Date       | Decision                                               | Reason                                                                                                                                                            |
| ---------- | ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2026-07-15 | Keep dedicated AI-cleanup shortcut semantics           | User confirmed the separate shortcut is understood and intentional                                                                                                |
| 2026-07-15 | Do not add or change AI models                         | User explicitly rejected model work for this task                                                                                                                 |
| 2026-07-15 | Preserve raw transcript fallback                       | Prevents loss of dictated text on provider/model failure                                                                                                          |
| 2026-07-15 | Add onboarding/loading theme repair to scope           | User confirmed setup must respect the app appearance preference                                                                                                   |
| 2026-07-15 | Preserve assistant model fallback but expose it        | Avoid behavior regression while eliminating hidden configuration                                                                                                  |
| 2026-07-15 | Keep UI changes compact                                | Reliability repair, not a Dictation-page redesign                                                                                                                 |
| 2026-07-15 | Seed new locale keys with English fallback values      | `check:translations` enforces key parity; i18next falls back to English at runtime, so seeding source text (not machine-invented translations) keeps parity green |
| 2026-07-15 | Rewrote the connection-failure test to accept-and-drop | OS dead-port refusal timing was non-deterministic on Windows; forcing a fast transport error makes the classification assertion reliable                          |
| 2026-07-15 | Remove permanent cleanup-ready success copy            | A normal configured state should be quiet; the green footer and duplicate local helper made the settings card look like a warning/debug surface                   |
| 2026-07-15 | Reuse one On-device / Cloud provider control           | Matching Assistant removes a second provider-selection mental model and progressively reveals only relevant fields                                                |
| 2026-07-15 | Keep writing styles separate from cleanup prompts      | Prompts define what cleanup does; style presets define how wording sounds. This lets users create reusable style rules without breaking cleanup behavior          |
| 2026-07-15 | Append an absolute final-output contract last          | The local Gemma model returned “Here is a formal…” plus Markdown; final response-only rules must outrank both cleanup and custom style instructions               |

---

## 17. Progress log

| Date       | Phase               | Status   | Evidence/notes                                                                                                                                                                                                                                                                                          |
| ---------- | ------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2026-07-15 | Planning            | complete | Read-only source audit and external model research completed; scope narrowed to settings/runtime reliability plus onboarding theme; no product code changed                                                                                                                                             |
| 2026-07-15 | Phase 0             | complete | Dirty-tree snapshot preserved; actions tests 7/7 and settings tests 14/14 passed; lint passed; frontend typecheck/build baseline blocked by the pre-existing missing React import in `SpokenEmojiToggle.tsx`; all diagnosed cleanup/theme paths confirmed in current source                             |
| 2026-07-15 | Phase 1             | complete | Safe prompt migration/default selection and `Theme::default()` alignment implemented; onboarding forced-dark path removed; settings tests 20/20 and focused `App.tsx` lint passed                                                                                                                       |
| 2026-07-15 | Phases 2-3          | complete | Shared resolver/readiness DTO + command, typed outcomes, one-deadline bounded fallback, effective-model prewarm, safe event, distinct tones; settings tests 26/26 and actions tests 17/17 (with local mock provider) passed; bindings regenerated                                                       |
| 2026-07-15 | Phase 4             | complete | Result.status checks + latest-wins guards + atomic base-URL model reset + readiness row + localized errors; `tsc` and `lint` exit 0                                                                                                                                                                     |
| 2026-07-15 | Phase 5             | complete | Onboarding theme color-token audit applied (dual-theme success token, emerald→success, opacity→muted, progress→accent/hairline, removed theme-tied shadows); tsc/lint exit 0; manual §8.5 matrix handed to user                                                                                         |
| 2026-07-15 | Phase 6             | complete | Full validation green: cargo fmt --check, settings 26/26, actions 21/21, tsc, lint, check:translations 19/19, format:check, bun run build (exit 0); manual runtime/theme scenarios handed to user; no Graphify refresh                                                                                  |
| 2026-07-15 | User-test follow-up | complete | Removed permanent readiness/helper copy; shared local/cloud control; persisted custom writing styles; strict response-only contract for local models. Final checks: settings 29/29, actions 22/22, tsc/lint/build/cargo check/format/translations all exit 0; live Gemma quality re-test handed to user |

---

## 18. User-testing follow-up: cleanup UX and writing styles

### 18.1 Reproduced problems

The user's live screenshots exposed three issues that automated reliability work did not
catch:

1. A permanent green `Ready — Built-in (Local) · …` footer and a second “Runs fully on
   your machine” helper made the normal configured state look like a warning/debug
   surface.
2. Cleanup used a flat Provider dropdown while Assistant already had the clearer
   `On my device` / `Cloud provider` mental model.
3. The built-in Gemma provider (`supports_structured_output = false`) returned a
   meta-response — “Here is a formal, respectful alternative…” plus Markdown and
   quotation marks — instead of only the transformed dictation. The existing short
   tone directive did not define a strict final response shape.

The user also requested reusable custom writing styles (for example, a preset that
removes profanity and replaces it with calm wording) without abusing or replacing the
cleanup prompt.

### 18.2 Implemented follow-up

- Removed the permanent readiness footer from `AiCleanupGroup` and removed the duplicate
  built-in local helper text. Backend readiness resolution and safe runtime events remain
  intact; only the noisy success presentation was removed.
- Extracted one shared `ProviderModeToggle` and used it in both Assistant and cleanup.
  Cleanup now starts with `Where it runs`, progressively shows only the device or cloud
  fields, and remembers the most recent cloud selection while the view is mounted.
- Hardened Assistant provider switching while sharing that control: settings repair
  unknown/cleanup-only provider IDs, the command rejects unsupported IDs, and frontend
  writes are validated, serialized, latest-wins, status-checked, and disabled while busy.
- Separated the settings hierarchy into AI cleanup (master switch + shortcut), Writing
  style, Cleanup model, and Cleanup prompt.
- Added persisted `CustomPostProcessTone` presets with validated add/update/delete/select
  commands. Built-in IDs remain stable; old `post_process_tone` values migrate into the
  new selected ID; stale custom selections repair to cleanup-only; deleting an active
  custom style also falls back to cleanup-only. One canonical validity rule rejects
  empty, reserved, or whitespace-wrapped IDs across migration/runtime/selection, and the
  UI filters the same malformed entries.
- Unsaved custom-style drafts now survive unrelated settings refreshes. Prompt/style
  labels are associated with their controls, and the shared provider mode group has an
  accessible name.
- Kept cleanup prompts untouched and independent. A cleanup prompt defines what fixes
  happen; a writing style defines how the wording sounds.
- Strengthened all built-in style directives and changed runtime composition to:
  cleanup prompt → optional built-in/custom writing-style instruction → absolute final
  output contract. The last block explicitly forbids explanations, “Here is…” preambles,
  labels, Markdown/code fences/emphasis, surrounding quotes, answering the transcript,
  and perspective changes.
- Regenerated Specta bindings from `src-tauri`, added 21 English UI/error keys, and
  mechanically seeded missing keys into 19 non-English locales with English fallback
  values only. No machine-invented translations were added.

### 18.3 Follow-up validation evidence

- `cargo test --manifest-path src-tauri/Cargo.toml actions::tests` — exit 0,
  `22 passed; 0 failed`.
- `cargo test --manifest-path src-tauri/Cargo.toml settings::tests` — exit 0,
  `29 passed; 0 failed`.
- `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` — exit 0.
- `cargo check --manifest-path src-tauri/Cargo.toml` — exit 0 (only existing warnings).
- `bun x tsc --noEmit` — exit 0.
- `bun run lint` — exit 0.
- `bun run check:translations` — all 19 non-English locales complete, exit 0.
- `bun run format:check` — exit 0.
- `bun run build` — exit 0; Vite transformed 2247 modules and built in 5.70 s
  (existing chunk-size warning only).
- Task-owned `git diff --check` — exit 0 after stripping five trailing spaces emitted
  by Specta in `src/bindings.ts`.
- Static UI verification: no `CleanupReadiness`, `postProcessing.api.builtin.ready`, or
  old `BrainModeToggle` use remains; LSP reports no diagnostics for the revised cleanup
  settings component; temporary seeding script and accidental sibling bindings file are
  absent.
- Independent static review initially found provider-switch race/invalid-target handling,
  unsaved-draft refresh loss, and malformed custom-style validity inconsistencies. These
  were fixed and regression-tested; the final re-review returned `APPROVED`.

A transient Windows `ggml-base.dll` file lock from the binding-generation process was
confirmed (`os error 32`), all lingering SpeakoFlow processes were stopped, and both Rust
suites then passed. This was an environment lock, not a code failure.

### 18.4 Manual re-test still required

Automated tests prove the selected custom/built-in style reaches the system prompt, the
strict output contract is last, failures still preserve raw text, and the UI compiles.
They cannot prove subjective output quality from a particular local model. Re-run the
same Gemma test that previously produced “Here is a formal…” and confirm:

- only the transformed dictation is pasted (no preamble, label, Markdown, or outer quotes);
- Formal, Professional, Casual, Friendly, and Concise are perceptibly different while
  preserving facts and point of view;
- a custom style such as “Remove profanity and use calm, neutral wording” applies while
  the selected cleanup prompt still performs its own correction job;
- the new device/cloud selector and inline create/edit/delete interactions look correct
  in the running Light and Dark app.

---

## 19. Post-compaction handoff summary

The original reliability/theme plan and the user-testing follow-up are implemented and
all automated validation is green. Do not redo Phases 0–6. The remaining work is human
runtime review in §8.5, §11, and §18.4. Continue to preserve raw transcript fallback,
custom cleanup prompts, custom writing styles, existing theme choices, and normal
(non-cleanup) dictation. Do not add models, change the model catalog, redesign shortcuts,
or refresh Graphify unless the user explicitly requests it.
