//! "Generate with Flow" — the spoken-command generation path.
//!
//! When the feature is enabled and a normal dictation begins with the
//! configured activation phrase (default "Hey Flow"), the rest of the
//! dictation becomes a one-shot generation command. The finished result is
//! pasted into the active application instead of the spoken words.
//!
//! Design constraints (see the feature spec):
//! - Stateless: no assistant history, personas, memory, or response-length
//!   settings participate. One system prompt + the spoken command, per turn.
//! - Same model: reuses the assistant's provider/model/key — no second
//!   provider configuration.
//! - All-or-nothing: the caller pastes only a successful, non-empty result.
//!   Errors, timeouts, and empty output paste nothing.
//! - Optional screen access: when `flow_screen_access` is on, the model may
//!   call a `capture_screen` tool once per command; the model decides whether
//!   the command actually needs it.

use log::{debug, error, warn};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::llm_client;
use crate::llm_client::ReasoningConfig;
use crate::settings::{AppSettings, PostProcessProvider};

/// Stable internal marker stored with recording-history rows created by Flow.
/// Keep this value unchanged: the frontend also uses it to classify older
/// entries that were saved before Flow had its own History filter.
pub const FLOW_HISTORY_MARKER: &str = "Generate with Flow";

/// Upper bound for one Flow generation (all rounds, AFTER the engine is up).
/// Generous compared to AI cleanup because Flow writes whole artifacts, but
/// still bounded so a stalled provider can never hold the dictation pipeline.
const FLOW_TIMEOUT_SECS: u64 = 90;

/// Separate budget for starting the built-in engine and loading the model
/// from disk. Kept OUTSIDE the generation timeout: a big local model's cold
/// load could otherwise eat the whole budget and fail every first-use command
/// ("Flow couldn't generate") before a single token was written.
const FLOW_ENGINE_START_TIMEOUT_SECS: u64 = 150;

/// How long the prewarm holds the model resident after loading it, bridging
/// the gap between "phrase heard mid-speech" and "generation turn begins".
/// Without this, an "Unload from memory: Immediately" setting evicts the
/// prewarmed model the moment it finishes loading — making the prewarm
/// pointless. The generation turn takes its own activity guard; if no turn
/// arrives (cancelled dictation), the bridge expires and the model unloads
/// per the user's setting.
const FLOW_PREWARM_BRIDGE_SECS: u64 = 60;

/// Tool-round cap: one optional capture round plus the answer, with one spare
/// round so a confused model can still recover to a final answer.
const MAX_FLOW_TOOL_ROUNDS: usize = 3;

/// Once-per-recording guard for the early prewarm (and its "stop watching"
/// latch once the transcript can no longer begin with the phrase). Re-armed
/// at every dictation start.
static PREWARM_DONE: AtomicBool = AtomicBool::new(true);

/// Monotonic cancellation generation for Flow. Each Escape/cancel action bumps
/// it; an in-flight generation keeps the value it started with and aborts as
/// soon as the global value changes. A generation counter avoids stale cancel
/// flags leaking into the next command.
static FLOW_CANCEL_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Whether a Flow turn is currently inside model startup or generation. This
/// lets the global Escape handler route cancellation after recording has ended.
static FLOW_GENERATION_ACTIVE: AtomicBool = AtomicBool::new(false);

struct FlowGenerationActivityGuard;

impl FlowGenerationActivityGuard {
    fn begin() -> Self {
        FLOW_GENERATION_ACTIVE.store(true, Ordering::SeqCst);
        Self
    }
}

impl Drop for FlowGenerationActivityGuard {
    fn drop(&mut self) {
        FLOW_GENERATION_ACTIVE.store(false, Ordering::SeqCst);
    }
}

/// Whether Escape should currently be allowed to cancel a Flow turn.
pub fn is_generation_active() -> bool {
    FLOW_GENERATION_ACTIVE.load(Ordering::SeqCst)
}

/// Snapshot the current cancellation generation before starting a Flow turn.
pub fn cancellation_generation() -> u64 {
    FLOW_CANCEL_GENERATION.load(Ordering::SeqCst)
}

