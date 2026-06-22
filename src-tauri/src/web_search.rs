//! Web search for the assistant.
//!
//! Design priority: **answer quality**. An enabled web search should let the
//! assistant answer current/factual questions as well as a human with a browser
//! would — so we favor real page content and well-formed queries over raw speed.
//!
//! Two stages make that work:
//! 1. **Planner** (`plan_search`): a capable LLM decides *whether* a live search
//!    is actually needed and rewrites the user's request — often a rough voice
//!    transcription — into one to three clean, keyword-rich search queries
//!    (fixing misheard names, dropping filler, splitting compound questions, and
//!    picking a freshness window). This replaces the old keyword heuristic, which
//!    couldn't understand intent, other languages, or reformulate a messy query.
//! 2. **Retrieval** (`search_with_plan`): runs those queries and, with Firecrawl,
//!    pulls the **full page content** of the top results (not just a one-line
//!    snippet) so the model answers from the actual source text. Results are
//!    merged, de-duplicated, and bounded to a token budget.
//!
//! Providers: **Firecrawl** (`/v2/search` with `scrapeOptions` → full markdown,
//! the recommended high-quality path), **Brave**, and **DuckDuckGo** (keyless,
//! snippet-only fallback). Any failure or timeout degrades gracefully — the turn
//! answers without web context rather than breaking.

use crate::llm_client;
use crate::settings::{AppSettings, PostProcessProvider};
use log::{debug, warn};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use std::collections::HashSet;
use std::time::Duration;

/// Timeout for the lightweight snippet providers (Brave, DuckDuckGo). Generous
/// compared to the old 6 s ceiling — we prioritize getting an answer over speed.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Timeout for Firecrawl, which also scrapes result pages and so legitimately
/// takes longer than a bare search. Quality over speed: we wait.
const FIRECRAWL_TIMEOUT: Duration = Duration::from_secs(45);

/// Per-snippet character cap (the short description shown when full content is
/// unavailable). Larger than before so snippet-only providers still give the
/// model something to work with.
const SNIPPET_MAX_CHARS: usize = 400;

/// Per-result full-content cap. Firecrawl returns whole pages as markdown; we
/// keep a meaty excerpt per source while staying within the prompt budget.
const CONTENT_MAX_CHARS: usize = 2_000;

/// Total cap on the web-context block fed to the model, across all sources.
/// Bounds prompt size regardless of how much content the pages contain.
const TOTAL_CONTEXT_MAX_CHARS: usize = 8_000;

/// Title character cap (defensive against pathological titles).
const TITLE_MAX_CHARS: usize = 160;

/// Hard cap on how many distinct queries the planner may run for one turn.
const MAX_QUERIES: usize = 3;

/// Upper bound on results per provider call / merged total.
const MAX_RESULTS_HARD: usize = 10;

/// A browser-like User-Agent. The DuckDuckGo HTML endpoint returns an empty
/// page to obviously-automated clients; a normal UA gets normal results. This
/// is the same approach the popular `duckduckgo-search` library uses.
const BROWSER_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

/// A single web result. `content` holds the scraped page text (markdown) when a
/// content-fetching provider was used; `snippet` is the short description that's
/// always available. The model prefers `content` and falls back to `snippet`.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    /// Full page content (markdown) when available; empty otherwise.
    #[serde(default)]
    pub content: String,
}

/// Appended to the system prompt only on turns where web results were found.
/// Tells the model to ground its answer in the provided source content and —
/// crucially for this panel — to write clean conversational prose. No citation
/// brackets (`[1]`), no raw URLs, no Markdown tables: the panel is small and
/// replies are often read aloud, so that formatting just becomes noise.
pub const WEB_SEARCH_SYSTEM_DIRECTIVE: &str = "Live web search results — including full excerpts from the source pages — are included with the user's message. Base your answer on them: synthesize across the sources, lead with the direct answer, and be specific with the concrete facts, numbers, and dates the sources provide. Write in plain, natural prose. Do NOT add citation markers, source numbers, brackets like [1], or raw URLs, and do NOT use Markdown tables — this is a small chat panel and replies may be read aloud. If the sources genuinely don't contain the answer, say so plainly instead of guessing.";

// ---------------------------------------------------------------------------
// Search planner (decides whether to search + rewrites the query)
// ---------------------------------------------------------------------------

