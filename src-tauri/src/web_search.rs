//! Lightweight web search for the assistant.
//!
//! Design goals (in priority order): **fast**, **few tokens**, **no setup**.
//! - Snippet-only: we never fetch/scrape result pages. One HTTP round-trip per
//!   search, and the model only ever sees short titles + snippets, so a turn
//!   stays cheap and quick — this assistant targets small, low-latency models.
//! - Bounded: results, per-snippet length, and request time are all capped so a
//!   slow or chatty provider can never stall or flood the prompt.
//! - Optional & free by default: DuckDuckGo needs no API key. Firecrawl and
//!   Brave are available for users who already have a key.
//!
//! Search runs *inline* before the single LLM call (no function-calling
//! round-trip), so it works with any OpenAI-compatible model regardless of
//! whether it supports tools.

use crate::settings::AppSettings;
use log::{debug, warn};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use specta::Type;
use std::time::Duration;

/// Hard ceiling on how long a single search may take. Kept short so an enabled
/// web search never makes the assistant feel sluggish; on timeout we simply
/// answer without web context.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);

/// Per-snippet character cap. Snippets are descriptions, not articles — this is
/// plenty to ground an answer while keeping the prompt small.
const SNIPPET_MAX_CHARS: usize = 220;

/// Title character cap (defensive against pathological titles).
const TITLE_MAX_CHARS: usize = 160;

/// A browser-like User-Agent. The DuckDuckGo HTML endpoint returns an empty
/// page to obviously-automated clients; a normal UA gets normal results. This
/// is the same approach the popular `duckduckgo-search` library uses.
const BROWSER_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";

/// A single web result, trimmed to the essentials the model needs.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Appended to the system prompt only on turns where web results were found.
/// Tells the model to ground its answer in the provided results and — crucially
/// for this panel — to write clean conversational prose. No citation brackets
/// (`[1]`), no raw URLs, no Markdown tables: the panel is small and replies are
/// often read aloud, so that formatting just becomes noise ("weird box values"
/// on screen, gibberish through TTS).
pub const WEB_SEARCH_SYSTEM_DIRECTIVE: &str = "Live web search results are included with the user's message. Use them to answer accurately and concisely in plain, natural prose, leading with the answer. Do NOT add citation markers, source numbers, brackets like [1], or raw URLs. Do NOT use Markdown tables — this is a small chat panel and replies may be read aloud. If the results don't contain the answer, say so plainly instead of guessing.";

/// Run a web search using the provider configured in settings.
///
/// Returns the (possibly empty) list of results, or an error string describing
/// why the search could not run (missing key, network failure, etc.). Callers
/// in the assistant pipeline treat any error as "no web context" and answer
/// normally — a failed search must never break a chat turn.
pub async fn search(settings: &AppSettings, query: &str) -> Result<Vec<SearchResult>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let max_results = (settings.assistant_web_search_max_results as usize).clamp(1, 8);
    let provider = settings.assistant_web_search_provider.as_str();

    debug!(
        "Web search via '{}' for {:?} (max {})",
        provider, query, max_results
    );

    match provider {
        "firecrawl" => {
            let key = settings
                .web_search_api_keys
                .get("firecrawl")
                .cloned()
                .unwrap_or_default();
            search_firecrawl(&key, query, max_results).await
        }
        "brave" => {
            let key = settings
                .web_search_api_keys
                .get("brave")
                .cloned()
                .unwrap_or_default();
            search_brave(&key, query, max_results).await
        }
        // "duckduckgo" and any unknown value fall back to the free engine.
        _ => search_duckduckgo(query, max_results).await,
    }
}

/// Format results as a compact context block to include with the user's
/// message. Plain dash bullets — deliberately *not* numbered and without URLs,
/// so the model has nothing to echo back as `[1]`-style citations and stays in
/// clean prose:
///
/// ```text
/// Web results for "query":
/// - Title — snippet
/// - Title — snippet
/// ```
pub fn format_results_for_prompt(query: &str, results: &[SearchResult]) -> String {
    let mut out = String::with_capacity(256);
    out.push_str("Web results for \"");
    out.push_str(query.trim());
    out.push_str("\":\n");
    for r in results {
        out.push_str("- ");
        out.push_str(&r.title);
        if !r.snippet.is_empty() {
            out.push_str(" — ");
            out.push_str(&r.snippet);
        }
        out.push('\n');
    }
    out
}

