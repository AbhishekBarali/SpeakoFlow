//! Web search for the assistant.
//!
//! Goal: **fast but genuinely good** — the kind of answer-with-search that
//! ChatGPT / Gemini give in a few seconds while you talk to them, *not* the
//! minutes-long "deep research" mode. The whole pipeline is one retrieval pass
//! with heavy parallelism and tight timeouts.
//!
//! Pipeline:
//! 1. **Model-decided** (`run_tool_search`): the assistant model itself calls
//!    the `web_search` tool when it needs current facts, passing its own query,
//!    freshness window, and news flag — there is no separate planner or keyword
//!    gate deciding for it.
//! 2. **Snippet search** (`snippet_search`): run each query in parallel against
//!    the configured provider and get back fast title+snippet results. News
//!    sources are pulled in when the model flags a current-events topic, and
//!    a freshness window is applied when the topic is time-sensitive.
//! 3. **Local rerank** (`rerank`): score the merged snippets by lexical overlap
//!    with the query plus a recency boost — purely local, no extra network or
//!    LLM call, so it costs ~nothing. The top sources are handed to the model.
//!
//! Snippet-first by design: result pages are never fetched/scraped, so a search
//! is a single fast HTTP round-trip per query and the model only ever sees
//! short titles + snippets (plus answer boxes / knowledge panels when the
//! provider returns them). A failed/slow search degrades gracefully — the turn
//! answers without web context rather than stalling.
//!
//! Providers (all snippet-first, all benefit from the local rerank, all use a
//! single API key): **Serper** (fast, cheap Google SERP — the default),
//! **Brave** (independent index), **Tavily** (LLM-optimized search + answer),
//! **Exa** (neural/semantic search), **SerpAPI** (Google SERP), and
//! **TinyFish** (own index, agent-oriented, free tier with no card). Any unknown
//! or legacy provider value routes to Serper.

use crate::settings::{AppSettings, AssistantSearchDepth};
use log::{debug, warn};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use serde_json::json;
use specta::Type;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Timeout for a snippet search HTTP call (Serper, Brave, Tavily, Exa, SerpAPI,
/// TinyFish).
///
/// A search happens inside a wait the user is sitting through in silence, so the
/// ceiling is a UX number, not just a safety net. Sized against measured
/// provider latency: Serper averages ~0.8s (p99 ~2.1s) and the slowest supported
/// providers average ~5s, so 6s still lets a slow provider finish while cutting
/// the worst case a hung request can impose on a turn.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);

/// Per-snippet character cap.
const SNIPPET_MAX_CHARS: usize = 500;

/// Title character cap (defensive against pathological titles).
const TITLE_MAX_CHARS: usize = 160;

/// Absolute per-result content cap (a tier may ask for less).
const CONTENT_HARD_CAP: usize = 4_000;

/// Upper bound on results per provider call.
const MAX_RESULTS_HARD: usize = 10;

/// A browser-like User-Agent. Some endpoints (and scraped pages) return empty
/// or non-200 responses to obviously-automated clients; a normal UA helps.
const BROWSER_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

/// A single web result handed to the model. `content` holds scraped page text
/// (markdown) when available; `snippet` is the short description that's always
/// present. The model prefers `content` and falls back to `snippet`.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    /// Full page content (markdown) when available; empty otherwise.
    #[serde(default)]
    pub content: String,
}

/// Built and appended to the system prompt only on turns where web results were
/// found. Three things drive answer quality (and were previously broken):
///
/// 1. **Attribution.** The results are injected into the *user* message, so the
///    model kept saying "the search results you gave me" / "your search
///    results" — jarring, and the user's #1 complaint. The directive now frames
///    them explicitly as the assistant's *own* retrieval and forbids
///    user-attribution wording.
/// 2. **No hedging.** Models under-deliver ("I can only confirm one score") even
///    when several results are present. The directive demands a direct BLUF
///    answer, all relevant items, and bans "ask the user to clarify" when
///    results exist.
/// 3. **TTS-aware formatting.** The reply is spoken verbatim (after Markdown is
///    stripped) when TTS is on, so tables/headings read as gibberish. With TTS
///    on we ask for speech-friendly prose; with it off we allow the compact
///    tables/bullets that make answers look like ChatGPT's.
pub fn web_search_system_directive(tts_enabled: bool) -> String {
    let mut s = String::with_capacity(1300);
    s.push_str(
        "The user's message includes live web search results that YOU just retrieved with your \
         built-in web tool for this turn. Treat them as your own current findings and as ground \
         truth, trusting them over your prior knowledge when they conflict. Never describe them as \
         something the user gave, sent, pasted, or provided, and never use phrases like \"your \
         search results\" or \"the results you sent\" — to the user it must read as if you simply \
         know the current answer. Open with the direct answer in the very first sentence (the name, \
         number, score, or date asked for), then add the key supporting specifics. When the results \
         contain several relevant items (scores, prices, options), give them all rather than \
         undercounting or claiming only one is available. If coverage is only partial, answer what \
         the results do support and add at most one short line on what's missing — never refuse, \
         stall, or ask the user to clarify when relevant results are present. Prefer the most recent \
         and most authoritative sources for time-sensitive facts and note any real disagreement in a \
         few words. Do not output citation markers like [1], footnotes, or raw URLs; you may name a \
         source in plain words when it helps.",
    );
    if tts_enabled {
        s.push_str(
            " Your reply is read aloud, so write natural spoken prose: short, clear sentences. A \
             simple bullet list is fine when listing several items, but do not use tables, headings, \
             or code blocks.",
        );
    } else {
        s.push_str(
            " The panel renders Markdown, so after the opening answer you may use light formatting \
             where it genuinely helps: bold labels, short bullet lists, or a compact table for \
             scores, comparisons, or multi-item results. Keep it tight and skip large headings.",
        );
    }
    s
}