/// Instruction block for the planner LLM. Kept provider-agnostic; the concrete
/// JSON shape is enforced by a strict schema when the provider supports it, and
/// described in-prose otherwise.
const PLANNER_SYSTEM_PROMPT: &str = "You are the search planner for a voice assistant. The user's latest message may be a rough, possibly garbled voice transcription (filler words, misheard proper nouns, missing punctuation) in any language.\n\nDecide whether answering it well requires a live web search. Search for: current events, news, prices, weather, sports scores, schedules, product releases/versions, people's current roles, and any niche or time-sensitive fact that a model's training data would not reliably know. Do NOT search for: greetings or small talk, questions about the assistant itself, or things the model can do from its own knowledge (general explanations, definitions, writing, brainstorming, coding, math, translation).\n\nWhen a search is warranted, rewrite the request into 1 to 3 focused, keyword-rich web search queries: correct likely transcription errors and proper names, strip filler, expand only where it clearly helps recall, and split a multi-part question into separate queries. Use the conversation context to make follow-up questions self-contained (resolve pronouns like \"it\"/\"they\" to the actual subject). Keep each query concise, as a person would type it into a search box.\n\nPick a freshness window: \"day\", \"week\", \"month\", or \"year\" for time-sensitive topics (more recent = tighter), or \"none\" when recency doesn't matter.";

/// The planner's structured decision.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchPlan {
    /// Whether a live web search should run for this turn.
    #[serde(default)]
    pub needs_search: bool,
    /// Cleaned, search-ready queries (1–3 after sanitizing).
    #[serde(default)]
    pub queries: Vec<String>,
    /// Freshness window: "none" | "day" | "week" | "month" | "year".
    #[serde(default = "default_freshness")]
    pub freshness: String,
}

fn default_freshness() -> String {
    "none".to_string()
}

impl SearchPlan {
    /// A trivial plan that searches the raw user text verbatim. Used as the
    /// fallback when the planner is unavailable (built-in model) or errors, so
    /// the feature degrades to "search the question as typed" rather than off.
    pub fn raw(user_text: &str) -> Self {
        let q = user_text.trim();
        SearchPlan {
            needs_search: !q.is_empty(),
            queries: if q.is_empty() {
                Vec::new()
            } else {
                vec![truncate_chars(q, 480)]
            },
            freshness: "none".to_string(),
        }
    }

    /// Clean up model output: trim/dedupe/cap queries and normalize freshness.
    fn sanitize(&mut self, user_text: &str) {
        let mut seen = HashSet::new();
        let mut cleaned = Vec::new();
        for q in self.queries.drain(..) {
            let q = q.trim();
            if q.is_empty() {
                continue;
            }
            // Firecrawl caps queries at 500 chars; keep a little headroom.
            let q = truncate_chars(q, 480);
            if seen.insert(q.to_lowercase()) {
                cleaned.push(q);
            }
            if cleaned.len() >= MAX_QUERIES {
                break;
            }
        }
        self.queries = cleaned;

        // If the model wants a search but gave no usable query, fall back to the
        // raw request rather than silently skipping the search.
        if self.needs_search && self.queries.is_empty() {
            let q = user_text.trim();
            if q.is_empty() {
                self.needs_search = false;
            } else {
                self.queries.push(truncate_chars(q, 480));
            }
        }

        self.freshness = match self.freshness.trim().to_lowercase().as_str() {
            "day" | "week" | "month" | "year" | "none" => self.freshness.trim().to_lowercase(),
            "hour" => "day".to_string(),
            _ => "none".to_string(),
        };
    }
}

