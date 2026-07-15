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
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::llm_client;
use crate::settings::{AppSettings, PostProcessProvider};

/// Upper bound for one whole Flow generation (engine start + all rounds).
/// Generous compared to AI cleanup because Flow writes whole artifacts, but
/// still bounded so a stalled provider can never hold the dictation pipeline.
const FLOW_TIMEOUT_SECS: u64 = 90;

/// Tool-round cap: one optional capture round plus the answer, with one spare
/// round so a confused model can still recover to a final answer.
const MAX_FLOW_TOOL_ROUNDS: usize = 3;

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

/// If `transcript` begins with the activation `phrase` (case- and
/// punctuation-insensitive, word by word), return the remaining command text
/// with the phrase and any separating punctuation removed. The command keeps
/// its original casing and formatting. `None` when the phrase doesn't lead.
pub fn strip_activation_phrase(transcript: &str, phrase: &str) -> Option<String> {
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
        if word != phrase_tokens[matched] {
            return None;
        }
        matched += 1;
    }

    let command = transcript[cursor..]
        .trim_start_matches(|c: char| {
            c.is_whitespace() || matches!(c, ',' | '.' | '!' | '?' | ':' | ';' | '-' | '—' | '–')
        })
        .trim_end();
    Some(command.to_string())
}

/// The stateless Flow system prompt. Everything the model outputs is pasted
/// verbatim, so the prompt forbids conversational framing entirely.
fn flow_system_prompt(screen_tool: bool) -> String {
    let mut s = String::from(
        "You are Flow, a silent writing engine inside a dictation app. The user spoke a command; \
         your ENTIRE response is pasted directly into whatever application they are using, exactly as-is. \
         Nobody reads it as a chat message.\n\
         \n\
         Rules:\n\
         - Output ONLY the requested content. No greetings, no preamble (never \"Sure\", \"Here is\", \"Certainly\"), \
         no explanations, no closing remarks, no offers to help further.\n\
         - NEVER ask questions or request clarification. If details are missing, make sensible neutral \
         assumptions and produce the best complete result anyway.\n\
         - Do not wrap the whole output in quotation marks or a code fence unless the content itself is code \
         that the user asked for in Markdown.\n\
         - Preserve real formatting: paragraphs, line breaks, lists, tables, Markdown, and code indentation. \
         For a code request, output raw code only — correct indentation, no surrounding fence, no commentary.\n\
         - For an email or letter, produce the complete ready-to-send text (a subject line only when one is \
         clearly useful), and never placeholders like [Your Name] unless the name is genuinely unknowable.\n\
         - Write in the same language the command was spoken in, unless the command says otherwise.\n",
    );
    if screen_tool {
        s.push_str(
            "\n## Tool\n\
             You may call capture_screen() ONCE to see the user's current screen. Call it ONLY when the \
             command clearly refers to something on screen (\"this email\", \"the text above\", \"look at\", \
             \"reply to this\", \"summarize this page\"). For self-contained commands, generate directly \
             without capturing. After looking, still output only the finished content.\n",
        );
    }
    s
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
/// error when nothing should be pasted. Bounded by `FLOW_TIMEOUT_SECS`.
pub async fn run_flow_generation(app: &AppHandle, command: &str) -> Result<String, String> {
    let settings = crate::settings::get_settings(app);
    let llm = resolve_flow_llm(&settings).ok_or_else(|| "Flow is not configured".to_string())?;
    let allow_screen = settings.flow_screen_access;
    debug!(
        "Flow generation: provider '{}', model '{}', screen access {}",
        llm.provider.id, llm.model, allow_screen
    );

    let generation = async {
        // Built-in engine: make sure it's serving the model, and hold an
        // activity guard so the idle watcher won't unload it mid-generation.
        let _llm_activity_guard = if llm.provider.id == "builtin" {
            let manager = app.state::<Arc<crate::managers::local_llm::LocalLlmManager>>();
            manager
                .ensure_running(&llm.model)
                .await
                .map_err(|e| format!("Local model failed to start: {}", e))?;
            Some(manager.begin_request())
        } else {
            None
        };

        let mut messages: Vec<Value> = vec![
            json!({"role": "system", "content": flow_system_prompt(allow_screen)}),
            json!({"role": "user", "content": command}),
        ];

        if !allow_screen {
            // Plain one-shot stream (tokens are ignored; only the final text
            // matters — nothing is pasted until the whole result exists).
            return llm_client::send_chat_stream(
                &llm.provider,
                llm.api_key.clone(),
                &llm.model,
                messages,
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
            let out = llm_client::send_chat_stream_with_tools(
                &llm.provider,
                llm.api_key.clone(),
                &llm.model,
                messages.clone(),
                tools.clone(),
                json!("auto"),
                |_| {},
            )
            .await?;
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

    let text = tokio::time::timeout(Duration::from_secs(FLOW_TIMEOUT_SECS), generation)
        .await
        .map_err(|_| "Flow generation timed out".to_string())??;

    let text = text.trim().to_string();
    if text.is_empty() {
        warn!("Flow generation returned no content");
        return Err("Flow returned no content".to_string());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

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