/// Added to the system prompt whenever web search is *enabled*, on EVERY turn —
/// whether or not this turn actually searched. Without it the model has no idea
/// the capability exists and falls back to "I can't browse the internet" on the
/// turns where the app chose not to auto-search, which is exactly the wrong
/// thing to say when the app *can* search. Byte-stable text, so it's safe for
/// provider-side prompt caching.
pub const WEB_SEARCH_CAPABILITY_NOTE: &str = "You have a live web search tool available in this app, and the user's current local date is provided with each message. Your training data has a cutoff and may be out of date, but you are NOT stuck in your training year: trust the provided current date, and rely on web search for anything time-sensitive (recent events, news, sports results, prices, releases, schedules, who currently holds a role) rather than answering from stale memory or assuming an old year. Use the tool ONLY when a question genuinely needs current or external facts. For greetings, small talk, opinions, advice, explanations, definitions, writing, coding, math, or anything you already know well, just answer directly — do NOT search. When a search is warranted the app runs it automatically and adds the results to the user's message; on a turn that arrives without results, never claim you cannot access the internet — if you're unsure about something current, give your best answer and offer to look it up.";

// ---------------------------------------------------------------------------
// Search plan (retrieval input)
// ---------------------------------------------------------------------------

/// A set of ready-to-run web queries plus retrieval hints. Built directly from
/// the model's `web_search` tool call (see `run_tool_search`) and consumed by
/// `search_with_plan`; the model itself decides whether and what to search, so
/// there is no separate planner or keyword heuristic anymore.
#[derive(Debug, Clone)]
pub struct SearchPlan {
    /// Cleaned, search-ready queries.
    pub queries: Vec<String>,
    /// Freshness window: "none" | "day" | "week" | "month" | "year".
    pub freshness: String,
    /// Whether to include news-source results (current events / breaking news).
    pub news: bool,
}

// ---------------------------------------------------------------------------
// Search depth tiers
// ---------------------------------------------------------------------------

/// Concrete retrieval knobs for a depth tier. All tiers are one snippet-only
/// pass; they differ only in breadth and how much text reaches the model.
#[derive(Clone, Copy, Debug)]
struct TierParams {
    /// Max queries to actually run from the plan.
    max_queries: usize,
    /// Results requested per source per query in the snippet phase.
    snippet_limit: usize,
    /// Total sources handed to the model.
    sources_out: usize,
    /// Total web-context cap across all sources (chars).
    total_budget_chars: usize,
}

fn tier_params(depth: AssistantSearchDepth) -> TierParams {
    match depth {
        AssistantSearchDepth::Low => TierParams {
            max_queries: 1,
            snippet_limit: 8,
            sources_out: 5,
            total_budget_chars: 7_000,
        },
        AssistantSearchDepth::Medium => TierParams {
            max_queries: 3,
            snippet_limit: 10,
            sources_out: 8,
            total_budget_chars: 14_000,
        },
        AssistantSearchDepth::High => TierParams {
            max_queries: 4,
            snippet_limit: 10,
            sources_out: 10,
            total_budget_chars: 24_000,
        },
    }
}

/// The total web-context character budget for a tier — used by the caller when
/// formatting results for the prompt.
pub fn context_budget_for(depth: AssistantSearchDepth) -> usize {
    tier_params(depth).total_budget_chars
}

// ---------------------------------------------------------------------------
// Retrieval
// ---------------------------------------------------------------------------

/// A candidate from the snippet phase.
#[derive(Debug, Clone)]
struct Candidate {
    title: String,
    url: String,
    snippet: String,
    /// True when this came from a news source — used for the recency boost.
    from_news: bool,
}

/// Run a single web search using the configured provider and return snippet
/// results. Used by the settings "Test search" button; surfaces provider errors
/// (missing key, rate limit, budget) to the caller.
pub async fn search(settings: &AppSettings, query: &str) -> Result<Vec<SearchResult>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let cands = snippet_search(settings, query, 5, None, false).await?;
    Ok(cands.into_iter().take(5).map(candidate_to_result).collect())
}

/// Run a single web search requested by the model's `web_search` tool call.
/// Reuses the full plan pipeline (parallel snippet search + local rerank) with
/// a one-query plan built from the tool arguments, so tool-calling turns get
/// the same retrieval quality as the planner path. Never errors: a failure
/// returns an empty list so the model can answer without web context.
pub async fn run_tool_search(
    settings: &AppSettings,
    query: &str,
    freshness: Option<&str>,
    news: bool,
) -> Vec<SearchResult> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let freshness = match freshness.map(|f| f.trim().to_ascii_lowercase()) {
        Some(ref f) if matches!(f.as_str(), "day" | "week" | "month" | "year") => f.clone(),
        _ => "none".to_string(),
    };
    let plan = SearchPlan {
        queries: vec![truncate_chars(query, 480)],
        freshness,
        news,
    };
    search_with_plan(settings, &plan).await
}