/// Cancel any in-flight Flow turn. No-op for future turns because they snapshot
/// the incremented generation when they start.
pub fn cancel_generation() {
    FLOW_CANCEL_GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// Whether a Flow turn that started at `generation` has since been cancelled.
pub fn is_generation_cancelled(generation: u64) -> bool {
    cancellation_generation() != generation
}

async fn wait_for_generation_cancel(generation: u64) {
    while !is_generation_cancelled(generation) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Committed live text longer than this can no longer *begin* with any
/// reasonable activation phrase — stop checking for the rest of the recording.
const PREWARM_WATCH_LIMIT: usize = 64;

/// Re-arm the live-transcript prewarm watcher. Called when a normal dictation
/// recording starts.
pub fn reset_prewarm_watch() {
    PREWARM_DONE.store(false, Ordering::SeqCst);
}

/// Stop watching live text. Called when the recording finishes or is cancelled
/// so another recording mode cannot inherit an armed watcher.
pub fn stop_prewarm_watch() {
    PREWARM_DONE.store(true, Ordering::SeqCst);
}

/// Leading filler words tolerated before the activation phrase in RAW live
/// text. (The final transcript has fillers filtered out before the strict
/// matcher runs; the live stream does not.)
const LEADING_FILLERS: [&str; 10] = [
    "um", "uh", "er", "ah", "hmm", "so", "okay", "ok", "well", "yeah",
];

/// One phrase word heard leniently: exact, or same first letter within edit
/// distance 2 — tolerant enough for "Flow" heard as "Flo" or "Flu".
fn lenient_token_match(expected: &str, heard: &str) -> bool {
    expected == heard
        || (expected.chars().next() == heard.chars().next()
            && strsim::levenshtein(expected, heard) <= 2)
}

/// Lenient phrase check for the prewarm watcher ONLY. The live stream shows
/// raw recognition ("Uh hey Flu, would you…") before filler filtering and
/// hint correction run, so the strict matcher would miss most real spoken
/// activations. A false positive merely warms a model that goes unused, so
/// tolerance is deliberately loose: up to two leading fillers are skipped and
/// each phrase word may be a near-miss spelling. Real activation still uses
/// the strict matcher on the corrected final transcript.
fn lenient_phrase_leads(transcript: &str, phrase: &str) -> bool {
    let phrase_tokens: Vec<String> = phrase
        .split_whitespace()
        .map(normalize_token)
        .filter(|t| !t.is_empty())
        .collect();
    if phrase_tokens.is_empty() {
        return false;
    }
    let words: Vec<String> = transcript
        .split_whitespace()
        .map(normalize_token)
        .filter(|w| !w.is_empty())
        .take(phrase_tokens.len() + 3)
        .collect();

    let max_skip = 2.min(words.len());
    for start in 0..=max_skip {
        if words[..start]
            .iter()
            .any(|w| !LEADING_FILLERS.contains(&w.as_str()))
        {
            break; // a real word precedes the phrase — it can't lead anymore
        }
        if words.len() - start < phrase_tokens.len() {
            continue; // not enough words yet; a later event may bring more
        }
        if phrase_tokens
            .iter()
            .zip(&words[start..])
            .all(|(expected, heard)| lenient_token_match(expected, heard))
        {
            return true;
        }
    }
    false
}

/// Watch the committed live transcript for the activation phrase and warm the
/// built-in local model the moment it is heard, so generation starts instantly
/// when the user finishes speaking. Fire-and-forget by design: a failed or
/// unfinished prewarm changes nothing — `run_flow_generation` performs its own
/// `ensure_running` and surfaces real errors through the normal Flow notice.
/// No-op for cloud providers (nothing to load) and for batch STT models (no
/// live text ever arrives).
pub fn note_live_transcript(app: &AppHandle, committed: &str) {
    if committed.is_empty() || PREWARM_DONE.load(Ordering::SeqCst) {
        return;
    }
    let settings = crate::settings::get_settings(app);
    if !settings.flow_enabled {
        PREWARM_DONE.store(true, Ordering::SeqCst);
        return;
    }
    if !lenient_phrase_leads(committed, &settings.flow_phrase) {
        // The phrase must LEAD the dictation; once the committed text is
        // clearly past any phrase-sized prefix, stop re-checking.
        if committed.len() > PREWARM_WATCH_LIMIT {
            PREWARM_DONE.store(true, Ordering::SeqCst);
        }
        return;
    }
    if PREWARM_DONE.swap(true, Ordering::SeqCst) {
        return; // lost a race with another event — already handled
    }
    let Some(llm) = resolve_flow_llm(&settings) else {
        return;
    };
    if llm.provider.id != "builtin" {
        debug!(
            "Flow: phrase heard, but provider '{}' has nothing to prewarm",
            llm.provider.id
        );
        return; // cloud / external server: nothing to warm
    }
    // Info-level on purpose: this is the observable proof the early warmup
    // fired, visible in the log file without debug mode.
    log::info!(
        "Flow: activation phrase heard in live transcript; prewarming '{}'",
        llm.model
    );
    let manager = app
        .state::<Arc<crate::managers::local_llm::LocalLlmManager>>()
        .inner()
        .clone();
    tauri::async_runtime::spawn(async move {
        // The bridge guard keeps the idle watcher off the model through the
        // load and for a short window afterwards, so it is still resident
        // when the generation turn starts — even with "Unload: Immediately".
        let bridge_guard = manager.begin_request();
        if let Err(e) = manager.ensure_running(&llm.model).await {
            debug!("Flow prewarm failed (generation will retry): {}", e);
            return;
        }
        tokio::time::sleep(Duration::from_secs(FLOW_PREWARM_BRIDGE_SECS)).await;
        drop(bridge_guard);
    });
}

/// What the dictation pipeline should do with a finished transcription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowPlan {
    /// Not a Flow command — continue as ordinary dictation.
    NotFlow,
    /// The phrase matched but no assistant provider/model is configured.
    /// Continue as ordinary dictation, but let the user know why.
    Unconfigured,
    /// The user said only the activation phrase, with no command after it.
    EmptyCommand,
    /// Generate: the phrase matched and Flow is ready to run.
    Generate { command: String },
}

/// The assistant provider/model/key Flow reuses. Mirrors the resolution in
/// `assistant::run_assistant_turn` (same settings fields, no extra config).
pub struct FlowLlm {
    pub provider: PostProcessProvider,
    pub model: String,
    pub api_key: String,
}

/// Resolve the assistant's active provider + model for Flow. `None` when no
/// provider is selected or it has no model configured — Flow then falls back
/// to ordinary dictation rather than failing the paste.
pub fn resolve_flow_llm(settings: &AppSettings) -> Option<FlowLlm> {
    let provider = settings.active_assistant_provider()?.clone();
    let model = settings
        .assistant_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        return None;
    }
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    Some(FlowLlm {
        provider,
        model,
        api_key,
    })
}

