use crate::settings::PostProcessProvider;
use futures_util::StreamExt;
use log::debug;
use once_cell::sync::Lazy;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Mutex;

#[derive(Debug, Serialize, Deserialize, Clone, Type)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Small JPEG **display thumbnails** (data URLs) for any images that rode
    /// along with this message — a screen capture and/or user-attached pictures.
    /// Display + history only: these are never sent to the model (the full-res
    /// copy is sent once, for that turn), they just let the panel show and
    /// hover-enlarge what was sent, and survive restarts. Older history rows
    /// (and text-only turns) simply have an empty list.
    #[serde(default)]
    pub images: Vec<String>,
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
    /// llama.cpp-specific template options. Sent only to the built-in engine.
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    /// OpenAI-style tool definitions (`[{type:"function", function:{…}}]`).
    /// Only sent on the tool-calling web-search path.
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Value>,
    /// Tool choice policy ("auto" for the web-search path). Sent only with `tools`.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Value>,
}

#[derive(Default)]
struct ChatRequestOptions {
    json_schema: Option<Value>,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
    stream: Option<bool>,
    tools: Option<Value>,
    tool_choice: Option<Value>,
}

/// Build an OpenAI-compatible request body without performing any I/O.
///
/// Keeping provider-specific message normalization and optional request fields
/// here gives transport tests a stable seam for plain, structured-output, and
/// tool-enabled rounds.
fn build_chat_completion_request(
    provider: &PostProcessProvider,
    model: &str,
    mut messages: Vec<Value>,
    options: ChatRequestOptions,
) -> ChatCompletionRequest {
    if provider.id == "builtin" {
        fold_system_into_first_user(&mut messages);
    }

    let response_format = options.json_schema.map(|schema| ResponseFormat {
        format_type: "json_schema".to_string(),
        json_schema: JsonSchema {
            name: "transcription_output".to_string(),
            strict: true,
            schema,
        },
    });

    ChatCompletionRequest {
        model: normalize_model_name(provider, model),
        messages,
        response_format,
        reasoning_effort: options.reasoning_effort,
        reasoning: options.reasoning,
        chat_template_kwargs: builtin_chat_template_kwargs(provider),
        stream: options.stream,
        tools: options.tools,
        tool_choice: options.tool_choice,
    }
}

