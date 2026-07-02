use crate::settings::PostProcessProvider;
use futures_util::StreamExt;
use log::debug;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct JsonSchema {
    name: String,
    strict: bool,
    schema: Value,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
    json_schema: JsonSchema,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    /// Message objects (`{role, content}`); content may be a plain string or
    /// an array of multimodal content parts (text / image_url).
    messages: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Debug, Deserialize)]
struct ChatMessageResponse {
    content: Option<String>,
}

/// Build headers for API requests based on provider type
fn build_headers(provider: &PostProcessProvider, api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();

    // Common headers
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    // OpenRouter reads `HTTP-Referer` (not the standard `Referer`) plus `X-Title`
    // for app attribution / leaderboard ranking; other gateways ignore both.
    // We send `HTTP-Referer` (the name OpenRouter documents) and keep the plain
    // `Referer` too — harmless everywhere, and some proxies still look at it.
    headers.insert(
        "HTTP-Referer",
        HeaderValue::from_static("https://github.com/AbhishekBarali/SpeakoFlow"),
    );
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://github.com/AbhishekBarali/SpeakoFlow"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("SpeakoFlow/1.0 (+https://github.com/AbhishekBarali/SpeakoFlow)"),
    );
    headers.insert("X-Title", HeaderValue::from_static("SpeakoFlow"));

    // Provider-specific auth headers
    if !api_key.is_empty() {
        if provider.id == "anthropic" {
            // Anthropic's OpenAI-compatible layer (api.anthropic.com/v1) accepts
            // either the native `x-api-key` header or `Authorization: Bearer`.
            // We send the native pair, which also works on the classic API.
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(api_key)
                    .map_err(|e| format!("Invalid API key header value: {}", e))?,
            );
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        } else {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .map_err(|e| format!("Invalid authorization header value: {}", e))?,
            );
            // Azure OpenAI's v1 endpoint (`{res}.openai.azure.com/openai/v1`)
            // accepts key auth via the `api-key` header as well as `Bearer`.
            // Send both for Azure hosts so a key works regardless of which auth
            // style the gateway honors; non-Azure hosts ignore the extra header.
            if provider.base_url.contains("azure.com") {
                headers.insert(
                    "api-key",
                    HeaderValue::from_str(api_key)
                        .map_err(|e| format!("Invalid api-key header value: {}", e))?,
                );
            }
        }
    }

    Ok(headers)
}