/// Ask the assistant's own (capable) model to plan the search: decide whether
/// it's needed and produce clean queries + a freshness window. Returns an error
/// on any LLM/parse failure so the caller can fall back to the raw query.
pub async fn plan_search(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    supports_structured_output: bool,
    recent_context: &str,
    user_text: &str,
) -> Result<SearchPlan, String> {
    let today = chrono::Local::now().format("%A, %B %-d, %Y").to_string();

    let mut system = String::with_capacity(PLANNER_SYSTEM_PROMPT.len() + 64);
    system.push_str(PLANNER_SYSTEM_PROMPT);
    system.push_str("\n\nToday's date is ");
    system.push_str(&today);
    system.push('.');

    let mut user = String::new();
    if !recent_context.trim().is_empty() {
        user.push_str("Conversation so far:\n");
        user.push_str(recent_context.trim());
        user.push_str("\n\n");
    }
    user.push_str("New user request (may be a rough voice transcription): \"");
    user.push_str(user_text.trim());
    user.push('"');

    let schema = if supports_structured_output {
        Some(json!({
            "type": "object",
            "properties": {
                "needs_search": { "type": "boolean" },
                "queries": { "type": "array", "items": { "type": "string" } },
                "freshness": { "type": "string", "enum": ["none", "day", "week", "month", "year"] }
            },
            "required": ["needs_search", "queries", "freshness"],
            "additionalProperties": false
        }))
    } else {
        system.push_str("\n\nReply with ONLY a JSON object of this exact shape, no prose and no code fences: {\"needs_search\": true|false, \"queries\": [\"...\"], \"freshness\": \"none|day|week|month|year\"}.");
        None
    };

    debug!("Planning web search for {:?}", user_text);

    let raw = llm_client::send_chat_completion_with_schema(
        provider,
        api_key,
        model,
        user,
        Some(system),
        schema,
        None,
        None,
    )
    .await?
    .ok_or_else(|| "Search planner returned an empty response".to_string())?;

    let mut plan = parse_plan(&raw).ok_or_else(|| {
        format!(
            "Could not parse a search plan from the model output: {}",
            truncate_chars(raw.trim(), 200)
        )
    })?;
    plan.sanitize(user_text);
    debug!(
        "Search plan: needs_search={}, freshness={}, queries={:?}",
        plan.needs_search, plan.freshness, plan.queries
    );
    Ok(plan)
}

/// Parse the planner's JSON, tolerating models that wrap it in prose or fences.
fn parse_plan(raw: &str) -> Option<SearchPlan> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_matches('`')
        .trim();
    if let Ok(p) = serde_json::from_str::<SearchPlan>(trimmed) {
        return Some(p);
    }
    // Fall back to the first {...} block anywhere in the text.
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<SearchPlan>(&raw[start..=end]).ok()
}

// ---------------------------------------------------------------------------
// Retrieval
// ---------------------------------------------------------------------------

/// Run a single web search using the provider configured in settings. This is
/// the one-off entry used by the settings "Test search" button; it surfaces
/// provider errors (missing key, rate limit) to the caller.
pub async fn search(settings: &AppSettings, query: &str) -> Result<Vec<SearchResult>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let max_results =
        (settings.assistant_web_search_max_results as usize).clamp(1, MAX_RESULTS_HARD);
    run_provider_search(
        settings,
        query,
        "none",
        settings.assistant_web_search_fetch_content,
        max_results,
    )
    .await
}

/// Execute a full search plan: run each query (in parallel), then merge and
/// de-duplicate the results, interleaving across queries for source diversity
/// and capping the total. Per-query errors are swallowed (logged) so one bad
/// query never sinks the whole turn.
pub async fn search_with_plan(settings: &AppSettings, plan: &SearchPlan) -> Vec<SearchResult> {
    let queries: Vec<&String> = plan
        .queries
        .iter()
        .filter(|q| !q.trim().is_empty())
        .take(MAX_QUERIES)
        .collect();
    if queries.is_empty() {
        return Vec::new();
    }

    let max_results =
        (settings.assistant_web_search_max_results as usize).clamp(1, MAX_RESULTS_HARD);
    let fetch_content = settings.assistant_web_search_fetch_content;
    let n = queries.len();
    // Spread the result budget across queries (at least 2 each), then cap the
    // merged total so multi-query turns don't flood the prompt.
    let per_query = if n <= 1 {
        max_results
    } else {
        max_results.div_ceil(n).max(2)
    };
    let total_cap = if n <= 1 {
        max_results
    } else {
        (max_results + 2).min(MAX_RESULTS_HARD)
    };

    let futures = queries.iter().map(|q| {
        run_provider_search(
            settings,
            q.as_str(),
            &plan.freshness,
            fetch_content,
            per_query,
        )
    });
    let per_query_results = futures_util::future::join_all(futures).await;

    let lists: Vec<Vec<SearchResult>> = per_query_results
        .into_iter()
        .enumerate()
        .map(|(i, r)| match r {
            Ok(v) => v,
            Err(e) => {
                warn!("Web search for {:?} failed: {}", queries[i], e);
                Vec::new()
            }
        })
        .collect();

    // Round-robin merge so each query contributes near the top, de-duping by URL
    // (or title when a result has no URL).
    let mut seen: HashSet<String> = HashSet::new();
    let mut merged: Vec<SearchResult> = Vec::new();
    let max_len = lists.iter().map(|v| v.len()).max().unwrap_or(0);
    'outer: for i in 0..max_len {
        for list in &lists {
            if let Some(r) = list.get(i) {
                let key = dedupe_key(r);
                if key.is_empty() || seen.insert(key) {
                    merged.push(r.clone());
                    if merged.len() >= total_cap {
                        break 'outer;
                    }
                }
            }
        }
    }
    merged
}