/// Execute a full search plan: snippet-search every query in parallel, then
/// merge + rerank locally and hand the top sources to the model. Per-query
/// errors are swallowed (logged) so one bad query never sinks the whole turn.
pub async fn search_with_plan(settings: &AppSettings, plan: &SearchPlan) -> Vec<SearchResult> {
    let tp = tier_params(settings.assistant_search_depth);

    let queries: Vec<&String> = plan
        .queries
        .iter()
        .filter(|q| !q.trim().is_empty())
        .take(tp.max_queries)
        .collect();
    if queries.is_empty() {
        return Vec::new();
    }

    // Recency-sensitive topics get news results and a date-restricted window.
    // Recency itself is applied in the local rerank, not as a Google sort order.
    let recency_sensitive = plan.news || matches!(plan.freshness.as_str(), "day" | "week");
    let include_news = plan.news || matches!(plan.freshness.as_str(), "day" | "week" | "month");
    let tbs = build_tbs(&plan.freshness);
    let tbs_ref = tbs.as_deref();

    // Stage 1 — snippet search, every query in parallel.
    let snippet_futs = queries.iter().map(|q| {
        snippet_search(
            settings,
            q.as_str(),
            tp.snippet_limit,
            tbs_ref,
            include_news,
        )
    });
    let per_query = futures_util::future::join_all(snippet_futs).await;

    let lists: Vec<Vec<Candidate>> = per_query
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

    let mut merged = round_robin_merge(lists);
    if merged.is_empty() {
        return Vec::new();
    }

    // Stage 2 — local rerank (lexical + recency). No network, no LLM.
    let primary = queries.first().map(|q| q.as_str()).unwrap_or("");
    rerank(primary, &mut merged, recency_sensitive);

    // Stage 3 — hand the top reranked snippets to the model.
    merged
        .into_iter()
        .take(tp.sources_out)
        .map(candidate_to_result)
        .collect()
}

/// Round-robin merge across per-query candidate lists, de-duping by URL (or
/// title when a result has no URL) so each query contributes near the top.
fn round_robin_merge(lists: Vec<Vec<Candidate>>) -> Vec<Candidate> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut merged: Vec<Candidate> = Vec::new();
    let max_len = lists.iter().map(|v| v.len()).max().unwrap_or(0);
    for i in 0..max_len {
        for list in &lists {
            if let Some(c) = list.get(i) {
                let key = dedupe_key(c);
                if key.is_empty() || seen.insert(key) {
                    merged.push(c.clone());
                }
            }
        }
    }
    merged
}

/// De-duplication key: normalized URL, falling back to the lowercased title.
fn dedupe_key(c: &Candidate) -> String {
    let url = c.url.trim().trim_end_matches('/').to_lowercase();
    if !url.is_empty() {
        url
    } else {
        c.title.trim().to_lowercase()
    }
}

/// Local relevance rerank: lexical overlap of query terms with title (weighted)
/// and snippet, plus a recency boost for news results when the topic is
/// time-sensitive. Stable order is preserved for ties.
fn rerank(query: &str, candidates: &mut Vec<Candidate>, recency_sensitive: bool) {
    let terms = query_terms(query);
    let mut scored: Vec<(f32, usize, Candidate)> = candidates
        .drain(..)
        .enumerate()
        .map(|(i, c)| {
            let s = candidate_score(&c, &terms, recency_sensitive);
            (s, i, c)
        })
        .collect();
    // Higher score first; original position breaks ties (stable).
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    *candidates = scored.into_iter().map(|(_, _, c)| c).collect();
}

/// Tokenize a query into meaningful lowercase terms (drop short words / stopwords).
fn query_terms(query: &str) -> Vec<String> {
    const STOP: [&str; 22] = [
        "the", "a", "an", "of", "to", "in", "is", "are", "was", "were", "what", "who", "when",
        "how", "and", "for", "on", "with", "about", "whats", "does", "did",
    ];
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !STOP.contains(w))
        .map(|w| w.to_string())
        .collect()
}

/// Score one candidate against the query terms (+ optional recency boost).
fn candidate_score(c: &Candidate, terms: &[String], recency_sensitive: bool) -> f32 {
    let title = c.title.to_lowercase();
    let snippet = c.snippet.to_lowercase();
    let mut hits = 0.0f32;
    for t in terms {
        if title.contains(t) {
            hits += 2.0;
        }
        if snippet.contains(t) {
            hits += 1.0;
        }
    }
    let denom = (terms.len() as f32).max(1.0);
    let mut score = hits / denom;
    if c.from_news {
        score += if recency_sensitive { 1.5 } else { 0.3 };
    }
    score
}

/// Build Google's `tbs` time filter, or `None` for an unfiltered search.
///
/// Deliberately a time *window* only, never a sort order. Google's `sbd:1`
/// ("sort by date") was applied here for any recency-sensitive turn, and it is
/// destructive: re-ranking the whole index by date routinely returns junk or an
/// empty set for a broad query ("2026 movie release calendar"), which then made
/// `search_serper` pay a second, sequential round trip to recover — the single
/// largest source of search latency on exactly the questions people ask an
/// assistant. Recency still matters, so it is applied where it is free: the
/// local rerank boosts fresh news results (see `rerank`).
fn build_tbs(freshness: &str) -> Option<String> {
    match freshness {
        "day" => Some("qdr:d".to_string()),
        "week" => Some("qdr:w".to_string()),
        "month" => Some("qdr:m".to_string()),
        "year" => Some("qdr:y".to_string()),
        _ => None,
    }
}

/// Dispatch one query to the configured provider, returning snippet candidates.
async fn snippet_search(
    settings: &AppSettings,
    query: &str,
    limit: usize,
    tbs: Option<&str>,
    include_news: bool,
) -> Result<Vec<Candidate>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, MAX_RESULTS_HARD);
    let provider = settings.assistant_web_search_provider.as_str();
    let key = |id: &str| {
        settings
            .web_search_api_keys
            .get(id)
            .cloned()
            .unwrap_or_default()
    };
    debug!(
        "Snippet search via '{}' for {:?} (limit {}, news {}, tbs {:?})",
        provider, query, limit, include_news, tbs
    );

    match provider {
        "brave" => {
            let results = search_brave(&key("brave"), query, limit, tbs).await?;
            Ok(results.into_iter().map(result_to_candidate).collect())
        }
        "tavily" => search_tavily(&key("tavily"), query, limit, tbs, include_news).await,
        "exa" => search_exa(&key("exa"), query, limit, tbs).await,
        "serpapi" => search_serpapi(&key("serpapi"), query, limit, tbs, include_news).await,
        "tinyfish" => search_tinyfish(&key("tinyfish"), query, limit, tbs).await,
        // "serper" is the default. Any unknown or legacy value (including the
        // removed "firecrawl"/"duckduckgo") also routes here so old settings
        // keep working.
        _ => search_serper(&key("serper"), query, limit, tbs, include_news).await,
    }
}