/// Decide what to do with a finished dictation transcription.
pub fn plan_flow(settings: &AppSettings, transcription: &str) -> FlowPlan {
    if !settings.flow_enabled {
        return FlowPlan::NotFlow;
    }
    let Some(command) = strip_activation_phrase(transcription, &settings.flow_phrase) else {
        return FlowPlan::NotFlow;
    };
    if resolve_flow_llm(settings).is_none() {
        return FlowPlan::Unconfigured;
    }
    if command.is_empty() {
        return FlowPlan::EmptyCommand;
    }
    FlowPlan::Generate { command }
}

/// Lowercase a spoken token and drop punctuation, so "Hey," matches "hey".
fn normalize_token(token: &str) -> String {
    token
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// One activation-phrase word, matched the way wake words should be: by
/// sound, not spelling. "Flow" and "Flo" are the same spoken word — the STT
/// engine merely picks a spelling — so requiring exact orthography would make
/// activation depend on the model's spelling mood (and unfairly punish
/// non-native speakers). Accepted: exact match, or a homophone-level miss —
/// same first letter, same Soundex code, edit distance ≤ 2, and near-equal
/// length (the length guard keeps real words like "follow" from matching
/// "flow").
fn phrase_token_matches(expected: &str, heard: &str) -> bool {
    if expected == heard {
        return true;
    }
    expected.chars().next() == heard.chars().next()
        && (expected.len() as i32 - heard.len() as i32).abs() <= 1
        && strsim::levenshtein(expected, heard) <= 2
        && natural::phonetics::soundex(expected, heard)
}

/// Walk the leading activation phrase over `transcript`. On a match, returns
/// the byte range from the start of the first matched token to the end of the
/// last matched token (including punctuation glued to it, e.g. `"Hey Flo,"`).
fn match_leading_phrase(transcript: &str, phrase: &str) -> Option<(usize, usize)> {
    let phrase_tokens: Vec<String> = phrase
        .split_whitespace()
        .map(normalize_token)
        .filter(|t| !t.is_empty())
        .collect();
    if phrase_tokens.is_empty() {
        return None;
    }

    let mut cursor = 0usize;
    let mut matched = 0usize;
    let mut match_start: Option<usize> = None;
    while matched < phrase_tokens.len() {
        let rest = &transcript[cursor..];
        let offset = rest.find(|c: char| !c.is_whitespace())?;
        let start = cursor + offset;
        let end = transcript[start..]
            .find(char::is_whitespace)
            .map(|i| i + start)
            .unwrap_or(transcript.len());
        let word = normalize_token(&transcript[start..end]);
        cursor = end;
        if word.is_empty() {
            // Pure punctuation between words — skip it.
            continue;
        }
        if !phrase_token_matches(&phrase_tokens[matched], &word) {
            return None;
        }
        if match_start.is_none() {
            match_start = Some(start);
        }
        matched += 1;
    }
    Some((match_start?, cursor))
}

/// If `transcript` begins with the activation `phrase` (case-, punctuation-,
/// and homophone-spelling-insensitive, word by word), return the remaining
/// command text with the phrase and any separating punctuation removed. The
/// command keeps its original casing and formatting. `None` when the phrase
/// doesn't lead.
pub fn strip_activation_phrase(transcript: &str, phrase: &str) -> Option<String> {
    let (_, end) = match_leading_phrase(transcript, phrase)?;
    let command = transcript[end..]
        .trim_start_matches(|c: char| {
            c.is_whitespace() || matches!(c, ',' | '.' | '!' | '?' | ':' | ';' | '-' | '—' | '–')
        })
        .trim_end();
    Some(command.to_string())
}

/// Display-side canonicalizer: if `transcript` begins with the activation
/// phrase in ANY tolerated spelling ("Hey Flo", "hey flu", "Hey, Flo"),
/// rewrite exactly that leading span to the configured spelling — preserving
/// punctuation glued to the last phrase word and every byte after it. This is
/// surgical on purpose: unlike the fuzzy corrector's greedy n-grams, it can
/// never absorb or drop neighboring words. Returns `None` when the phrase
/// doesn't lead or is already spelled canonically.
pub fn canonicalize_leading_phrase(transcript: &str, phrase: &str) -> Option<String> {
    let phrase = phrase.trim();
    if phrase.is_empty() {
        return None;
    }
    let (start, end) = match_leading_phrase(transcript, phrase)?;
    let matched = &transcript[start..end];
    // Punctuation stuck to the last phrase word ("Flo," → ",") survives.
    let suffix_start = matched
        .char_indices()
        .rev()
        .take_while(|(_, c)| !c.is_alphanumeric())
        .map(|(i, _)| i)
        .last()
        .unwrap_or(matched.len());
    let replacement = format!("{}{}", phrase, &matched[suffix_start..]);
    if matched == replacement {
        return None;
    }
    let mut out = String::with_capacity(transcript.len() + phrase.len());
    out.push_str(&transcript[..start]);
    out.push_str(&replacement);
    out.push_str(&transcript[end..]);
    Some(out)
}

/// The stateless Flow system prompt. Everything the model outputs is pasted
/// verbatim, so the prompt forbids conversational framing entirely.
fn flow_system_prompt(screen_tool: bool) -> String {
    let mut s = String::from(
        "You are Flow, a silent text generator inside a dictation tool. The user spoke ONE command. \
         Your entire response is pasted directly into the application they are working in, exactly \
         as-is. It is never read as a chat message — there is no conversation.\n\
         \n\
         OUTPUT CONTRACT (absolute):\n\
         - Respond with the finished content and NOTHING else. The first character of your response \
         is the first character of the content; the last character is its last.\n\
         - Forbidden: greetings, preambles (\"Sure\", \"Here is\", \"Certainly\", \"Okay\"), explanations \
         of what you did or why, closing remarks, offers to help further, apologies, disclaimers, and \
         any meta-commentary.\n\
         - Forbidden: visible reasoning, thinking tags, notes to self, or multiple drafts. Produce only \
         the single final version.\n\
         - NEVER ask for clarification. If details are missing, pick sensible, neutral specifics and \
         write the best complete result anyway.\n\
         \n\
         FORMATTING:\n\
         - Use real structure where the content calls for it: paragraphs, line breaks, lists, tables, \
         headings, indentation.\n\
         - Plain prose (emails, messages, letters, paragraphs) gets no Markdown syntax unless the \
         command asks for Markdown.\n\
         - Code requests: output raw code only — correct indentation, no surrounding code fence, no \
         comments explaining the code (unless asked). Add a fence only when the command explicitly \
         asks for Markdown.\n\
         - Never wrap the whole output in quotation marks or a code fence.\n\
         \n\
         CONTENT:\n\
         - Match the shape and length to the request: a short reply stays short; a document gets \
         structure. Do not pad.\n\
         - Emails and letters: complete and ready to send — greeting, body, sign-off. Include a \
         subject line only when one is clearly useful. Never use placeholders like [Your Name] or \
         [Date]; if something is unknowable, phrase around it naturally.\n\
         - Rewrites and replies: keep every fact, name, number, and commitment from the source; \
         change only what the command asks.\n\
         - Write in the language the command was spoken in, unless it says otherwise.\n",
    );
    if screen_tool {
        s.push_str(
            "\nTOOL:\n\
             You may call capture_screen() ONCE to see the user's current screen. Call it ONLY when the \
             command clearly refers to something on screen (\"this email\", \"the text above\", \"look at\", \
             \"reply to this\", \"summarize this page\"). For self-contained commands, generate directly \
             without capturing. After looking, the output contract still applies: only the finished \
             content, nothing about the screenshot.\n",
        );
    }
    s
}

/// Make a raw model response paste-safe, or reject it.
///
/// Models occasionally disobey the output contract in ways that would paste
/// garbage: leaked reasoning (`<think>…</think>`) or the whole answer wrapped
/// in one Markdown code fence. This strips what can be stripped and returns
/// `None` only when nothing remains — symbol-only output can be intentional.
fn sanitize_flow_output(raw: &str) -> Option<String> {
    let text = strip_reasoning_blocks(raw.trim());
    let text = unwrap_full_code_fence(text.trim());
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.to_string())
}