/// Create an HTTP client with provider-specific headers
fn create_client(provider: &PostProcessProvider, api_key: &str) -> Result<reqwest::Client, String> {
    let headers = build_headers(provider, api_key)?;
    reqwest::Client::builder()
        .default_headers(headers)
        // HTTP/1.1 avoids h2 flow-control issues seen with some gateways
        // (Azure) on large bodies, e.g. multi-hundred-KB image payloads
        // arriving truncated ("Unterminated string" 400s).
        .http1_only()
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// If `raw` points at an Azure OpenAI–style host, return just the host. Covers
/// the Azure OpenAI resource domain, the AI Foundry domain, the classic
/// Cognitive Services domain, and the US-gov domain.
fn azure_openai_host(raw: &str) -> Option<String> {
    let without_scheme = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
        .unwrap_or(raw);
    let host = without_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase();
    if host.ends_with(".openai.azure.com")
        || host.ends_with(".services.ai.azure.com")
        || host.ends_with(".cognitiveservices.azure.com")
        || host.ends_with(".openai.azure.us")
    {
        Some(host)
    } else {
        None
    }
}

/// Resolve the effective OpenAI-compatible base URL for a provider.
///
/// Most providers are used verbatim (trailing slash trimmed). Azure endpoints
/// are normalized to the v1 API surface: whatever the user pastes from the
/// portal — `https://{res}.openai.azure.com/`, the AI Foundry project endpoint
/// `https://{res}.services.ai.azure.com/api/projects/{proj}`, or a
/// `cognitiveservices.azure.com` domain — is rewritten to
/// `https://{host}/openai/v1`, which is where the app appends
/// `/chat/completions` and `/models`. This makes "paste the endpoint from the
/// Azure portal" just work instead of 404ing on a missing `/openai/v1` path.
fn effective_base_url(provider: &PostProcessProvider) -> String {
    let raw = provider.base_url.trim().trim_end_matches('/');
    if let Some(host) = azure_openai_host(raw) {
        return format!("https://{}/openai/v1", host);
    }
    raw.to_string()
}

/// Send a chat completion request to an OpenAI-compatible API
/// Returns Ok(Some(content)) on success, Ok(None) if response has no content,
/// or Err on actual errors (HTTP, parsing, etc.)
pub async fn send_chat_completion(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    prompt: String,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<Option<String>, String> {
    send_chat_completion_with_schema(
        provider,
        api_key,
        model,
        prompt,
        None,
        None,
        reasoning_effort,
        reasoning,
    )
    .await
}

/// Send a chat completion request with structured output support
/// When json_schema is provided, uses structured outputs mode
/// system_prompt is used as the system message when provided
/// reasoning_effort sets the OpenAI-style top-level field (e.g., "none", "low", "medium", "high")
/// reasoning sets the OpenRouter-style nested object (effort + exclude)
pub async fn send_chat_completion_with_schema(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<Option<String>, String> {
    let base_url = effective_base_url(provider);
    let url = format!("{}/chat/completions", base_url);

    debug!("Sending chat completion request to: {}", url);

    let client = create_client(provider, &api_key)?;

    // Build messages vector
    let mut messages = Vec::new();

    // Add system prompt if provided
    if let Some(system) = system_prompt {
        messages.push(serde_json::json!({"role": "system", "content": system}));
    }

    // Add user message
    messages.push(serde_json::json!({"role": "user", "content": user_content}));

    // Build response_format if schema is provided
    let response_format = json_schema.map(|schema| ResponseFormat {
        format_type: "json_schema".to_string(),
        json_schema: JsonSchema {
            name: "transcription_output".to_string(),
            strict: true,
            schema,
        },
    });

    let request_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        response_format,
        reasoning_effort,
        reasoning,
        stream: None,
    };

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read error response".to_string());
        return Err(format!(
            "API request failed with status {}: {}",
            status, error_text
        ));
    }

    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse API response: {}", e))?;

    Ok(completion
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone()))
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
}

/// Send a streaming chat completion request to an OpenAI-compatible API.
/// Parses the SSE response (`data: {...}` lines, `data: [DONE]` sentinel) and
/// invokes `on_token` for every content delta. Returns the full accumulated
/// response text on success.
pub async fn send_chat_stream(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    messages: Vec<Value>,
    mut on_token: impl FnMut(&str),
) -> Result<String, String> {
    let base_url = effective_base_url(provider);
    let url = format!("{}/chat/completions", base_url);

    debug!("Sending streaming chat completion request to: {}", url);

    let client = create_client(provider, &api_key)?;

    let request_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        response_format: None,
        reasoning_effort: None,
        reasoning: None,
        stream: Some(true),
    };

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read error response".to_string());
        return Err(format!(
            "API request failed with status {}: {}",
            status, error_text
        ));
    }

    let mut stream = response.bytes_stream();
    let mut line_buf = String::new();
    let mut full_text = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Stream read failed: {}", e))?;
        line_buf.push_str(&String::from_utf8_lossy(&chunk));

        // Process complete lines; keep any partial trailing line in the buffer.
        while let Some(newline_pos) = line_buf.find('\n') {
            let line: String = line_buf.drain(..=newline_pos).collect();
            let line = line.trim();

            let Some(data) = line.strip_prefix("data:") else {
                continue; // ignore comments, event: lines, blank keep-alives
            };
            let data = data.trim();

            if data == "[DONE]" {
                return Ok(full_text);
            }

            match serde_json::from_str::<StreamChunk>(data) {
                Ok(parsed) => {
                    if let Some(token) = parsed
                        .choices
                        .first()
                        .and_then(|c| c.delta.content.as_deref())
                    {
                        if !token.is_empty() {
                            full_text.push_str(token);
                            on_token(token);
                        }
                    }
                }
                Err(e) => {
                    debug!("Skipping unparsable SSE chunk: {} ({})", data, e);
                }
            }
        }
    }

    // Stream ended without [DONE]; return what we have.
    Ok(full_text)
}