fn builtin_chat_template_kwargs(provider: &PostProcessProvider) -> Option<Value> {
    (provider.id == "builtin").then(|| serde_json::json!({ "enable_thinking": false }))
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

/// Configured clients, keyed by the provider/auth fingerprint that decides the
/// default headers.
///
/// A `reqwest::Client` owns its connection pool, so building a fresh one per
/// request throws that pool away and re-pays DNS + TCP + TLS on every call —
/// easily 100–300ms against a cloud provider, and more on a slow link. Cloning
/// a cached client is cheap and shares the pool, so a warm keep-alive
/// connection is reused across turns. Bounded to a handful of entries because
/// the key only varies with provider/base URL/key, not per request.
static CLIENT_CACHE: Lazy<Mutex<HashMap<u64, reqwest::Client>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Fingerprint the inputs that affect a client's default headers. The API key
/// is hashed rather than stored so the long-lived cache key holds no secret.
fn client_cache_key(provider: &PostProcessProvider, api_key: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    provider.id.hash(&mut hasher);
    provider.base_url.hash(&mut hasher);
    api_key.hash(&mut hasher);
    hasher.finish()
}

/// Get an HTTP client with provider-specific headers, reusing a cached one (and
/// therefore its live connections) whenever the provider and key are unchanged.
fn create_client(provider: &PostProcessProvider, api_key: &str) -> Result<reqwest::Client, String> {
    let key = client_cache_key(provider, api_key);
    if let Ok(cache) = CLIENT_CACHE.lock() {
        if let Some(client) = cache.get(&key) {
            return Ok(client.clone());
        }
    }

    let client = build_client(provider, api_key)?;
    if let Ok(mut cache) = CLIENT_CACHE.lock() {
        // Guard against unbounded growth if a caller ever varies the key wildly.
        if cache.len() >= 8 {
            cache.clear();
        }
        cache.insert(key, client.clone());
    }
    Ok(client)
}

/// A client that shares no connection pool with the cached one.
///
/// Used for the single retry after a gateway-shaped rejection: the point of the
/// retry is to not reuse whatever socket produced it.
fn uncached_client(
    provider: &PostProcessProvider,
    api_key: &str,
) -> Result<reqwest::Client, String> {
    build_client(provider, api_key)
}

fn build_client(provider: &PostProcessProvider, api_key: &str) -> Result<reqwest::Client, String> {
    let headers = build_headers(provider, api_key)?;
    reqwest::Client::builder()
        .default_headers(headers)
        // HTTP/1.1 avoids h2 flow-control issues seen with some gateways
        // (Azure) on large bodies, e.g. multi-hundred-KB image payloads
        // arriving truncated ("Unterminated string" 400s).
        .http1_only()
        // Bound connection setup so an unreachable/hung host fails fast instead
        // of leaving a turn pending forever (which would pin the built-in
        // engine's in-flight guard and block idle-unload). Intentionally NOT an
        // overall `.timeout()`: that would abort long, legitimate streaming
        // generations. Per-chunk stall detection lives in `read_sse_round`.
        .connect_timeout(std::time::Duration::from_secs(30))
        // Keep idle sockets around between turns so a follow-up question reuses
        // the handshake instead of repeating it.
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// Why a chat request did not come back with a usable response.
pub(crate) enum ChatRequestFailure {
    /// The client or request could not be constructed — a bad header value, say.
    Build(String),
    Transport(String),
    Status {
        status: u16,
        detail: String,
    },
}

impl ChatRequestFailure {
    /// The wording the rest of the app shows or speaks.
    fn message(&self) -> String {
        match self {
            Self::Build(detail) => format!("Request setup failed: {detail}"),
            Self::Transport(detail) => format!("HTTP request failed: {detail}"),
            Self::Status { status, detail } => {
                format!("API request failed with status {status}: {detail}")
            }
        }
    }
}

/// Whether an error body came from an HTTP front end rather than the model API.
///
/// Every provider reports its own errors as JSON. An HTML error page means the
/// request was rejected before anything looked at it — observed live on an Azure
/// AI Foundry endpoint, which answered a vision turn with IIS/http.sys's
/// "Bad Request - Invalid Verb" page while the identical URL accepted 400 KB
/// bodies from curl moments later.
///
/// Two consequences. The HTML must never reach the panel or the text-to-speech
/// engine, which would otherwise read out a DOCTYPE. And it earns exactly one
/// retry on a connection that has nothing to do with the one that failed, since
/// a socket a gateway has already half-closed produces this shape.
fn is_gateway_html(body: &str) -> bool {
    let head = body.trim_start().to_lowercase();
    head.starts_with("<!doctype html")
        || head.starts_with("<html")
        || head.contains("microsoft-httpapi")
        || head.contains("the request verb is invalid")
}

/// A sentence for a gateway rejection, in place of a page of markup.
fn gateway_message(status: u16) -> String {
    format!(
        "The model provider's gateway rejected the request before the model saw it (HTTP {status}). \
This is not an answer the model refused — it never got there. If it keeps happening on turns with \
a screenshot, lower the screen-capture quality or try another provider."
    )
}

/// Send one chat request, retrying once on a fresh connection when the response
/// looks like it came from a front end rather than the API.
///
/// The body is serialised once and reused, both so the retry is byte-identical
/// and so the size can be logged: when a request fails for reasons the provider
/// will not explain, its shape is the only evidence there is, and reconstructing
/// it after the fact is impossible.
async fn send_chat_request(
    provider: &PostProcessProvider,
    api_key: &str,
    url: &str,
    request_body: &ChatCompletionRequest,
) -> Result<reqwest::Response, ChatRequestFailure> {
    let bytes = serde_json::to_vec(request_body)
        .map_err(|e| ChatRequestFailure::Build(format!("could not encode the request: {e}")))?;
    let images = count_image_parts(&request_body.messages);
    debug!(
        "Chat request to {}: {} message(s), {} image part(s), {} KB",
        url,
        request_body.messages.len(),
        images,
        bytes.len() / 1024
    );

    let client = create_client(provider, api_key).map_err(ChatRequestFailure::Build)?;
    let failure = match client.post(url).body(bytes.clone()).send().await {
        Ok(response) if response.status().is_success() => return Ok(response),
        Ok(response) => {
            let status = response.status().as_u16();
            let detail = response
                .text()
                .await
                .unwrap_or_else(|_| "Failed to read error response".to_string());
            ChatRequestFailure::Status { status, detail }
        }
        Err(error) => ChatRequestFailure::Transport(error.to_string()),
    };

    // Only a gateway-shaped rejection is retried. A real API error (a bad model
    // name, a rate limit, an oversized image the provider explains) would fail
    // identically and the second attempt would only cost the user time.
    let ChatRequestFailure::Status { status, detail } = &failure else {
        return Err(failure);
    };
    if !is_gateway_html(detail) {
        return Err(failure);
    }
    log::warn!(
        "Provider gateway rejected the request with HTTP {} and an HTML page ({} message(s), {} image part(s), {} KB). Retrying once on a new connection.",
        status,
        request_body.messages.len(),
        images,
        bytes.len() / 1024
    );

    let fresh = uncached_client(provider, api_key).map_err(ChatRequestFailure::Build)?;
    match fresh.post(url).body(bytes).send().await {
        Ok(response) if response.status().is_success() => {
            debug!("The retry on a fresh connection succeeded.");
            Ok(response)
        }
        Ok(response) => {
            let status = response.status().as_u16();
            let detail = response.text().await.unwrap_or_default();
            if is_gateway_html(&detail) {
                // Twice, on two connections: the request itself is what the
                // gateway objects to. Say that, rather than quoting its markup.
                return Err(ChatRequestFailure::Status {
                    status,
                    detail: gateway_message(status),
                });
            }
            Err(ChatRequestFailure::Status { status, detail })
        }
        Err(error) => Err(ChatRequestFailure::Transport(error.to_string())),
    }
}

/// How many image parts a message list carries, for diagnostics.
fn count_image_parts(messages: &[Value]) -> usize {
    messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("image_url"))
        .count()
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

/// True when a provider targets Google Gemini's OpenAI-compatible surface.
fn is_gemini_provider(provider: &PostProcessProvider) -> bool {
    provider
        .base_url
        .contains("generativelanguage.googleapis.com")
}

/// Normalize a model id for provider-specific quirks before it is sent.
///
/// Google Gemini's OpenAI-compatible `/models` endpoint returns ids with a
/// `models/` prefix (e.g. `models/gemini-2.5-flash`), mirroring the native REST
/// resource names — but its `/chat/completions` endpoint expects the BARE id
/// (`gemini-2.5-flash`), exactly like every example in Google's OpenAI-compat
/// docs. A user who picks a model from the "Load models" list therefore ends up
/// with a `models/…` value the chat endpoint rejects (surfacing as "couldn't
/// get an answer"), while OpenRouter and the other providers — which return
/// bare ids — just work. Strip the prefix for Gemini so it behaves like the
/// rest. No-op for every other provider and for values that are already bare.
///
/// Applies to both the Assistant and the dictation AI-cleanup path, since both
/// share this client and the provider table.
fn normalize_model_name(provider: &PostProcessProvider, model: &str) -> String {
    if is_gemini_provider(provider) {
        if let Some(bare) = model.strip_prefix("models/") {
            return bare.to_string();
        }
    }
    model.to_string()
}

/// Extract the plain-text content of a chat message whose `content` may be a
/// string or an array of OpenAI-style content parts (text / image_url). Only
/// text parts contribute; images are ignored.
fn message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Fold a leading `system` message into the first `user` message.
///
/// The bundled llama.cpp engine (the "Built-in (Local)" provider) runs with
/// `--jinja`, so it applies each model's own chat template. Some templates —
/// notably Gemma's — reject a `system` role outright and require messages to
/// start with `user` and strictly alternate, returning a 400 "Conversation
/// roles must alternate user/assistant". To keep the zero-setup built-in
/// provider working across every local model, we merge the system prompt into
/// the first user turn instead of sending it as its own message. External
/// providers (Ollama / LM Studio / cloud) manage their own templating and are
/// never passed through this.
fn fold_system_into_first_user(messages: &mut Vec<Value>) {
    // Only act when the first message is a system message.
    let leads_with_system = messages
        .first()
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        == Some("system");
    if !leads_with_system {
        return;
    }

    let system_msg = messages.remove(0);
    let system_text = message_text(&system_msg);
    if system_text.trim().is_empty() {
        return;
    }

    // Graft the system text onto the first user message.
    if let Some(user_msg) = messages
        .iter_mut()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
    {
        match user_msg.get_mut("content") {
            Some(Value::String(s)) => {
                *s = format!("{}\n\n{}", system_text, s);
            }
            // Multimodal content (text + image parts): prepend a text part so
            // the system prompt still leads the turn without disturbing images.
            Some(Value::Array(parts)) => {
                parts.insert(
                    0,
                    serde_json::json!({ "type": "text", "text": format!("{}\n\n", system_text) }),
                );
            }
            _ => {
                user_msg["content"] = Value::String(system_text);
            }
        }
    } else {
        // No user message to attach to — re-add the content as a user turn so
        // the request isn't empty and still satisfies the alternation rule.
        messages.insert(
            0,
            serde_json::json!({ "role": "user", "content": system_text }),
        );
    }
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

#[derive(Debug)]
pub(crate) enum ChatCompletionError {
    RequestBuild(String),
    Transport(String),
    HttpStatus { status: u16, detail: String },
    ResponseDecode(String),
}

impl std::fmt::Display for ChatCompletionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RequestBuild(detail) => write!(f, "Request setup failed: {detail}"),
            Self::Transport(detail) => write!(f, "HTTP request failed: {detail}"),
            Self::HttpStatus { status, detail } => {
                write!(f, "API request failed with status {status}: {detail}")
            }
            Self::ResponseDecode(detail) => write!(f, "Failed to parse API response: {detail}"),
        }
    }
}