/// Remove leaked reasoning blocks (`<think>`, `<thinking>`, `<reasoning>`,
/// case-insensitive). A block that never closes swallows the rest of the text
/// — an unfinished thought is reasoning, not content.
fn strip_reasoning_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // ASCII lowercase preserves byte offsets exactly (the tags are ASCII), so
    // indices found in `lower` are valid for `text`.
    let lower = text.to_ascii_lowercase();
    let mut pos = 0usize;
    while pos < text.len() {
        // Find the next reasoning-open tag at or after `pos`.
        let next = ["<think>", "<thinking>", "<reasoning>"]
            .iter()
            .filter_map(|tag| lower[pos..].find(*tag).map(|i| (pos + i, *tag)))
            .min_by_key(|(i, _)| *i);
        let Some((start, tag)) = next else {
            out.push_str(&text[pos..]);
            break;
        };
        out.push_str(&text[pos..start]);
        // `tag` is like "<think>", so this yields the full "</think>".
        let close = format!("</{}", &tag[1..]);
        match lower[start..].find(&close) {
            Some(rel) => pos = start + rel + close.len(),
            None => break, // unclosed: drop the rest
        }
    }
    out
}

/// If the ENTIRE output is a single Markdown code fence, unwrap it — the
/// prompt asks for raw content, so a lone wrapper fence is framing, not
/// formatting. Output with interior fences (real mixed Markdown) is left
/// untouched.
fn unwrap_full_code_fence(text: &str) -> &str {
    let t = text.trim();
    if !t.starts_with("```") {
        return t;
    }
    let Some(first_newline) = t.find('\n') else {
        return t;
    };
    let tail = t[first_newline + 1..].trim_end();
    let Some(body) = tail.strip_suffix("```") else {
        return t;
    };
    // The closing fence must sit on its own line.
    if !(body.is_empty() || body.ends_with('\n')) {
        return t;
    }
    // An interior fence line means this is real Markdown, not one wrapper.
    if body
        .lines()
        .any(|line| line.trim_start().starts_with("```"))
    {
        return t;
    }
    body.trim_end_matches('\n')
}