/// Fast, allocation-light heuristic deciding whether a query is worth a web
/// search. Runs locally (no LLM round-trip) so it adds no latency.
///
/// Philosophy: web search is opt-in, so once the user has enabled it we *search
/// by default* and skip only clear non-search work — trivial chit-chat,
/// questions about the assistant, text-generation/transform/coding tasks, pure
/// arithmetic, and conceptual "explain/define" requests. Everything else,
/// including bare factual phrases like "prime minister of canada", searches.
/// A false positive costs one quick request; a false negative gives a stale or
/// wrong answer — so we bias toward searching.
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

    // Strong "needs fresh/external info" signals → always search.
    const FRESHNESS: [&str; 27] = [
        "latest",
        "current",
        "currently",
        "today",
        "tonight",
        "right now",
        " now",
        "recent",
        "recently",
        "this week",
        "this month",
        "this year",
        "news",
        "weather",
        "forecast",
        "price",
        "stock",
        "score",
        "release",
        "released",
        "version",
        "update",
        "who won",
        "results",
        "2024",
        "2025",
        "2026",
    ];
    if FRESHNESS.iter().any(|m| q.contains(m)) {
        return true;
    }

    // Question-shaped input → likely a lookup. Covers "who/what/when/where/
    // which/whose", quantity questions, and yes/no factual openers, plus any
    // input that ends with a question mark.
    // Conceptual "teach me" requests are best answered from the model's own
    // knowledge rather than the live web. Kept deliberately narrow (only the
    // clearest openers) so genuine lookups still search.
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

    // The user explicitly turned web search on, so treat anything that survived
    // the filters above as worth a lookup — including bare factual phrases like
    // "prime minister of canada" or "tesla q3 earnings". (The previous rule
    // required a question word or "?", which silently skipped the short
    // noun-phrase queries people commonly type.)
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

/// Build a reqwest client with our timeout and a browser User-Agent.
fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(BROWSER_UA)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// Firecrawl `/v2/search`. Without `scrapeOptions` it returns only
/// `{title, description, url}` per result — exactly the snippet data we want,
/// at the lowest credit cost.
async fn search_firecrawl(
    api_key: &str,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Firecrawl API key is not set. Add it in Settings → Assistant → Web Search."
                .to_string(),
        );
    }

    let client = http_client()?;
    let body = serde_json::json!({
        "query": query,
        "limit": max_results,
        "sources": ["web"],
    });

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
            push_result(&mut results, title, url, snippet);
            if results.len() >= max_results {
                break;
            }
        }
    }
    Ok(results)
}

/// Brave Web Search API. JSON results live at `web.results[]`; descriptions can
/// contain `<strong>` highlight tags, which we strip.
async fn search_brave(
    api_key: &str,
    query: &str,
    max_results: usize,
) -> Result<Vec<SearchResult>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Brave Search API key is not set. Add it in Settings → Assistant → Web Search."
                .to_string(),
        );
    }

    let client = http_client()?;
    // Brave caps `count` at 20.
    let count = max_results.clamp(1, 20).to_string();

    let resp = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", api_key)
        .header(reqwest::header::ACCEPT, "application/json")
        .query(&[("q", query), ("count", count.as_str())])
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
            push_result(&mut results, title, url, &snippet);
            if results.len() >= max_results {
                break;
            }
        }
    }
    Ok(results)
}

/// DuckDuckGo via the keyless HTML endpoint. We POST the query and parse the
/// returned HTML for result links + snippets. Free, no account, no API key.
async fn search_duckduckgo(query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
    let client = http_client()?;

    let resp = client
        .post("https://html.duckduckgo.com/html/")
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .form(&[("q", query)])
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

        push_result(&mut results, title, url, &snippet);
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
fn push_result(results: &mut Vec<SearchResult>, title: &str, url: &str, snippet: &str) {
    let title = truncate_chars(title.trim(), TITLE_MAX_CHARS);
    let url = url.trim().to_string();
    let snippet = truncate_chars(snippet.trim(), SNIPPET_MAX_CHARS);
    if title.is_empty() && url.is_empty() {
        return;
    }
    results.push(SearchResult {
        title,
        url,
        snippet,
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
    fn should_search_triggers_on_questions_and_freshness() {
        assert!(should_search("who is the prime minister of canada"));
        assert!(should_search("what's the weather in Paris"));
        assert!(should_search("latest iphone price"));
        assert!(should_search("how many people live in Tokyo?"));
        assert!(should_search("Bitcoin price today"));
    }

    #[test]
    fn should_search_triggers_on_bare_factual_phrases() {
        // The key fix: short noun-phrase lookups (common when typing) must
        // search even without a question word or freshness keyword.
        assert!(should_search("prime minister of canada"));
        assert!(should_search("tesla q3 earnings"));
        assert!(should_search("population of japan"));
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
    fn format_results_are_plain_bullets_without_numbers_or_urls() {
        let results = vec![
            SearchResult {
                title: "A".to_string(),
                url: "https://a.com".to_string(),
                snippet: "snip a".to_string(),
            },
            SearchResult {
                title: "B".to_string(),
                url: "https://b.com".to_string(),
                snippet: String::new(),
            },
        ];
        let block = format_results_for_prompt("q", &results);
        assert!(block.starts_with("Web results for \"q\":"));
        assert!(block.contains("- A — snip a"));
        assert!(block.contains("- B"));
        // No citation numbers and no URLs to tempt the model into echoing them.
        assert!(!block.contains("[1]"));
        assert!(!block.contains("https://"));
    }
}