pub(crate) async fn send_chat_completion_with_schema_typed(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<Option<String>, ChatCompletionError> {
    let base_url = effective_base_url(provider);
    let url = format!("{}/chat/completions", base_url);

    debug!("Sending chat completion request to: {}", url);

    let mut messages = Vec::new();
    if let Some(system) = system_prompt {
        messages.push(serde_json::json!({"role": "system", "content": system}));
    }
    messages.push(serde_json::json!({"role": "user", "content": user_content}));

    let request_body = build_chat_completion_request(
        provider,
        model,
        messages,
        ChatRequestOptions {
            json_schema,
            reasoning_effort,
            reasoning,
            ..Default::default()
        },
    );

    let response = send_chat_request(provider, &api_key, &url, &request_body)
        .await
        .map_err(|failure| match failure {
            ChatRequestFailure::Build(detail) => ChatCompletionError::RequestBuild(detail),
            ChatRequestFailure::Transport(detail) => ChatCompletionError::Transport(detail),
            ChatRequestFailure::Status { status, detail } => {
                ChatCompletionError::HttpStatus { status, detail }
            }
        })?;

    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|error| ChatCompletionError::ResponseDecode(error.to_string()))?;

    Ok(completion
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone()))
}

/// Send a chat completion request with structured output support.
/// Existing assistant/memory callers retain the historical string error API;
/// cleanup uses the typed inner function above for safe classification.
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
    send_chat_completion_with_schema_typed(
        provider,
        api_key,
        model,
        user_content,
        system_prompt,
        json_schema,
        reasoning_effort,
        reasoning,
    )
    .await
    .map_err(|error| error.to_string())
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
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCall>>,
}