/// Reasoning suppression, mirroring AI cleanup's provider matrix. Flow is a
/// paste, not a chat: a reasoning model left on its defaults can burn most of
/// the budget "thinking" (felt as a long stall) or return reasoning-only
/// content with an empty answer — both showed up as "Flow couldn't generate"
/// on API providers. "custom" gateways get the OpenAI-style knob; OpenRouter
/// gets its native reasoning config.
fn flow_reasoning_options(provider_id: &str) -> (Option<String>, Option<ReasoningConfig>) {
    match provider_id {
        "custom" => (Some("none".to_string()), None),
        "openrouter" => (
            None,
            Some(ReasoningConfig {
                effort: Some("none".to_string()),
                exclude: Some(true),
            }),
        ),
        _ => (None, None),
    }
}

/// OpenAI-style tool definitions for a Flow turn: just `capture_screen`.
fn flow_tool_defs() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "capture_screen",
                "description": "Take one screenshot of the user's current screen. Use it only when the command refers to something visible on screen. May be called at most once.",
                "parameters": { "type": "object", "properties": {} }
            }
        }
    ])
}

/// Take a full-screen capture sized for this provider, off the async runtime.
async fn capture_screen_for(provider: &PostProcessProvider) -> Result<String, String> {
    let profile = crate::screenshot::CaptureProfile::for_base_url(&provider.base_url);
    tauri::async_runtime::spawn_blocking(move || {
        crate::screenshot::capture_screen_data_url_at(None, profile)
    })
    .await
    .map_err(|e| format!("Screen capture task failed: {}", e))?
}