/// Format results as a context block to include with the user's message, within
/// `total_budget` characters across all sources. No numbered citations or URLs,
/// so the model has nothing to echo back and stays in clean prose.
pub fn format_results_for_prompt(results: &[SearchResult], total_budget: usize) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str(
        "[Web search results you retrieved for this turn — your own findings, NOT provided by the user]\n",
    );
    let mut budget = total_budget;
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

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

/// Build a reqwest client with the given timeout and a browser User-Agent.
///
/// Cached per timeout. A fresh `Client` carries a fresh connection pool, so
/// building one per search meant a DNS lookup and a full TLS handshake before
/// every query — a few hundred milliseconds of the user's silence, repeated on
/// every turn. Reusing the client keeps the connection to the search provider
/// warm between turns, exactly as `llm_client` does for providers.
fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    static CLIENTS: OnceLock<Mutex<HashMap<u64, reqwest::Client>>> = OnceLock::new();
    let cache = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let key = timeout.as_millis() as u64;
    if let Ok(clients) = cache.lock() {
        if let Some(client) = clients.get(&key) {
            return Ok(client.clone());
        }
    }
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(BROWSER_UA)
        // Keep the socket around between searches so a follow-up question
        // reuses the handshake instead of repeating it.
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;
    if let Ok(mut clients) = cache.lock() {
        clients.insert(key, client.clone());
    }
    Ok(client)
}

/// Tavily Search (`POST /search`) — an LLM-optimized search API returning clean
/// per-result snippets (`content`) plus an optional synthesized `answer`. Auth
/// is a Bearer token. Freshness maps to Tavily's `time_range`; a news topic
/// switches `topic` to "news".
async fn search_tavily(
    api_key: &str,
    query: &str,
    max_results: usize,
    tbs: Option<&str>,
    include_news: bool,
) -> Result<Vec<Candidate>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Tavily API key is not set. Add it in Settings → Assistant → Web Search.".to_string(),
        );
    }

    let client = http_client(REQUEST_TIMEOUT)?;
    let max = max_results.clamp(1, 20);
    let mut body = json!({
        "query": query,
        "max_results": max,
        "search_depth": "basic",
        "topic": if include_news { "news" } else { "general" },
        "include_answer": true,
    });
    if let Some(tr) = tavily_time_range_from_tbs(tbs) {
        body["time_range"] = json!(tr);
    }

    let resp = client
        .post("https://api.tavily.com/search")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Tavily request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Tavily search failed ({}): {}",
            status,
            truncate_chars(&text, 200)
        ));
    }

    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Tavily response: {}", e))?;

    let mut candidates = Vec::new();
    // Tavily's synthesized answer — highest value for quick facts; added first.
    if let Some(answer) = value.get("answer").and_then(|v| v.as_str()) {
        if !answer.trim().is_empty() {
            push_candidate(&mut candidates, "Answer", "", answer, false);
        }
    }
    if let Some(items) = value.get("results").and_then(|v| v.as_array()) {
        for item in items {
            let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let snippet = item.get("content").and_then(|v| v.as_str()).unwrap_or("");
            push_candidate(&mut candidates, title, url, snippet, include_news);
        }
    }
    Ok(candidates)
}