/// De-duplication key: normalized URL, falling back to the lowercased title.
fn dedupe_key(r: &SearchResult) -> String {
    let url = r.url.trim().trim_end_matches('/').to_lowercase();
    if !url.is_empty() {
        url
    } else {
        r.title.trim().to_lowercase()
    }
}

/// Dispatch one query to the configured provider.
async fn run_provider_search(
    settings: &AppSettings,
    query: &str,
    freshness: &str,
    fetch_content: bool,
    max_results: usize,
) -> Result<Vec<SearchResult>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let provider = settings.assistant_web_search_provider.as_str();
    debug!(
        "Web search via '{}' for {:?} (max {}, freshness {}, content {})",
        provider, query, max_results, freshness, fetch_content
    );

    match provider {
        "firecrawl" => {
            let key = settings
                .web_search_api_keys
                .get("firecrawl")
                .cloned()
                .unwrap_or_default();
            search_firecrawl(
                &key,
                query,
                max_results,
                freshness_to_tbs(freshness),
                fetch_content,
            )
            .await
        }
        "brave" => {
            let key = settings
                .web_search_api_keys
                .get("brave")
                .cloned()
                .unwrap_or_default();
            search_brave(&key, query, max_results, freshness).await
        }
        // "duckduckgo" and any unknown value fall back to the free engine.
        _ => search_duckduckgo(query, max_results, freshness).await,
    }
}

/// Map a freshness window to a Firecrawl `tbs` value (Google-style time filter).
fn freshness_to_tbs(freshness: &str) -> Option<&'static str> {
    match freshness {
        "day" => Some("qdr:d"),
        "week" => Some("qdr:w"),
        "month" => Some("qdr:m"),
        "year" => Some("qdr:y"),
        _ => None,
    }
}

/// Map a freshness window to a Brave `freshness` value.
fn freshness_to_brave(freshness: &str) -> Option<&'static str> {
    match freshness {
        "day" => Some("pd"),
        "week" => Some("pw"),
        "month" => Some("pm"),
        "year" => Some("py"),
        _ => None,
    }
}

/// Map a freshness window to a DuckDuckGo `df` value (year unsupported there).
fn freshness_to_ddg(freshness: &str) -> Option<&'static str> {
    match freshness {
        "day" => Some("d"),
        "week" => Some("w"),
        "month" => Some("m"),
        _ => None,
    }
}

/// Format results as a context block to include with the user's message. Each
/// source gets its title and, within a shared character budget, its full
/// content (or snippet). Deliberately no numbered citations or URLs, so the
/// model has nothing to echo back as `[1]`-style markers and stays in clean
/// prose:
///
/// ```text
/// Live web search results:
///
/// ---
/// Source: Title
/// <page content excerpt>
/// ```
pub fn format_results_for_prompt(results: &[SearchResult]) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("Live web search results:\n");
    let mut budget = TOTAL_CONTEXT_MAX_CHARS;
    for r in results {
        out.push_str("\n---\nSource: ");
        out.push_str(&r.title);
        out.push('\n');
        let body = if !r.content.is_empty() {
            r.content.as_str()
        } else {
            r.snippet.as_str()
        };
        if !body.is_empty() && budget > 0 {
            let take = body.chars().count().min(budget);
            out.extend(body.chars().take(take));
            out.push('\n');
            budget = budget.saturating_sub(take);
        }
    }
    out
}