/// Run one Flow generation end to end. Returns the paste-ready text, or an
/// error when nothing should be pasted. Engine start is bounded by
/// `FLOW_ENGINE_START_TIMEOUT_SECS`; the generation itself by
/// `FLOW_TIMEOUT_SECS`.
pub async fn run_flow_generation(
    app: &AppHandle,
    command: &str,
    cancel_generation: u64,
) -> Result<String, String> {
    if is_generation_cancelled(cancel_generation) {
        return Err("Flow generation cancelled".to_string());
    }
    let _activity_guard = FlowGenerationActivityGuard::begin();

    let settings = crate::settings::get_settings(app);
    let llm = resolve_flow_llm(&settings).ok_or_else(|| "Flow is not configured".to_string())?;
    let allow_screen = settings.flow_screen_access;
    debug!(
        "Flow generation: provider '{}', model '{}', screen access {}",
        llm.provider.id, llm.model, allow_screen
    );

    // Built-in engine: start it and load the model OUTSIDE the generation
    // timeout, under its own budget — a cold multi-GB load must never eat the
    // writing time. The activity guard is taken BEFORE the load and held for
    // the whole turn, so the idle watcher (even "Unload: Immediately") cannot
    // evict the model between load and generation.
    let _llm_activity_guard = if llm.provider.id == "builtin" {
        let manager = app
            .state::<Arc<crate::managers::local_llm::LocalLlmManager>>()
            .inner()
            .clone();
        let guard = manager.begin_request();
        let start_result = tokio::select! {
            biased;
            _ = wait_for_generation_cancel(cancel_generation) => {
                return Err("Flow generation cancelled".to_string());
            }
            result = tokio::time::timeout(
                Duration::from_secs(FLOW_ENGINE_START_TIMEOUT_SECS),
                manager.ensure_running(&llm.model),
            ) => result,
        };
        start_result
            .map_err(|_| "Local model took too long to start".to_string())?
            .map_err(|e| format!("Local model failed to start: {}", e))?;
        Some(guard)
    } else {
        None
    };

    let generation = async {
        let mut messages: Vec<Value> = vec![
            json!({"role": "system", "content": flow_system_prompt(allow_screen)}),
            json!({"role": "user", "content": command}),
        ];
        let (reasoning_effort, reasoning) = flow_reasoning_options(&llm.provider.id);

        if !allow_screen {
            // Plain one-shot stream (tokens are ignored; only the final text
            // matters — nothing is pasted until the whole result exists).
            return llm_client::send_chat_stream(
                &llm.provider,
                llm.api_key.clone(),
                &llm.model,
                messages,
                reasoning_effort,
                reasoning,
                |_| {},
            )
            .await;
        }

        // Screen-permitted turn: expose the capture_screen tool and let the
        // model decide. At most one screenshot per command.
        let tools = flow_tool_defs();
        let mut captured = false;
        let mut last_text = String::new();
        for round in 0..MAX_FLOW_TOOL_ROUNDS {
            let round_result = llm_client::send_chat_stream_with_tools(
                &llm.provider,
                llm.api_key.clone(),
                &llm.model,
                messages.clone(),
                tools.clone(),
                json!("auto"),
                reasoning_effort.clone(),
                reasoning.clone(),
                |_| {},
            )
            .await;
            let out = match round_result {
                Ok(out) => out,
                // Some OpenAI-compatible gateways reject any request carrying
                // `tools`. On the FIRST round the conversation is still
                // pristine, so retry once as a plain generation instead of
                // failing the whole command over an unsupported feature.
                Err(e) if round == 0 => {
                    warn!("Flow tool round failed ({}); retrying without tools", e);
                    return llm_client::send_chat_stream(
                        &llm.provider,
                        llm.api_key.clone(),
                        &llm.model,
                        messages,
                        reasoning_effort,
                        reasoning,
                        |_| {},
                    )
                    .await;
                }
                Err(e) => return Err(e),
            };
            if out.tool_calls.is_empty() {
                return Ok(out.text);
            }
            last_text = out.text.clone();
            if round + 1 >= MAX_FLOW_TOOL_ROUNDS {
                break;
            }

            let tool_calls_json: Vec<Value> = out
                .tool_calls
                .iter()
                .map(|tc| {
                    json!({
                        "id": tc.id,
                        "type": "function",
                        "function": { "name": tc.name, "arguments": tc.arguments }
                    })
                })
                .collect();
            messages.push(json!({
                "role": "assistant",
                "content": out.text,
                "tool_calls": tool_calls_json
            }));

            for tc in &out.tool_calls {
                if tc.name == "capture_screen" && !captured {
                    captured = true;
                    // Briefly tell the user Flow is looking at the screen.
                    crate::overlay::show_vision_overlay(app);
                    let result = capture_screen_for(&llm.provider).await;
                    crate::overlay::show_generating_overlay(app);
                    match result {
                        Ok(data_url) => {
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": tc.id,
                                "content": "Screenshot captured. It is attached in the next message."
                            }));
                            messages.push(json!({
                                "role": "user",
                                "content": [
                                    {"type": "text", "text": "[Screenshot of the current screen, from the capture_screen tool. Use it to fulfill the command, then output only the finished content.]"},
                                    {"type": "image_url", "image_url": {"url": data_url}}
                                ]
                            }));
                        }
                        Err(e) => {
                            error!("Flow screen capture failed: {}", e);
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": tc.id,
                                "content": "Screen capture failed. Produce the best result you can without it."
                            }));
                        }
                    }
                } else {
                    let note = if tc.name == "capture_screen" {
                        "A screenshot was already captured for this command. Produce the final content now."
                    } else {
                        "Unknown tool. Produce the final content now."
                    };
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": note
                    }));
                }
            }
        }
        // Round cap reached: fall back to whatever text the last round carried.
        Ok(last_text)
    };

    let text = tokio::select! {
        biased;
        _ = wait_for_generation_cancel(cancel_generation) => {
            return Err("Flow generation cancelled".to_string());
        }
        result = tokio::time::timeout(Duration::from_secs(FLOW_TIMEOUT_SECS), generation) => {
            result
                .map_err(|_| "Flow generation timed out".to_string())??
        }
    };

    if is_generation_cancelled(cancel_generation) {
        return Err("Flow generation cancelled".to_string());
    }

    // Paste-safety pass: strip leaked reasoning and a lone wrapper fence, then
    // reject output only if nothing remains.
    match sanitize_flow_output(&text) {
        Some(clean) => Ok(clean),
        None => {
            warn!("Flow generation returned no usable content");
            Err("Flow returned no usable content".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_prewarm_matches_raw_live_speech() {
        // The two real-world cases from the field: leading filler + near-miss
        // spelling of "Flow".
        assert!(lenient_phrase_leads(
            "Uh hey Flu, would you mind writing a mail",
            "Hey Flow"
        ));
        assert!(lenient_phrase_leads(
            "Um hey Flo so I want you to write me a mail",
            "Hey Flow"
        ));
        assert!(lenient_phrase_leads("hey flow write a poem", "Hey Flow"));
        assert!(lenient_phrase_leads("Um, so hey flow do this", "Hey Flow"));
    }

    #[test]
    fn lenient_prewarm_ignores_unrelated_speech() {
        assert!(!lenient_phrase_leads("Hello there my friend", "Hey Flow"));
        assert!(!lenient_phrase_leads(
            "I said hey flow yesterday",
            "Hey Flow"
        ));
        assert!(!lenient_phrase_leads(
            "Please write this down now",
            "Hey Flow"
        ));
        assert!(!lenient_phrase_leads("", "Hey Flow"));
        assert!(!lenient_phrase_leads("hey flow", ""));
    }

    #[test]
    fn sanitize_passes_normal_content_through() {
        assert_eq!(
            sanitize_flow_output("Dear Sam,\n\nSee you Monday.\nBest,\nAlex"),
            Some("Dear Sam,\n\nSee you Monday.\nBest,\nAlex".to_string())
        );
        // Interior formatting (including a real fenced block inside larger
        // Markdown) is preserved untouched.
        let mixed = "Intro paragraph.\n\n```py\nprint(1)\n```\n\nOutro.";
        assert_eq!(sanitize_flow_output(mixed), Some(mixed.to_string()));
    }

    #[test]
    fn sanitize_rejects_empty_or_empty_wrapper_output() {
        assert_eq!(sanitize_flow_output(""), None);
        assert_eq!(sanitize_flow_output("   \n\n\n \t \n"), None);
        assert_eq!(sanitize_flow_output("```\n\n```"), None);
    }

    #[test]
    fn sanitize_preserves_symbol_only_content() {
        assert_eq!(
            sanitize_flow_output("...\n---\n\"\"\n"),
            Some("...\n---\n\"\"".to_string())
        );
        assert_eq!(sanitize_flow_output("✅"), Some("✅".to_string()));
        assert_eq!(sanitize_flow_output("{}"), Some("{}".to_string()));
    }

    #[test]
    fn sanitize_strips_leaked_reasoning() {
        assert_eq!(
            sanitize_flow_output(
                "<think>The user wants a poem. I should rhyme.</think>\nWaves rise and fall."
            ),
            Some("Waves rise and fall.".to_string())
        );
        // Unclosed reasoning swallows the rest (it is thought, not content).
        assert_eq!(
            sanitize_flow_output("<thinking>hmm this is hard and I never stop"),
            None
        );
        // Reasoning-only output is a failure, not a paste.
        assert_eq!(sanitize_flow_output("<think>only thoughts</think>"), None);
    }

    #[test]
    fn sanitize_unwraps_a_lone_wrapper_fence() {
        assert_eq!(
            sanitize_flow_output("```\nfn main() {}\n```"),
            Some("fn main() {}".to_string())
        );
        assert_eq!(
            sanitize_flow_output("```python\nprint(\"hi\")\n```"),
            Some("print(\"hi\")".to_string())
        );
        // A fence that doesn't wrap the whole output stays as-is.
        let partial = "Some text\n```\ncode\n```";
        assert_eq!(sanitize_flow_output(partial), Some(partial.to_string()));
    }

    #[test]
    fn canonicalize_fixes_leading_phrase_spelling_only() {
        // The exact field case: homophone spelling + glued comma + untouched rest.
        assert_eq!(
            canonicalize_leading_phrase(
                "Hey Flo, I was thinking about not going to college tomorrow. Um",
                "Hey Flow"
            ),
            Some("Hey Flow, I was thinking about not going to college tomorrow. Um".to_string())
        );
        assert_eq!(
            canonicalize_leading_phrase("hey flu write a poem", "Hey Flow"),
            Some("Hey Flow write a poem".to_string())
        );
        // Bare phrase, no command yet (mid-recording).
        assert_eq!(
            canonicalize_leading_phrase("Hey Flo.", "Hey Flow"),
            Some("Hey Flow.".to_string())
        );
    }

    #[test]
    fn canonicalize_leaves_everything_else_alone() {
        // Already canonical → no rewrite churn.
        assert_eq!(
            canonicalize_leading_phrase("Hey Flow, write an email", "Hey Flow"),
            None
        );
        // Not a leading phrase → untouched.
        assert_eq!(
            canonicalize_leading_phrase("I said hey flow yesterday", "Hey Flow"),
            None
        );
        assert_eq!(
            canonicalize_leading_phrase("Hello there my friend", "Hey Flow"),
            None
        );
        assert_eq!(canonicalize_leading_phrase("anything", ""), None);
    }

    #[test]
    fn phrase_matches_homophone_spellings() {
        // "Flo"/"FLO" are the same spoken word as "Flow" — the STT engine
        // just picks a spelling. Activation must not depend on orthography.
        assert_eq!(
            strip_activation_phrase("Hey Flo, write an email.", "Hey Flow"),
            Some("write an email.".to_string())
        );
        assert_eq!(
            strip_activation_phrase("hey FLO write an email", "Hey Flow"),
            Some("write an email".to_string())
        );
        assert_eq!(
            strip_activation_phrase("Hey flo.", "Hey Flow"),
            Some(String::new())
        );
    }

    #[test]
    fn phrase_homophone_tolerance_has_limits() {
        // Real different words must not activate: "follow" fails the length
        // guard, "floor" fails Soundex, "blow" fails the first letter.
        assert_eq!(
            strip_activation_phrase("Hey follow the instructions", "Hey Flow"),
            None
        );
        assert_eq!(
            strip_activation_phrase("Hey floor plans are ready", "Hey Flow"),
            None
        );
        assert_eq!(
            strip_activation_phrase("Hey blow out the candles", "Hey Flow"),
            None
        );
    }

    #[test]
    fn phrase_matches_with_punctuation_and_case() {
        assert_eq!(
            strip_activation_phrase("Hey Flow, write an email.", "Hey Flow"),
            Some("write an email.".to_string())
        );
        assert_eq!(
            strip_activation_phrase("hey flow write an email", "Hey Flow"),
            Some("write an email".to_string())
        );
        assert_eq!(
            strip_activation_phrase("Hey, Flow: draft a reply", "Hey Flow"),
            Some("draft a reply".to_string())
        );
    }

    #[test]
    fn phrase_only_yields_empty_command() {
        assert_eq!(
            strip_activation_phrase("Hey Flow.", "Hey Flow"),
            Some(String::new())
        );
        assert_eq!(
            strip_activation_phrase("  hey flow  ", "Hey Flow"),
            Some(String::new())
        );
    }

    #[test]
    fn non_matching_transcripts_pass_through() {
        assert_eq!(strip_activation_phrase("Hello there", "Hey Flow"), None);
        assert_eq!(
            strip_activation_phrase("Hey, please flow with it", "Hey Flow"),
            None
        );
        assert_eq!(
            strip_activation_phrase("So hey flow do this", "Hey Flow"),
            None
        );
        assert_eq!(strip_activation_phrase("", "Hey Flow"), None);
    }

    #[test]
    fn phrase_mid_sentence_does_not_trigger() {
        assert_eq!(
            strip_activation_phrase("I said hey flow yesterday", "Hey Flow"),
            None
        );
    }

    #[test]
    fn custom_phrase_is_respected() {
        assert_eq!(
            strip_activation_phrase("Okay computer, write a poem", "Okay Computer"),
            Some("write a poem".to_string())
        );
        assert_eq!(
            strip_activation_phrase("Hey Flow write a poem", "Okay Computer"),
            None
        );
    }

    #[test]
    fn command_keeps_original_formatting() {
        assert_eq!(
            strip_activation_phrase("Hey Flow, write: Dear Sam,\nSee you Monday.", "Hey Flow"),
            Some("write: Dear Sam,\nSee you Monday.".to_string())
        );
    }

    #[test]
    fn empty_phrase_never_matches() {
        assert_eq!(strip_activation_phrase("anything", ""), None);
        assert_eq!(strip_activation_phrase("anything", " , "), None);
    }
}