/// Exa Search (`POST /search`) — neural/semantic search. Auth is the `x-api-key`
/// header. We request `highlights` (with a short `text` fallback) as the snippet
/// and use the `fast` search type for low latency. Freshness maps to a
/// `startPublishedDate` lower bound.
async fn search_exa(
    api_key: &str,
    query: &str,
    max_results: usize,
    tbs: Option<&str>,
) -> Result<Vec<Candidate>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Exa API key is not set. Add it in Settings → Assistant → Web Search.".to_string(),
        );
    }

    let client = http_client(REQUEST_TIMEOUT)?;
    let num = max_results.clamp(1, 20);
    let mut body = json!({
        "query": query,
        "numResults": num,
        "type": "fast",
        "contents": {
            "highlights": true,
            "text": { "maxCharacters": 800 }
        }
    });
    if let Some(start) = exa_start_date_from_tbs(tbs) {
        body["startPublishedDate"] = json!(start);
    }

    let resp = client
        .post("https://api.exa.ai/search")
        .header("x-api-key", api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Exa request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Exa search failed ({}): {}",
            status,
            truncate_chars(&text, 200)
        ));
    }

    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Exa response: {}", e))?;

    let mut candidates = Vec::new();
    if let Some(items) = value.get("results").and_then(|v| v.as_array()) {
        for item in items {
            let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
            // Prefer the highlighted passages; fall back to summary, then text.
            let snippet = item
                .get("highlights")
                .and_then(|v| v.as_array())
                .map(|hs| {
                    hs.iter()
                        .filter_map(|h| h.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    item.get("summary")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .or_else(|| {
                    item.get("text")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_default();
            push_candidate(&mut candidates, title, url, &snippet, false);
        }
    }
    Ok(candidates)
}

/// SerpAPI Google Search (`GET /search.json`) — Google SERP JSON. Auth is the
/// `api_key` query param. Parses the answer box, knowledge graph, organic
/// results, and (when news-y) `news_results`. `tbs` is Google's own time filter,
/// passed straight through.
async fn search_serpapi(
    api_key: &str,
    query: &str,
    max_results: usize,
    tbs: Option<&str>,
    include_news: bool,
) -> Result<Vec<Candidate>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "SerpAPI key is not set. Add it in Settings → Assistant → Web Search.".to_string(),
        );
    }

    let client = http_client(REQUEST_TIMEOUT)?;
    let num = max_results.clamp(1, 20).to_string();
    let mut params: Vec<(&str, String)> = vec![
        ("engine", "google".to_string()),
        ("q", query.to_string()),
        ("num", num),
        ("api_key", api_key.to_string()),
    ];
    if let Some(tbs) = tbs {
        params.push(("tbs", tbs.to_string()));
    }

    let resp = client
        .get("https://serpapi.com/search.json")
        .query(&params)
        .send()
        .await
        .map_err(|e| format!("SerpAPI request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "SerpAPI search failed ({}): {}",
            status,
            truncate_chars(&text, 200)
        ));
    }

    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse SerpAPI response: {}", e))?;

    let mut candidates = Vec::new();

    // Answer box: Google's direct answer.
    if let Some(ab) = value.get("answer_box") {
        let title = ab.get("title").and_then(|v| v.as_str()).unwrap_or("Answer");
        let url = ab.get("link").and_then(|v| v.as_str()).unwrap_or("");
        let snippet = ab
            .get("answer")
            .and_then(|v| v.as_str())
            .or_else(|| ab.get("snippet").and_then(|v| v.as_str()))
            .unwrap_or("");
        if !snippet.is_empty() {
            push_candidate(&mut candidates, title, url, snippet, false);
        }
    }

    // Knowledge graph: structured entity facts.
    if let Some(kg) = value.get("knowledge_graph") {
        let title = kg.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let url = kg.get("website").and_then(|v| v.as_str()).unwrap_or("");
        let snippet = kg.get("description").and_then(|v| v.as_str()).unwrap_or("");
        if !snippet.is_empty() {
            push_candidate(&mut candidates, title, url, snippet, false);
        }
    }

    // Fresh news coverage, only when the planner flagged the turn as news-y.
    if include_news {
        if let Some(items) = value.get("news_results").and_then(|v| v.as_array()) {
            for item in items {
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let url = item.get("link").and_then(|v| v.as_str()).unwrap_or("");
                let base = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
                let date = item.get("date").and_then(|v| v.as_str()).unwrap_or("");
                let snippet = match (date.is_empty(), base.is_empty()) {
                    (false, false) => format!("{} — {}", date, base),
                    (false, true) => date.to_string(),
                    _ => base.to_string(),
                };
                push_candidate(&mut candidates, title, url, &snippet, true);
            }
        }
    }

    // Organic web results.
    if let Some(items) = value.get("organic_results").and_then(|v| v.as_array()) {
        for item in items {
            let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let url = item.get("link").and_then(|v| v.as_str()).unwrap_or("");
            let base = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
            let snippet = match item.get("date").and_then(|v| v.as_str()) {
                Some(date) if !date.is_empty() => format!("{} — {}", date, base),
                _ => base.to_string(),
            };
            push_candidate(&mut candidates, title, url, &snippet, false);
        }
    }

    Ok(candidates)
}

/// Brave Web Search API (snippet-only). `tbs` is ignored (Brave uses its own
/// `freshness` param, applied via the dedicated freshness mapping).
async fn search_brave(
    api_key: &str,
    query: &str,
    max_results: usize,
    tbs: Option<&str>,
) -> Result<Vec<SearchResult>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Brave Search API key is not set. Add it in Settings → Assistant → Web Search."
                .to_string(),
        );
    }

    let client = http_client(REQUEST_TIMEOUT)?;
    let count = max_results.clamp(1, 20).to_string();

    let mut query_params: Vec<(&str, String)> = vec![("q", query.to_string()), ("count", count)];
    if let Some(f) = brave_freshness_from_tbs(tbs) {
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

/// Serper.dev Google Search API (snippet-only). Fast (~1–2 s) and cheap
/// (1 credit per 10 results, generous free tier). Parses Google's answer box and
/// knowledge graph when present (great for quick facts), the organic results,
/// and — when the turn is news-y — `topStories`. `tbs` is Google's own time
/// filter and is passed straight through (e.g. `qdr:d`, `sbd:1,qdr:w`).
async fn search_serper(
    api_key: &str,
    query: &str,
    max_results: usize,
    tbs: Option<&str>,
    include_news: bool,
) -> Result<Vec<Candidate>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Serper API key is not set. Add it in Settings → Assistant → Web Search.".to_string(),
        );
    }

    let client = http_client(REQUEST_TIMEOUT)?;
    let num = max_results.clamp(1, 20);

    // Serper proxies live Google, and a 200 response intermittently comes back
    // with an *empty* result set — most often on the first, uncached hit for a
    // query, or when a `tbs` time window filters everything out. Verified
    // against the live API: the very next identical request returns the full
    // result set. Without a fallback the turn silently degrades to "no web
    // context" and the model answers from stale memory — which reads to the user
    // as "web search doesn't work".
    //
    // When a time filter is in play, the fallback runs *concurrently* rather
    // than after the first request fails. Sequentially it cost two full round
    // trips — the difference between a ~1s search and a ~3s one — and the
    // filtered request is the one that can come back empty, so waiting to find
    // out is the expensive way to learn it. The filtered results still win
    // whenever they exist; the unfiltered pass is only consulted if they don't.
    let candidates = match tbs {
        Some(tbs) => {
            let (filtered, unfiltered) = futures_util::future::join(
                serper_query(&client, api_key, query, num, Some(tbs), include_news),
                serper_query(&client, api_key, query, num, None, include_news),
            )
            .await;
            match filtered {
                Ok(hits) if !hits.is_empty() => hits,
                Ok(_) => {
                    debug!(
                        "Serper returned no results for {:?} within the {} window; using the unfiltered pass",
                        query, tbs
                    );
                    unfiltered?
                }
                // A hard failure on the filtered request (bad key, rate limit)
                // will have failed the other one too; report the original.
                Err(e) => return Err(e),
            }
        }
        None => {
            let hits = serper_query(&client, api_key, query, num, None, include_news).await?;
            if hits.is_empty() {
                debug!("Serper returned no results for {:?}; retrying once", query);
                serper_query(&client, api_key, query, num, None, include_news).await?
            } else {
                hits
            }
        }
    };

    debug!(
        "Serper returned {} candidate(s) for {:?}",
        candidates.len(),
        query
    );
    Ok(candidates)
}