/// Fast, allocation-light heuristic used as a cheap *pre-gate* before the LLM
/// planner: it skips clear non-search work (chit-chat, questions about the
/// assistant, text-generation/coding tasks, pure arithmetic, "explain/define"
/// requests) so we don't spend a planner round-trip on them. Everything else
/// proceeds to the planner, which makes the real decision and crafts queries.
pub fn should_search(query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return false;
    }

    // Very short greetings / acknowledgements — never worth a search.
    const SMALL_TALK: [&str; 16] = [
        "hi",
        "hey",
        "hello",
        "yo",
        "sup",
        "thanks",
        "thank you",
        "ty",
        "ok",
        "okay",
        "cool",
        "nice",
        "lol",
        "bye",
        "good morning",
        "good night",
    ];
    if SMALL_TALK.contains(&q.as_str()) {
        return false;
    }

    // Questions about the assistant itself don't need the web.
    const SELF_REFERENTIAL: [&str; 8] = [
        "who are you",
        "what are you",
        "what can you do",
        "what is your name",
        "your name",
        "help me",
        "what do you do",
        "introduce yourself",
    ];
    if SELF_REFERENTIAL.iter().any(|p| q.contains(p)) {
        return false;
    }

    // Clear non-search tasks: generation, transformation, or coding. These are
    // about producing/altering text the user supplies, not looking things up.
    const TASK_PREFIXES: [&str; 18] = [
        "write ",
        "compose ",
        "draft ",
        "create ",
        "generate ",
        "make a ",
        "translate ",
        "summarize ",
        "summarise ",
        "rewrite ",
        "rephrase ",
        "paraphrase ",
        "fix ",
        "refactor ",
        "debug ",
        "improve ",
        "correct ",
        "proofread ",
    ];
    if TASK_PREFIXES.iter().any(|p| q.starts_with(p)) {
        return false;
    }

    // Code is a strong non-search signal.
    if q.contains("```") || q.contains("def ") || q.contains("function ") {
        return false;
    }

    // Pure arithmetic like "12 * (3 + 4)" — the model can do this directly.
    if is_simple_math(&q) {
        return false;
    }

    // Conceptual "teach me" requests are best answered from the model's own
    // knowledge rather than the live web. Kept deliberately narrow (only the
    // clearest openers) so genuine lookups still reach the planner.
    const CONCEPTUAL_PREFIXES: [&str; 5] = [
        "explain ",
        "define ",
        "what is a ",
        "what is an ",
        "what does it mean",
    ];
    if CONCEPTUAL_PREFIXES.iter().any(|p| q.starts_with(p)) {
        return false;
    }

    // Anything that survives the filters proceeds to the planner.
    true
}

/// Rough detector for "this is just arithmetic": only digits, whitespace and
/// math operators, and at least one operator present.
fn is_simple_math(q: &str) -> bool {
    let mut has_op = false;
    for c in q.chars() {
        match c {
            '0'..='9' | ' ' | '.' | ',' | '(' | ')' => {}
            '+' | '-' | '*' | '/' | '^' | '%' | '=' => has_op = true,
            _ => return false,
        }
    }
    has_op
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

/// Build a reqwest client with the given timeout and a browser User-Agent.
fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(BROWSER_UA)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// Firecrawl `/v2/search`. With `scrapeOptions.formats = ["markdown"]` it returns
/// the full page content of each result as markdown — the key to grounded,
/// complete answers (snippets alone are too thin). `tbs` applies a freshness
/// window; `parsers: []` and `proxy: "auto"` keep credit cost in check.
async fn search_firecrawl(
    api_key: &str,
    query: &str,
    max_results: usize,
    tbs: Option<&str>,
    fetch_content: bool,
) -> Result<Vec<SearchResult>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Firecrawl API key is not set. Add it in Settings → Assistant → Web Search."
                .to_string(),
        );
    }

    let timeout = if fetch_content {
        FIRECRAWL_TIMEOUT
    } else {
        REQUEST_TIMEOUT
    };
    let client = http_client(timeout)?;

    let mut body = json!({
        "query": query,
        "limit": max_results,
        "sources": ["web"],
        // Server-side timeout, kept under our client timeout.
        "timeout": 40000,
    });
    if let Some(tbs) = tbs {
        body["tbs"] = json!(tbs);
    }
    if fetch_content {
        // Pull the main page content as markdown for each result. `onlyMainContent`
        // strips nav/boilerplate; `parsers: []` avoids paid PDF parsing.
        body["scrapeOptions"] = json!({
            "formats": ["markdown"],
            "onlyMainContent": true,
            "parsers": [],
            "proxy": "auto",
        });
    }

    let resp = client
        .post("https://api.firecrawl.dev/v2/search")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Firecrawl request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Firecrawl search failed ({}): {}",
            status,
            truncate_chars(&text, 200)
        ));
    }

    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Firecrawl response: {}", e))?;

    let mut results = Vec::new();
    if let Some(items) = value
        .get("data")
        .and_then(|d| d.get("web"))
        .and_then(|w| w.as_array())
    {
        for item in items {
            let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let snippet = item
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // Prefer scraped markdown; fall back to a "summary" field if present.
            let content = item
                .get("markdown")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("summary").and_then(|v| v.as_str()))
                .unwrap_or("");
            push_result(&mut results, title, url, snippet, content);
            if results.len() >= max_results {
                break;
            }
        }
    }
    Ok(results)
}