/// A streamed tool-call delta. `id`/`name` arrive on the first chunk for an
/// index; `arguments` streams in fragments that must be concatenated per index.
#[derive(Debug, Deserialize)]
struct StreamToolCall {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<StreamToolCallFn>,
    /// Gemini-only side channel (see `ToolCall::thought_signature`). Absent on
    /// every other OpenAI-compatible provider.
    #[serde(default)]
    extra_content: Option<StreamToolCallExtra>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCallFn {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// `tool_calls[].extra_content` as Gemini's OpenAI-compatibility layer emits it:
/// `{"google": {"thought_signature": "<opaque>"}}`.
#[derive(Debug, Deserialize)]
struct StreamToolCallExtra {
    #[serde(default)]
    google: Option<StreamToolCallGoogleExtra>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCallGoogleExtra {
    #[serde(default)]
    thought_signature: Option<String>,
}

/// A fully-assembled tool call the model asked us to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON arguments string (parse at the call site).
    pub arguments: String,
    /// Gemini thinking models return an opaque, encrypted `thought_signature`
    /// alongside every function call and require it echoed back verbatim on the
    /// follow-up request, or the next call fails with
    /// "Function call is missing a thought_signature in functionCall parts"
    /// (HTTP 400). It is opaque to us: capture it, carry it, hand it back
    /// untouched (see the `extra_content` re-attach in `assistant.rs`).
    /// `None` for every provider that does not send one, which is all of them
    /// except Gemini — so nothing extra is ever put on the wire elsewhere.
    pub thought_signature: Option<String>,
}

/// Outcome of one streamed tool-calling round: any content the model streamed
/// (via `on_token`) is accumulated in `text`; if the model instead asked to run
/// tools, they're in `tool_calls` (and `text` is usually empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolStreamOutcome {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

/// Transport-neutral result of one assistant chat round. This crate-visible
/// shape can be scripted directly by later agent-loop tests without mocking an
/// HTTP client or adding a test dependency.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ChatRound {
    pub(crate) text: String,
    pub(crate) tool_calls: Vec<ToolCall>,
}

impl From<ChatRound> for ToolStreamOutcome {
    fn from(round: ChatRound) -> Self {
        Self {
            text: round.text,
            tool_calls: round.tool_calls,
        }
    }
}

/// Per-index scratch space while a tool call streams in. Named fields (rather
/// than a positional tuple) keep the merge below readable now that a fourth,
/// provider-specific value rides along.
#[derive(Default)]
struct ToolCallParts {
    id: String,
    name: String,
    arguments: String,
    thought_signature: Option<String>,
}

/// Byte-buffered decoder and accumulator for an OpenAI-compatible SSE chat
/// round. Complete lines are decoded only after all their bytes arrive, so a
/// UTF-8 code point may be split across any number of network chunks safely.
#[derive(Default)]
struct SseChatAccumulator {
    pending: Vec<u8>,
    round: ChatRound,
    tool_call_parts: Vec<ToolCallParts>,
    done: bool,
}

impl SseChatAccumulator {
    fn push(&mut self, chunk: &[u8], on_token: &mut dyn FnMut(&str)) {
        if self.done {
            return;
        }

        self.pending.extend_from_slice(chunk);
        while !self.done {
            let Some(newline_pos) = self.pending.iter().position(|byte| *byte == b'\n') else {
                break;
            };
            let line: Vec<u8> = self.pending.drain(..=newline_pos).collect();
            self.process_line(&line[..line.len() - 1], on_token);
        }

        if self.done {
            self.pending.clear();
        }
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn finish(mut self, _on_token: &mut dyn FnMut(&str)) -> ChatRound {
        // Preserve the wrappers' historical EOF behavior: return everything
        // decoded from complete lines, but ignore an unterminated trailing line.
        self.round.tool_calls = assemble_tool_calls(self.tool_call_parts);
        self.round
    }

    fn process_line(&mut self, line: &[u8], on_token: &mut dyn FnMut(&str)) {
        let line = trim_ascii_whitespace(line);
        let Some(data) = line.strip_prefix(b"data:") else {
            return; // ignore comments, event: lines, and blank keep-alives
        };
        let data = trim_ascii_whitespace(data);

        if data == b"[DONE]" {
            self.done = true;
            return;
        }

        let data = match std::str::from_utf8(data) {
            Ok(data) => data,
            Err(error) => {
                debug!(
                    "Skipping unparsable SSE chunk: {} ({})",
                    String::from_utf8_lossy(data),
                    error
                );
                return;
            }
        };

        match serde_json::from_str::<StreamChunk>(data) {
            Ok(parsed) => self.accumulate_chunk(parsed, on_token),
            Err(error) => {
                // Assistant wrappers historically skip malformed provider
                // frames and continue streaming; preserve that behavior.
                debug!("Skipping unparsable SSE chunk: {} ({})", data, error);
            }
        }
    }

    fn accumulate_chunk(&mut self, parsed: StreamChunk, on_token: &mut dyn FnMut(&str)) {
        let Some(choice) = parsed.choices.first() else {
            return;
        };

        if let Some(token) = choice.delta.content.as_deref() {
            if !token.is_empty() {
                self.round.text.push_str(token);
                on_token(token);
            }
        }

        if let Some(tool_calls) = &choice.delta.tool_calls {
            for tool_call in tool_calls {
                while self.tool_call_parts.len() <= tool_call.index {
                    self.tool_call_parts.push(ToolCallParts::default());
                }
                let slot = &mut self.tool_call_parts[tool_call.index];
                if let Some(id) = &tool_call.id {
                    if !id.is_empty() {
                        slot.id = id.clone();
                    }
                }
                if let Some(function) = &tool_call.function {
                    if let Some(name) = &function.name {
                        if !name.is_empty() {
                            slot.name = name.clone();
                        }
                    }
                    if let Some(arguments) = &function.arguments {
                        slot.arguments.push_str(arguments);
                    }
                }
                // Gemini may deliver the signature on any chunk for this index
                // (often a frame of its own, with no function payload), so merge
                // it by index like everything else and never clobber a captured
                // value with a later empty one.
                if let Some(signature) = tool_call
                    .extra_content
                    .as_ref()
                    .and_then(|extra| extra.google.as_ref())
                    .and_then(|google| google.thought_signature.as_deref())
                {
                    if !signature.is_empty() {
                        slot.thought_signature = Some(signature.to_string());
                    }
                }
            }
        }
    }
}

fn trim_ascii_whitespace(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

async fn read_sse_round(
    response: reqwest::Response,
    mut on_token: impl FnMut(&str),
) -> Result<ChatRound, String> {
    // Abort a stream that goes silent for too long. A stalled provider — or a
    // wedged local llama-server (e.g. KV-cache exhaustion) — would otherwise
    // keep this future pending indefinitely, which pins the assistant turn's
    // in-flight guard so the built-in engine never idle-unloads and its RAM is
    // never reclaimed. Generous enough not to trip on slow first tokens or long
    // reasoning pauses between chunks.
    const SSE_STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

    let mut stream = response.bytes_stream();
    let mut accumulator = SseChatAccumulator::default();

    loop {
        match tokio::time::timeout(SSE_STALL_TIMEOUT, stream.next()).await {
            Ok(Some(chunk)) => {
                let chunk = chunk.map_err(|error| format!("Stream read failed: {}", error))?;
                accumulator.push(&chunk, &mut on_token);
                if accumulator.is_done() {
                    break;
                }
            }
            Ok(None) => break, // stream ended normally
            Err(_) => {
                return Err(format!(
                    "LLM stream stalled: no data for {}s",
                    SSE_STALL_TIMEOUT.as_secs()
                ));
            }
        }
    }

    Ok(accumulator.finish(&mut on_token))
}

/// Send a streaming chat completion request to an OpenAI-compatible API.
/// Parses the SSE response (`data: {...}` lines, `data: [DONE]` sentinel) and
/// invokes `on_token` for every content delta. Returns the full accumulated
/// response text on success. `reasoning_effort` / `reasoning` suppress or tune
/// reasoning-model thinking (pass `None` for provider defaults).
pub async fn send_chat_stream(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    messages: Vec<Value>,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
    on_token: impl FnMut(&str),
) -> Result<String, String> {
    let base_url = effective_base_url(provider);
    let url = format!("{}/chat/completions", base_url);

    debug!("Sending streaming chat completion request to: {}", url);

    let request_body = build_chat_completion_request(
        provider,
        model,
        messages,
        ChatRequestOptions {
            stream: Some(true),
            reasoning_effort,
            reasoning,
            ..Default::default()
        },
    );

    let response = send_chat_request(provider, &api_key, &url, &request_body)
        .await
        .map_err(|failure| failure.message())?;

    Ok(read_sse_round(response, on_token).await?.text)
}

/// Streaming chat completion WITH tool support (the web-search tool-calling
/// path). Sends `tools` + `tool_choice: "auto"`; streams any assistant content
/// via `on_token`, and if the model instead requests tool calls, assembles and
/// returns them. The caller runs the tool(s), appends the results as messages,
/// and calls this again. Not used for the built-in/local engine (small models
/// handle tools poorly) — the caller gates on provider capability and falls
/// back to the planner path.
pub async fn send_chat_stream_with_tools(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    messages: Vec<Value>,
    tools: Value,
    tool_choice: Value,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
    on_token: impl FnMut(&str),
) -> Result<ToolStreamOutcome, String> {
    let base_url = effective_base_url(provider);
    let url = format!("{}/chat/completions", base_url);
    let request_body = build_chat_completion_request(
        provider,
        model,
        messages,
        ChatRequestOptions {
            stream: Some(true),
            tools: Some(tools),
            tool_choice: Some(tool_choice),
            reasoning_effort,
            reasoning,
            ..Default::default()
        },
    );

    let response = send_chat_request(provider, &api_key, &url, &request_body)
        .await
        .map_err(|failure| failure.message())?;

    Ok(read_sse_round(response, on_token).await?.into())
}

/// Turn accumulated tool-call fragments into ToolCalls, dropping any entry
/// without a function name (defensive against malformed streams).
fn assemble_tool_calls(acc: Vec<ToolCallParts>) -> Vec<ToolCall> {
    acc.into_iter()
        .enumerate()
        .filter(|(_, parts)| !parts.name.is_empty())
        .map(|(i, parts)| ToolCall {
            id: if parts.id.is_empty() {
                format!("call_{}", i)
            } else {
                parts.id
            },
            name: parts.name,
            arguments: parts.arguments,
            thought_signature: parts.thought_signature,
        })
        .collect()
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

    // Present bare model ids for Gemini (its `/models` lists `models/…` names
    // that its chat endpoint rejects), so both the picker and the value that
    // gets stored are the ones chat actually accepts. No-op elsewhere.
    let models = models
        .into_iter()
        .map(|model| normalize_model_name(provider, &model))
        .collect();

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
    fn builtin_requests_disable_optional_thinking() {
        let builtin = provider("builtin", "http://127.0.0.1:8080/v1");
        let kwargs = builtin_chat_template_kwargs(&builtin)
            .expect("built-in requests should carry chat template options");
        assert_eq!(kwargs["enable_thinking"], false);

        let cloud = provider("openai", "https://api.openai.com/v1");
        assert!(builtin_chat_template_kwargs(&cloud).is_none());
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
    fn gemini_model_name_strips_leading_models_prefix() {
        let gemini = provider(
            "gemini",
            "https://generativelanguage.googleapis.com/v1beta/openai",
        );
        // The `models/…` id returned by Gemini's /models list is reduced to the
        // bare id its chat endpoint expects.
        assert_eq!(
            normalize_model_name(&gemini, "models/gemini-2.5-flash"),
            "gemini-2.5-flash"
        );
        assert_eq!(
            normalize_model_name(&gemini, "models/gemini-3.1-flash-lite"),
            "gemini-3.1-flash-lite"
        );
        // An already-bare id is untouched.
        assert_eq!(
            normalize_model_name(&gemini, "gemini-2.5-flash"),
            "gemini-2.5-flash"
        );
        // Only a single leading prefix is stripped; interior text is preserved.
        assert_eq!(
            normalize_model_name(&gemini, "models/tuned-models/my-model"),
            "tuned-models/my-model"
        );
    }

    #[test]
    fn non_gemini_model_names_are_untouched() {
        // OpenRouter uses `vendor/model` ids that must NOT be altered.
        let openrouter = provider("openrouter", "https://openrouter.ai/api/v1");
        assert_eq!(
            normalize_model_name(&openrouter, "google/gemini-2.5-flash"),
            "google/gemini-2.5-flash"
        );
        // A hypothetical `models/`-prefixed id on a non-Gemini host is left as-is.
        let openai = provider("openai", "https://api.openai.com/v1");
        assert_eq!(normalize_model_name(&openai, "models/foo"), "models/foo");
    }

    #[test]
    fn request_builder_normalizes_gemini_model() {
        let gemini = provider(
            "gemini",
            "https://generativelanguage.googleapis.com/v1beta/openai",
        );
        let request = build_chat_completion_request(
            &gemini,
            "models/gemini-2.5-flash",
            vec![serde_json::json!({"role": "user", "content": "hi"})],
            ChatRequestOptions::default(),
        );
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["model"], "gemini-2.5-flash");
    }

    #[test]
    fn azure_host_detection() {
        assert!(azure_openai_host("https://res.openai.azure.com/openai/v1").is_some());
        assert!(azure_openai_host("https://res.services.ai.azure.com/api/projects/p").is_some());
        assert!(azure_openai_host("https://api.openai.com/v1").is_none());
        assert!(azure_openai_host("http://localhost:11434/v1").is_none());
    }

    #[test]
    fn fold_system_merges_into_string_user_message() {
        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "You are helpful."}),
            serde_json::json!({"role": "user", "content": "Hi there"}),
        ];
        fold_system_into_first_user(&mut messages);