/// Issue a single Serper request and parse it into candidates. Parses Google's
/// answer box and knowledge graph (when present), `topStories` (when the turn is
/// news-y), and the organic results. Split out from `search_serper` so the
/// caller can cheaply retry on an empty result set.
async fn serper_query(
    client: &reqwest::Client,
    api_key: &str,
    query: &str,
    num: usize,
    tbs: Option<&str>,
    include_news: bool,
) -> Result<Vec<Candidate>, String> {
    let mut body = json!({
        "q": query,
        "num": num,
    });
    if let Some(tbs) = tbs {
        body["tbs"] = json!(tbs);
    }

    let resp = client
        .post("https://google.serper.dev/search")
        .header("X-API-KEY", api_key)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Serper request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Serper search failed ({}): {}",
            status,
            truncate_chars(&text, 200)
        ));
    }

    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Serper response: {}", e))?;

    let mut candidates = Vec::new();

    // Answer box: Google's direct answer — highest value for quick facts. Added
    // first; the local rerank still scores it on its own merits.
    if let Some(ab) = value.get("answerBox") {
        let title = ab.get("title").and_then(|v| v.as_str()).unwrap_or("Answer");
        let url = ab.get("link").and_then(|v| v.as_str()).unwrap_or("");
        let snippet = ab
            .get("answer")
            .and_then(|v| v.as_str())
            .or_else(|| ab.get("snippet").and_then(|v| v.as_str()))
            .unwrap_or("");
        if !snippet.is_empty() {
            push_candidate(&mut candidates, title, url, snippet, false);
        }
    }

    // Knowledge graph: structured entity facts (people, places, orgs).
    if let Some(kg) = value.get("knowledgeGraph") {
        let title = kg.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let url = kg
            .get("website")
            .and_then(|v| v.as_str())
            .or_else(|| kg.get("descriptionLink").and_then(|v| v.as_str()))
            .unwrap_or("");
        let snippet = kg.get("description").and_then(|v| v.as_str()).unwrap_or("");
        if !snippet.is_empty() {
            push_candidate(&mut candidates, title, url, snippet, false);
        }
    }

    // Fresh news coverage, only when the planner flagged the turn as news-y.
    if include_news {
        if let Some(items) = value.get("topStories").and_then(|v| v.as_array()) {
            for item in items {
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let url = item.get("link").and_then(|v| v.as_str()).unwrap_or("");
                let source = item.get("source").and_then(|v| v.as_str()).unwrap_or("");
                let date = item.get("date").and_then(|v| v.as_str()).unwrap_or("");
                // topStories carry no snippet; fold source + date in so there's
                // something for the rerank and the model to read.
                let snippet = [source, date]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" · ");
                push_candidate(&mut candidates, title, url, &snippet, true);
            }
        }
    }

    // Organic web results.
    if let Some(items) = value.get("organic").and_then(|v| v.as_array()) {
        for item in items {
            let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let url = item.get("link").and_then(|v| v.as_str()).unwrap_or("");
            let base = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
            // Some organic results carry a date; prepend it so recency is visible
            // to the rerank.
            let snippet = match item.get("date").and_then(|v| v.as_str()) {
                Some(date) if !date.is_empty() => format!("{} — {}", date, base),
                _ => base.to_string(),
            };
            push_candidate(&mut candidates, title, url, &snippet, false);
        }
    }

    Ok(candidates)
}

/// TinyFish Search (`GET https://api.search.tinyfish.ai`) — snippet-only search
/// over TinyFish's own index, built for agent/LLM consumption. Auth is an
/// `X-API-Key` header. Free tier needs no card (30 queries/minute), and TinyFish
/// documents sub-second latency, which puts it in the same class as Serper for a
/// voice turn.
///
/// Note this is TinyFish's *Search* endpoint, not their *Agent* endpoint — no
/// page fetching, no browsing, one round-trip, same as every other backend here.
///
/// Freshness maps to TinyFish's `recency_minutes`. The request always uses the
/// default `domain_type=web`: a TinyFish call returns *either* web or news
/// results, never both, and a second call for news would double the round-trip
/// this module exists to avoid. Instead, results carrying a `date`/`publisher`
/// are flagged `from_news`, so the local rerank still gets a recency signal out
/// of a single request.
async fn search_tinyfish(
    api_key: &str,
    query: &str,
    max_results: usize,
    tbs: Option<&str>,
) -> Result<Vec<Candidate>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "TinyFish API key is not set. Add it in Settings → Assistant → Web Search.".to_string(),
        );
    }

    let client = http_client(REQUEST_TIMEOUT)?;
    let mut query_params: Vec<(&str, String)> = vec![("query", query.to_string())];
    if let Some(minutes) = tinyfish_recency_minutes_from_tbs(tbs) {
        query_params.push(("recency_minutes", minutes.to_string()));
    }

    let resp = client
        .get("https://api.search.tinyfish.ai")
        .header("X-API-Key", api_key)
        .header(reqwest::header::ACCEPT, "application/json")
        .query(&query_params)
        .send()
        .await
        .map_err(|e| format!("TinyFish request failed: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "TinyFish search failed ({}): {}",
            status,
            truncate_chars(&text, 200)
        ));
    }

    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse TinyFish response: {}", e))?;

    let candidates = parse_tinyfish_results(&value, max_results);
    debug!(
        "TinyFish returned {} candidate(s) for {:?}",
        candidates.len(),
        query
    );
    Ok(candidates)
}