/// Brave Web Search API. JSON results live at `web.results[]`; descriptions can
/// contain `<strong>` highlight tags, which we strip. Snippet-only (Brave does
/// not return page bodies).
async fn search_brave(
    api_key: &str,
    query: &str,
    max_results: usize,
    freshness: &str,
) -> Result<Vec<SearchResult>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Brave Search API key is not set. Add it in Settings → Assistant → Web Search."
                .to_string(),
        );
    }

    let client = http_client(REQUEST_TIMEOUT)?;
    // Brave caps `count` at 20.
    let count = max_results.clamp(1, 20).to_string();

    let mut query_params: Vec<(&str, String)> = vec![("q", query.to_string()), ("count", count)];
    if let Some(f) = freshness_to_brave(freshness) {
        query_params.push(("freshness", f.to_string()));
    }

    let resp = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", api_key)
        .header(reqwest::header::ACCEPT, "application/json")
        .query(&query_params)
        .send()
        .await
        .map_err(|e| format!("Brave request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Brave search failed ({}): {}",
            status,
            truncate_chars(&text, 200)
        ));
    }

    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Brave response: {}", e))?;

    let mut results = Vec::new();
    if let Some(items) = value
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array())
    {
        for item in items {
            let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let raw_snippet = item
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let snippet = clean_html_text(raw_snippet);
            push_result(&mut results, title, url, &snippet, "");
            if results.len() >= max_results {
                break;
            }
        }
    }
    Ok(results)
}