        // System turn is gone; conversation now starts with user.
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "You are helpful.\n\nHi there");
    }

    #[test]
    fn fold_system_preserves_history_alternation() {
        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "SYS"}),
            serde_json::json!({"role": "user", "content": "first"}),
            serde_json::json!({"role": "assistant", "content": "reply"}),
            serde_json::json!({"role": "user", "content": "second"}),
        ];
        fold_system_into_first_user(&mut messages);

        // Only the FIRST user message absorbs the system prompt; the rest keep
        // their strict user/assistant/user alternation (what Gemma requires).
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "SYS\n\nfirst");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"], "second");
    }

    #[test]
    fn fold_system_prepends_text_part_for_multimodal_user() {
        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "SYS"}),
            serde_json::json!({"role": "user", "content": [
                {"type": "text", "text": "what is this"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}}
            ]}),
        ];
        fold_system_into_first_user(&mut messages);

        assert_eq!(messages.len(), 1);
        let parts = messages[0]["content"].as_array().unwrap();
        // A leading text part carries the system prompt; the image is untouched.
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[0]["text"], "SYS\n\n");
        assert_eq!(parts[1]["type"], "text");
        assert_eq!(parts[2]["type"], "image_url");
    }

    #[test]
    fn fold_system_is_noop_without_leading_system() {
        let mut messages = vec![
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "assistant", "content": "hi"}),
        ];
        let before = messages.clone();
        fold_system_into_first_user(&mut messages);
        assert_eq!(messages, before);
    }

    fn decode_sse(chunks: impl IntoIterator<Item = Vec<u8>>) -> (ChatRound, Vec<String>) {
        let mut accumulator = SseChatAccumulator::default();
        let mut tokens = Vec::new();
        {
            let mut on_token = |token: &str| tokens.push(token.to_string());
            for chunk in chunks {
                accumulator.push(&chunk, &mut on_token);
            }
            let round = accumulator.finish(&mut on_token);
            return (round, tokens);
        }
    }

    #[test]
    fn request_builder_omits_optional_fields_for_plain_completion() {
        let request = build_chat_completion_request(
            &provider("openai", "https://api.openai.com/v1"),
            "gpt-test",
            vec![serde_json::json!({"role": "user", "content": "hello"})],
            ChatRequestOptions::default(),
        );
        let value = serde_json::to_value(request).unwrap();

        assert_eq!(value["model"], "gpt-test");
        assert_eq!(value["messages"][0]["content"], "hello");
        for omitted in [
            "response_format",
            "reasoning_effort",
            "reasoning",
            "chat_template_kwargs",
            "stream",
            "tools",
            "tool_choice",
        ] {
            assert!(value.get(omitted).is_none(), "{omitted} should be omitted");
        }
    }

    #[test]
    fn request_builder_emits_structured_output_fields() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
            "additionalProperties": false
        });
        let request = build_chat_completion_request(
            &provider("openai", "https://api.openai.com/v1"),
            "gpt-test",
            vec![serde_json::json!({"role": "user", "content": "hello"})],
            ChatRequestOptions {
                json_schema: Some(schema.clone()),
                reasoning_effort: Some("low".to_string()),
                reasoning: Some(ReasoningConfig {
                    effort: Some("minimal".to_string()),
                    exclude: Some(true),
                }),
                ..Default::default()
            },
        );
        let value = serde_json::to_value(request).unwrap();

        assert_eq!(value["response_format"]["type"], "json_schema");
        assert_eq!(
            value["response_format"]["json_schema"]["name"],
            "transcription_output"
        );
        assert_eq!(value["response_format"]["json_schema"]["strict"], true);
        assert_eq!(value["response_format"]["json_schema"]["schema"], schema);
        assert_eq!(value["reasoning_effort"], "low");
        assert_eq!(value["reasoning"]["effort"], "minimal");
        assert_eq!(value["reasoning"]["exclude"], true);
        assert!(value.get("tools").is_none());
    }

    #[test]
    fn request_builder_emits_tool_round_fields() {
        let tools = serde_json::json!([{
            "type": "function",
            "function": {
                "name": "web_search",
                "parameters": {"type": "object"}
            }
        }]);
        let request = build_chat_completion_request(
            &provider("openai", "https://api.openai.com/v1"),
            "gpt-test",
            vec![serde_json::json!({"role": "user", "content": "latest news"})],
            ChatRequestOptions {
                stream: Some(true),
                tools: Some(tools.clone()),
                tool_choice: Some(serde_json::json!("auto")),
                ..Default::default()
            },
        );
        let value = serde_json::to_value(request).unwrap();

        assert_eq!(value["stream"], true);
        assert_eq!(value["tools"], tools);
        assert_eq!(value["tool_choice"], "auto");
        assert!(value.get("response_format").is_none());
    }

    #[test]
    fn sse_decoder_preserves_utf8_split_at_every_byte_and_stops_at_done() {
        let first = serde_json::json!({
            "choices": [{"delta": {"content": "hé"}}]
        });
        let second = serde_json::json!({
            "choices": [{"delta": {"content": "llo 🌍"}}]
        });
        let ignored = serde_json::json!({
            "choices": [{"delta": {"content": " ignored"}}]
        });
        let wire =
            format!("data: {first}\r\n\r\ndata: {second}\n\ndata: [DONE]\n\ndata: {ignored}\n\n");
        let chunks = wire
            .as_bytes()
            .iter()
            .map(|byte| vec![*byte])
            .collect::<Vec<_>>();

        let (round, tokens) = decode_sse(chunks);
        assert_eq!(round.text, "héllo 🌍");
        assert_eq!(tokens, ["hé", "llo 🌍"]);
        assert!(round.tool_calls.is_empty());
    }

    #[test]
    fn sse_decoder_assembles_complete_tool_fragments_at_eof() {
        let first = serde_json::json!({
            "choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "id": "call_screen",
                "function": {"name": "capture_screen", "arguments": "{\"tim"}
            }]}}]
        });
        let second = serde_json::json!({
            "choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "function": {"arguments": "ing\":\"now\"}"}
            }]}}]
        });
        let unterminated = serde_json::json!({
            "choices": [{"delta": {"content": " ignored"}}]
        });
        // Omit [DONE]. Complete lines must survive EOF, while the final
        // unterminated line is discarded to match the historical wrappers.
        let wire = format!("data: {first}\n\ndata: {second}\ndata: {unterminated}");
        let chunks = wire
            .as_bytes()
            .chunks(7)
            .map(<[u8]>::to_vec)
            .collect::<Vec<_>>();

        let (round, tokens) = decode_sse(chunks);
        assert!(round.text.is_empty());
        assert!(tokens.is_empty());
        assert_eq!(
            round.tool_calls,
            [ToolCall {
                id: "call_screen".to_string(),
                name: "capture_screen".to_string(),
                arguments: "{\"timing\":\"now\"}".to_string(),
                thought_signature: None,
            }]
        );
    }

    /// Gemini thinking models return an opaque `thought_signature` with every
    /// function call and 400 the follow-up request if it is not echoed back. It
    /// can arrive on a later frame than the call itself, so the decoder has to
    /// merge it by index like the argument fragments.
    #[test]
    fn sse_decoder_captures_gemini_thought_signature_from_extra_content() {
        let call = serde_json::json!({
            "choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "id": "call_search",
                "function": {"name": "web_search", "arguments": "{\"query\":\"rust\"}"}
            }]}}]
        });
        let signature = serde_json::json!({
            "choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "extra_content": {"google": {"thought_signature": "SIG-ABC"}}
            }]}}]
        });
        let wire = format!("data: {call}\n\ndata: {signature}\n\ndata: [DONE]\n\n");

        let (round, _) = decode_sse(vec![wire.into_bytes()]);
        assert_eq!(
            round.tool_calls,
            [ToolCall {
                id: "call_search".to_string(),
                name: "web_search".to_string(),
                arguments: "{\"query\":\"rust\"}".to_string(),
                thought_signature: Some("SIG-ABC".to_string()),
            }]
        );
    }

    /// Providers that send no signature must stay at `None`, so nothing extra is
    /// ever attached to a non-Gemini follow-up request.
    #[test]
    fn sse_decoder_leaves_thought_signature_unset_without_extra_content() {
        let call = serde_json::json!({
            "choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "id": "call_time",
                "function": {"name": "get_current_datetime", "arguments": "{}"}
            }]}}]
        });
        let wire = format!("data: {call}\n\ndata: [DONE]\n\n");

        let (round, _) = decode_sse(vec![wire.into_bytes()]);
        assert_eq!(round.tool_calls.len(), 1);
        assert_eq!(round.tool_calls[0].thought_signature, None);
    }

    #[test]
    fn sse_decoder_skips_malformed_frames_and_keeps_following_content() {
        let valid = serde_json::json!({
            "choices": [{"delta": {"content": "still works"}}]
        });
        let mut wire = b"data: {not json}\n\ndata: \xff\n\n".to_vec();
        wire.extend_from_slice(format!("data: {valid}\n").as_bytes());

        let (round, tokens) = decode_sse([wire]);
        assert_eq!(round.text, "still works");
        assert_eq!(tokens, ["still works"]);
    }
}