/// Parse a TinyFish `/search` payload into candidates. Split out from the HTTP
/// call so the response shape — `results[]` of `title` / `url` / `snippet`, with
/// `date` and `publisher` present on news-ish entries — is unit-testable.
fn parse_tinyfish_results(value: &serde_json::Value, max_results: usize) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    let Some(items) = value.get("results").and_then(|r| r.as_array()) else {
        return candidates;
    };
    for item in items {
        let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let snippet = item.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
        // `publisher`/`date` show up on news results and dated web results — a
        // free recency signal for the rerank, with no second request.
        let has_text = |key: &str| {
            item.get(key)
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty())
        };
        let from_news = has_text("publisher") || has_text("date");
        push_candidate(&mut candidates, title, url, snippet, from_news);
        if candidates.len() >= max_results {
            break;
        }
    }
    candidates
}

/// Extract a Brave `freshness` value from our `tbs` (which encodes the qdr window).
fn brave_freshness_from_tbs(tbs: Option<&str>) -> Option<&'static str> {
    let tbs = tbs?;
    if tbs.contains("qdr:d") {
        Some("pd")
    } else if tbs.contains("qdr:w") {
        Some("pw")
    } else if tbs.contains("qdr:m") {
        Some("pm")
    } else if tbs.contains("qdr:y") {
        Some("py")
    } else {
        None
    }
}

/// Map our `tbs` (which encodes the qdr window) to Tavily's `time_range`.
fn tavily_time_range_from_tbs(tbs: Option<&str>) -> Option<&'static str> {
    let tbs = tbs?;
    if tbs.contains("qdr:d") {
        Some("day")
    } else if tbs.contains("qdr:w") {
        Some("week")
    } else if tbs.contains("qdr:m") {
        Some("month")
    } else if tbs.contains("qdr:y") {
        Some("year")
    } else {
        None
    }
}

/// Map our `tbs` (qdr window) to TinyFish's `recency_minutes` freshness window.
/// TinyFish accepts 1..=5_256_000 (10 years), so every window below is in range.
fn tinyfish_recency_minutes_from_tbs(tbs: Option<&str>) -> Option<u32> {
    let tbs = tbs?;
    if tbs.contains("qdr:d") {
        Some(1_440) // 24 hours
    } else if tbs.contains("qdr:w") {
        Some(10_080) // 7 days
    } else if tbs.contains("qdr:m") {
        Some(43_200) // 30 days
    } else if tbs.contains("qdr:y") {
        Some(525_600) // 365 days
    } else {
        None
    }
}