/// DuckDuckGo via the keyless HTML endpoint. We POST the query and parse the
/// returned HTML for result links + snippets. Free, no account, no API key, but
/// snippet-only and occasionally rate-limited.
async fn search_duckduckgo(
    query: &str,
    max_results: usize,
    freshness: &str,
) -> Result<Vec<SearchResult>, String> {
    let client = http_client(REQUEST_TIMEOUT)?;

    let mut form: Vec<(&str, &str)> = vec![("q", query)];
    if let Some(df) = freshness_to_ddg(freshness) {
        form.push(("df", df));
    }

    let resp = client
        .post("https://html.duckduckgo.com/html/")
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("DuckDuckGo request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("DuckDuckGo search failed ({})", status));
    }

    let html = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read DuckDuckGo response: {}", e))?;

    let results = parse_duckduckgo_html(&html, max_results);
    if results.is_empty() {
        // Either a genuine no-results page or a rate-limit/challenge response.
        warn!("DuckDuckGo returned no parseable results (possible rate limit)");
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// DuckDuckGo HTML parsing
// ---------------------------------------------------------------------------

static DDG_TITLE_RE: Lazy<Regex> = Lazy::new(|| {
    // The result title anchor: `<a ... class="result__a" ... href="URL">TITLE</a>`.
    // `(?s)` lets `.` span newlines inside the title.
    Regex::new(r#"(?s)<a[^>]*class="[^"]*result__a[^"]*"[^>]*href="([^"]*)"[^>]*>(.*?)</a>"#)
        .expect("valid DDG title regex")
});

static DDG_SNIPPET_RE: Lazy<Regex> = Lazy::new(|| {
    // The snippet anchor: `<a ... class="result__snippet" ...>SNIPPET</a>`.
    Regex::new(r#"(?s)<a[^>]*class="[^"]*result__snippet[^"]*"[^>]*>(.*?)</a>"#)
        .expect("valid DDG snippet regex")
});

/// Parse the DuckDuckGo HTML results page. Pairs each title link with the
/// snippet that follows it (by document position, so missing snippets and
/// interleaved ads don't misalign the pairs), decodes DDG's redirect URLs, and
/// drops sponsored results.
fn parse_duckduckgo_html(html: &str, max_results: usize) -> Vec<SearchResult> {
    // Collect (position, url, title) for every result link.
    let titles: Vec<(usize, String, String)> = DDG_TITLE_RE
        .captures_iter(html)
        .filter_map(|c| {
            let pos = c.get(0)?.start();
            let href = c.get(1)?.as_str();
            let title = clean_html_text(c.get(2)?.as_str());
            Some((pos, decode_ddg_url(href), title))
        })
        .collect();

    // Collect (position, snippet) for every snippet.
    let snippets: Vec<(usize, String)> = DDG_SNIPPET_RE
        .captures_iter(html)
        .filter_map(|c| {
            let pos = c.get(0)?.start();
            let text = clean_html_text(c.get(1)?.as_str());
            Some((pos, text))
        })
        .collect();

    let mut results = Vec::new();
    for (i, (t_pos, url, title)) in titles.iter().enumerate() {
        // Skip sponsored results (DuckDuckGo routes ad clicks through y.js).
        if url.contains("duckduckgo.com/y.js") || url.is_empty() {
            continue;
        }
        if title.is_empty() {
            continue;
        }

        // The matching snippet is the first one positioned after this title and
        // before the next title.
        let next_pos = titles.get(i + 1).map(|(p, _, _)| *p).unwrap_or(usize::MAX);
        let snippet = snippets
            .iter()
            .find(|(s_pos, _)| *s_pos > *t_pos && *s_pos < next_pos)
            .map(|(_, s)| s.clone())
            .unwrap_or_default();

        push_result(&mut results, title, url, &snippet, "");
        if results.len() >= max_results {
            break;
        }
    }
    results
}

/// DuckDuckGo wraps result links in a redirect: `//duckduckgo.com/l/?uddg=<percent-encoded-url>&...`.
/// Extract and decode the real destination. Handles bare `//host` and direct
/// `http(s)://` hrefs too.
fn decode_ddg_url(href: &str) -> String {
    if let Some(idx) = href.find("uddg=") {
        let rest = &href[idx + 5..];
        let encoded = rest.split('&').next().unwrap_or(rest);
        return percent_decode(encoded);
    }
    if let Some(stripped) = href.strip_prefix("//") {
        return format!("https://{}", stripped);
    }
    href.to_string()
}

/// Minimal `%XX` percent-decoder (UTF-8 aware). Avoids pulling in the `url`
/// crate just for this one path.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

static HTML_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]*>").expect("valid tag regex"));
static MULTI_NEWLINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\n{3,}").expect("valid newline regex"));

/// Strip HTML tags, unescape common entities, and collapse whitespace.
fn clean_html_text(input: &str) -> String {
    let no_tags = HTML_TAG_RE.replace_all(input, "");
    let unescaped = no_tags
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&#x2F;", "/")
        .replace("&nbsp;", " ");
    // Collapse runs of whitespace into single spaces.
    unescaped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Clean scraped page content: trim, collapse excessive blank lines, and cap to
/// the per-result content budget. Markdown structure is otherwise preserved.
fn sanitize_content(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let collapsed = MULTI_NEWLINE_RE.replace_all(trimmed, "\n\n");
    truncate_chars(collapsed.trim(), CONTENT_MAX_CHARS)
}

/// Truncate to at most `max` characters on a char boundary, adding an ellipsis
/// when content was dropped.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Push a cleaned, bounded result, skipping entries with neither title nor URL.
fn push_result(
    results: &mut Vec<SearchResult>,
    title: &str,
    url: &str,
    snippet: &str,
    content: &str,
) {
    let title = truncate_chars(title.trim(), TITLE_MAX_CHARS);
    let url = url.trim().to_string();
    let snippet = truncate_chars(snippet.trim(), SNIPPET_MAX_CHARS);
    let content = sanitize_content(content);
    if title.is_empty() && url.is_empty() {
        return;
    }
    results.push(SearchResult {
        title,
        url,
        snippet,
        content,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_search_skips_small_talk_and_tasks() {
        assert!(!should_search("hi"));
        assert!(!should_search("thanks"));
        assert!(!should_search("write a haiku about the sea"));
        assert!(!should_search("translate this to French"));
        assert!(!should_search("who are you"));
        assert!(!should_search("12 * (3 + 4)"));
        // Conceptual "teach me" requests answer from the model's own knowledge.
        assert!(!should_search("explain how recursion works"));
        assert!(!should_search("define entropy"));
    }

    #[test]
    fn should_search_passes_lookups_to_planner() {
        assert!(should_search("who is the prime minister of canada"));
        assert!(should_search("what's the weather in Paris"));
        assert!(should_search("latest iphone price"));
        assert!(should_search("prime minister of canada"));
        assert!(should_search("tesla q3 earnings"));
    }

    #[test]
    fn freshness_maps_to_provider_params() {
        assert_eq!(freshness_to_tbs("day"), Some("qdr:d"));
        assert_eq!(freshness_to_tbs("year"), Some("qdr:y"));
        assert_eq!(freshness_to_tbs("none"), None);
        assert_eq!(freshness_to_brave("week"), Some("pw"));
        assert_eq!(freshness_to_ddg("month"), Some("m"));
        assert_eq!(freshness_to_ddg("year"), None); // DDG has no year filter
    }

    #[test]
    fn parse_plan_handles_plain_and_fenced_json() {
        let plain = r#"{"needs_search": true, "queries": ["a", "b"], "freshness": "week"}"#;
        let p = parse_plan(plain).expect("plain json");
        assert!(p.needs_search);
        assert_eq!(p.queries.len(), 2);
        assert_eq!(p.freshness, "week");

        let fenced = "Sure!\n```json\n{\"needs_search\": false, \"queries\": [], \"freshness\": \"none\"}\n```";
        let p = parse_plan(fenced).expect("fenced json");
        assert!(!p.needs_search);
        assert!(p.queries.is_empty());
    }

    #[test]
    fn sanitize_dedupes_caps_and_fixes_freshness() {
        let mut plan = SearchPlan {
            needs_search: true,
            queries: vec![
                "  Tesla earnings ".to_string(),
                "tesla earnings".to_string(), // dup (case/space)
                "tesla stock".to_string(),
                "tesla news".to_string(),
                "tesla revenue".to_string(), // beyond MAX_QUERIES
            ],
            freshness: "HOUR".to_string(),
        };
        plan.sanitize("tesla earnings");
        assert_eq!(plan.queries.len(), MAX_QUERIES);
        assert_eq!(plan.queries[0], "Tesla earnings");
        assert_eq!(plan.freshness, "day"); // "hour" normalized to "day"
    }

    #[test]
    fn sanitize_falls_back_to_raw_when_no_queries() {
        let mut plan = SearchPlan {
            needs_search: true,
            queries: vec![],
            freshness: "bogus".to_string(),
        };
        plan.sanitize("who won the game last night");
        assert_eq!(
            plan.queries,
            vec!["who won the game last night".to_string()]
        );
        assert_eq!(plan.freshness, "none"); // unknown normalized to "none"
    }

    #[test]
    fn raw_plan_searches_the_question() {
        let p = SearchPlan::raw("  population of japan  ");
        assert!(p.needs_search);
        assert_eq!(p.queries, vec!["population of japan".to_string()]);
        let empty = SearchPlan::raw("   ");
        assert!(!empty.needs_search);
        assert!(empty.queries.is_empty());
    }

    #[test]
    fn decode_ddg_url_handles_redirect_and_plain() {
        assert_eq!(
            decode_ddg_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa%3Fb%3Dc&rut=x"),
            "https://example.com/a?b=c"
        );
        assert_eq!(
            decode_ddg_url("//example.com/path"),
            "https://example.com/path"
        );
        assert_eq!(decode_ddg_url("https://example.com"), "https://example.com");
    }

    #[test]
    fn parse_duckduckgo_html_extracts_pairs() {
        let html = r#"
            <div class="result results_links">
              <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fen.wikipedia.org%2Fwiki%2FCanada&rut=z">Prime Minister of <b>Canada</b></a>
              <a class="result__snippet" href="x">The current prime minister is someone &amp; notable.</a>
            </div>
            <div class="result results_links result--ad">
              <a class="result__a" href="//duckduckgo.com/y.js?ad=1">Sponsored</a>
              <a class="result__snippet" href="x">an ad</a>
            </div>
        "#;
        let results = parse_duckduckgo_html(html, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Prime Minister of Canada");
        assert_eq!(results[0].url, "https://en.wikipedia.org/wiki/Canada");
        assert!(results[0].snippet.contains("current prime minister"));
        assert!(results[0].snippet.contains("&")); // entity unescaped
    }

    #[test]
    fn format_results_use_content_without_numbers_or_urls() {
        let results = vec![
            SearchResult {
                title: "Alpha".to_string(),
                url: "https://a.com".to_string(),
                snippet: "snip a".to_string(),
                content: "Full page content about Alpha.".to_string(),
            },
            SearchResult {
                title: "Beta".to_string(),
                url: "https://b.com".to_string(),
                snippet: "snippet for beta".to_string(),
                content: String::new(),
            },
        ];
        let block = format_results_for_prompt(&results);
        assert!(block.starts_with("Live web search results:"));
        assert!(block.contains("Source: Alpha"));
        assert!(block.contains("Full page content about Alpha."));
        // Falls back to snippet when content is empty.
        assert!(block.contains("Source: Beta"));
        assert!(block.contains("snippet for beta"));
        // No citation numbers and no URLs to tempt the model into echoing them.
        assert!(!block.contains("[1]"));
        assert!(!block.contains("https://"));
    }
}