/// Fetch available models from an OpenAI-compatible API
/// Returns a list of model IDs
pub async fn fetch_models(
    provider: &PostProcessProvider,
    api_key: String,
) -> Result<Vec<String>, String> {
    let base_url = effective_base_url(provider);
    let url = format!("{}/models", base_url);

    debug!("Fetching models from: {}", url);

    let client = create_client(provider, &api_key)?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch models: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!(
            "Model list request failed ({}): {}",
            status, error_text
        ));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let mut models = Vec::new();

    // Handle OpenAI format: { data: [ { id: "..." }, ... ] }
    if let Some(data) = parsed.get("data").and_then(|d| d.as_array()) {
        for entry in data {
            if let Some(id) = entry.get("id").and_then(|i| i.as_str()) {
                models.push(id.to_string());
            } else if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
                models.push(name.to_string());
            }
        }
    }
    // Handle array format: [ "model1", "model2", ... ]
    else if let Some(array) = parsed.as_array() {
        for entry in array {
            if let Some(model) = entry.as_str() {
                models.push(model.to_string());
            }
        }
    }

    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::PostProcessProvider;

    fn provider(id: &str, base: &str) -> PostProcessProvider {
        PostProcessProvider {
            id: id.to_string(),
            label: id.to_string(),
            base_url: base.to_string(),
            allow_base_url_edit: true,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
        }
    }

    #[test]
    fn azure_openai_endpoints_normalize_to_v1() {
        // Bare resource endpoint from the Azure portal.
        assert_eq!(
            effective_base_url(&provider("azure_openai", "https://res.openai.azure.com/")),
            "https://res.openai.azure.com/openai/v1"
        );
        // AI Foundry project endpoint (path is stripped).
        assert_eq!(
            effective_base_url(&provider(
                "azure_openai",
                "https://res.services.ai.azure.com/api/projects/proj"
            )),
            "https://res.services.ai.azure.com/openai/v1"
        );
        // Classic Cognitive Services custom domain.
        assert_eq!(
            effective_base_url(&provider(
                "azure_openai",
                "https://res.cognitiveservices.azure.com"
            )),
            "https://res.cognitiveservices.azure.com/openai/v1"
        );
        // Already-correct v1 endpoint is left equivalent.
        assert_eq!(
            effective_base_url(&provider(
                "azure_openai",
                "https://res.openai.azure.com/openai/v1"
            )),
            "https://res.openai.azure.com/openai/v1"
        );
    }

    #[test]
    fn non_azure_base_urls_are_untouched() {
        assert_eq!(
            effective_base_url(&provider("openai", "https://api.openai.com/v1/")),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            effective_base_url(&provider("groq", "https://api.groq.com/openai/v1")),
            "https://api.groq.com/openai/v1"
        );
        assert_eq!(
            effective_base_url(&provider("local", "http://localhost:11434/v1")),
            "http://localhost:11434/v1"
        );
    }

    #[test]
    fn azure_host_detection() {
        assert!(azure_openai_host("https://res.openai.azure.com/openai/v1").is_some());
        assert!(azure_openai_host("https://res.services.ai.azure.com/api/projects/p").is_some());
        assert!(azure_openai_host("https://api.openai.com/v1").is_none());
        assert!(azure_openai_host("http://localhost:11434/v1").is_none());
    }
}