/// Map our `tbs` (qdr window) to an Exa `startPublishedDate` lower bound (ISO
/// 8601), computed as "now minus the window".
fn exa_start_date_from_tbs(tbs: Option<&str>) -> Option<String> {
    let tbs = tbs?;
    let days = if tbs.contains("qdr:d") {
        1
    } else if tbs.contains("qdr:w") {
        7
    } else if tbs.contains("qdr:m") {
        30
    } else if tbs.contains("qdr:y") {
        365
    } else {
        return None;
    };
    let start = chrono::Utc::now() - chrono::Duration::days(days);
    Some(start.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
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
    unescaped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Clean scraped page content: trim, collapse excessive blank lines, cap length.
fn cap_content(input: &str, max: usize) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let collapsed = MULTI_NEWLINE_RE.replace_all(trimmed, "\n\n");
    truncate_chars(collapsed.trim(), max)
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

/// Push a cleaned, bounded candidate, skipping entries with neither title nor URL.
fn push_candidate(
    candidates: &mut Vec<Candidate>,
    title: &str,
    url: &str,
    snippet: &str,
    from_news: bool,
) {
    let title = truncate_chars(title.trim(), TITLE_MAX_CHARS);
    let url = url.trim().to_string();
    let snippet = truncate_chars(&clean_html_text(snippet), SNIPPET_MAX_CHARS);
    if title.is_empty() && url.is_empty() {
        return;
    }
    candidates.push(Candidate {
        title,
        url,
        snippet,
        from_news,
    });
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
    let content = cap_content(content, CONTENT_HARD_CAP);
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

/// Convert a snippet-only candidate into a result (no scraped content).
fn candidate_to_result(c: Candidate) -> SearchResult {
    SearchResult {
        title: truncate_chars(c.title.trim(), TITLE_MAX_CHARS),
        url: c.url,
        snippet: truncate_chars(c.snippet.trim(), SNIPPET_MAX_CHARS),
        content: String::new(),
    }
}

/// Convert a provider `SearchResult` (Brave) into a candidate.
fn result_to_candidate(r: SearchResult) -> Candidate {
    Candidate {
        title: r.title,
        url: r.url,
        snippet: r.snippet,
        from_news: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_tbs_is_a_window_never_a_sort_order() {
        // `sbd:1` (sort by date) must never appear: re-ranking Google by date
        // empties broad queries, which used to cost a second round trip.
        for freshness in ["day", "week", "month", "year", "none", ""] {
            let tbs = build_tbs(freshness).unwrap_or_default();
            assert!(
                !tbs.contains("sbd"),
                "{freshness:?} produced a sort order: {tbs}"
            );
        }
        assert_eq!(build_tbs("day"), Some("qdr:d".to_string()));
        assert_eq!(build_tbs("week"), Some("qdr:w".to_string()));
        assert_eq!(build_tbs("month"), Some("qdr:m".to_string()));
        assert_eq!(build_tbs("year"), Some("qdr:y".to_string()));
        assert_eq!(build_tbs("none"), None);
        assert_eq!(build_tbs("nonsense"), None);
    }

    #[test]
    fn provider_freshness_mappings() {
        assert_eq!(brave_freshness_from_tbs(Some("sbd:1,qdr:w")), Some("pw"));
        assert_eq!(brave_freshness_from_tbs(Some("qdr:y")), Some("py"));
        assert_eq!(brave_freshness_from_tbs(None), None);
        assert_eq!(tavily_time_range_from_tbs(Some("sbd:1,qdr:d")), Some("day"));
        assert_eq!(tavily_time_range_from_tbs(Some("qdr:m")), Some("month"));
        assert_eq!(tavily_time_range_from_tbs(None), None);
        // Exa maps the window to an ISO start date (or None when unbounded).
        assert!(exa_start_date_from_tbs(Some("qdr:w")).is_some());
        assert_eq!(exa_start_date_from_tbs(None), None);
        // TinyFish maps the window to a `recency_minutes` integer.
        assert_eq!(
            tinyfish_recency_minutes_from_tbs(Some("qdr:d")),
            Some(1_440)
        );
        assert_eq!(
            tinyfish_recency_minutes_from_tbs(Some("sbd:1,qdr:w")),
            Some(10_080)
        );
        assert_eq!(
            tinyfish_recency_minutes_from_tbs(Some("qdr:y")),
            Some(525_600)
        );
        assert_eq!(tinyfish_recency_minutes_from_tbs(Some("sbd:1")), None);
        assert_eq!(tinyfish_recency_minutes_from_tbs(None), None);
    }

    #[test]
    fn parse_tinyfish_reads_title_url_snippet() {
        let payload = json!({
            "query": "web automation tools",
            "results": [
                {
                    "position": 1,
                    "site_name": "tinyfish.ai",
                    "title": "TinyFish — AI Web Automation Platform",
                    "snippet": "Automate any website with natural language instructions...",
                    "url": "https://tinyfish.ai"
                }
            ],
            "total_results": 10,
            "page": 0
        });
        let candidates = parse_tinyfish_results(&payload, 10);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].title, "TinyFish — AI Web Automation Platform");
        assert_eq!(candidates[0].url, "https://tinyfish.ai");
        assert!(candidates[0].snippet.starts_with("Automate any website"));
        // No date/publisher => plain web result.
        assert!(!candidates[0].from_news);
    }

    #[test]
    fn parse_tinyfish_flags_dated_results_as_news() {
        let payload = json!({
            "results": [
                { "title": "Dated", "url": "https://a.com", "snippet": "s", "date": "2026-07-30" },
                { "title": "Published", "url": "https://b.com", "snippet": "s", "publisher": "Reuters" },
                { "title": "Blank date", "url": "https://c.com", "snippet": "s", "date": "  " },
                { "title": "Plain", "url": "https://d.com", "snippet": "s" }
            ]
        });
        let candidates = parse_tinyfish_results(&payload, 10);
        assert_eq!(candidates.len(), 4);
        assert!(candidates[0].from_news, "a `date` is a recency signal");
        assert!(candidates[1].from_news, "a `publisher` is a recency signal");
        assert!(!candidates[2].from_news, "whitespace-only date is not one");
        assert!(!candidates[3].from_news);
    }

    #[test]
    fn parse_tinyfish_respects_limit_and_bad_shapes() {
        let payload = json!({
            "results": [
                { "title": "one", "url": "https://1.com", "snippet": "s" },
                { "title": "two", "url": "https://2.com", "snippet": "s" },
                { "title": "three", "url": "https://3.com", "snippet": "s" }
            ]
        });
        assert_eq!(parse_tinyfish_results(&payload, 2).len(), 2);

        // Entries with neither title nor URL are dropped by push_candidate.
        let sparse = json!({ "results": [ { "snippet": "orphan" } ] });
        assert!(parse_tinyfish_results(&sparse, 10).is_empty());

        // A missing / wrongly-typed `results` key yields no candidates, not a panic.
        assert!(parse_tinyfish_results(&json!({ "error": "nope" }), 10).is_empty());
        assert!(parse_tinyfish_results(&json!({ "results": "nope" }), 10).is_empty());
    }

    #[test]
    fn rerank_prefers_query_overlap_and_recency() {
        let mut cands = vec![
            Candidate {
                title: "Unrelated cooking blog".to_string(),
                url: "https://a.com".to_string(),
                snippet: "recipes and food".to_string(),
                from_news: false,
            },
            Candidate {
                title: "World Cup final result".to_string(),
                url: "https://b.com".to_string(),
                snippet: "the world cup final ended".to_string(),
                from_news: true,
            },
        ];
        rerank("world cup final", &mut cands, true);
        assert_eq!(cands[0].url, "https://b.com"); // relevant + news wins
    }

    #[test]
    fn rerank_news_boost_only_when_recency_sensitive() {
        let news = Candidate {
            title: "x".to_string(),
            url: "n".to_string(),
            snippet: "y".to_string(),
            from_news: true,
        };
        let web = Candidate {
            title: "x".to_string(),
            url: "w".to_string(),
            snippet: "y".to_string(),
            from_news: false,
        };
        let terms = query_terms("z");
        assert!(candidate_score(&news, &terms, true) > candidate_score(&web, &terms, true));
    }

    #[test]
    fn format_results_use_content_within_budget() {
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
        let block = format_results_for_prompt(&results, 8_000);
        assert!(block.starts_with("[Web search results you retrieved"));
        assert!(block.contains("Source: Alpha"));
        assert!(block.contains("Full page content about Alpha."));
        assert!(block.contains("Source: Beta"));
        assert!(block.contains("snippet for beta"));
        assert!(!block.contains("[1]"));
        assert!(!block.contains("https://"));
    }

    #[test]
    fn format_results_respects_tiny_budget() {
        let results = vec![SearchResult {
            title: "T".to_string(),
            url: "u".to_string(),
            snippet: "s".to_string(),
            content: "abcdefghij".to_string(),
        }];
        let block = format_results_for_prompt(&results, 3);
        // Only 3 chars of the body should appear.
        assert!(block.contains("abc"));
        assert!(!block.contains("abcd"));
    }
}
