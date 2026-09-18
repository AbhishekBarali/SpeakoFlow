//! Finding the parts of a transcript that answer a question.
//!
//! # Why retrieval and not map-reduce
//!
//! [`super::summarize`] already fits a long transcript into a small context by
//! map-reduce, and that is the right shape for generating notes: one job, run once
//! when the call ends, producing a document. It is the wrong shape for a question.
//!
//! Two reasons, and the second matters more. Cost: a two-hour meeting is nine
//! sequential model calls, so every question would take tens of seconds on a cloud
//! provider and minutes on a local model. And *quality*: the map step's whole
//! purpose is to throw away detail, so by the time the reduce step is answering
//! "what did Priya say the number was", the sentence containing the number has
//! already been condensed into "discussed pricing".
//!
//! Retrieval does the opposite — it finds the handful of utterances that mention
//! the thing being asked about and sends those verbatim.
//!
//! # The short-transcript shortcut
//!
//! Retrieval is skipped entirely when the whole transcript fits the budget.
//! `summarize::generate_meeting_notes` makes the same call for the same reason: a
//! search over material the model could simply read is an extra failure mode and a
//! worse answer. Most meetings are short enough that this is the common path.
//!
//! # Everything here is pure except one function
//!
//! [`fts_query`] and [`hit_ids`] are pure and unit-tested, because the FTS5 query
//! is the piece that can turn a legitimate question into a **SQL error** rather
//! than an empty result — and that failure only appears for particular user input
//! ("what about C++?", "did we say \"yes\"?"), which is exactly the kind of thing
//! that ships broken.

use anyhow::Result;

use super::session::label_segments;
use super::store::MeetingStore;
use super::MeetingSegment;

/// Characters of transcript that go to the model in one question.
///
/// Larger than [`super::summarize::WINDOW_CHAR_BUDGET`] because that budget has to
/// leave room for a template, the user's notes and a document-length answer, while
/// this leaves room for a short answer and a few turns of chat history. Still a
/// character count rather than a token count, for the reason `summarize` documents:
/// the tokenizer is a property of the model, and this code does not know which
/// model it is talking to.
pub const CONTEXT_CHAR_BUDGET: usize = 24_000;

/// Segments fetched per FTS query before context expansion.
///
/// Twelve hits, each widened by [`CONTEXT_SEGMENTS`] on both sides, is up to ~60
/// utterances — comfortably inside the budget for a normal meeting and enough that
/// a topic discussed in three separate places is all found.
const MAX_HITS: u32 = 12;

/// Utterances of context kept on each side of a hit.
///
/// A matched line is often unanswerable alone: "yeah, let's do that" matches a
/// question about a decision and contains none of it. Two on each side is enough to
/// recover who was being answered.
const CONTEXT_SEGMENTS: u32 = 2;

/// Words too common to narrow anything, dropped from the query.
///
/// Not a general stopword list, and deliberately small: over-filtering turns "what
/// did we decide about the API" into a search for "decide API" — fine — but would
/// turn "who is on point" into an empty query. Anything that empties the query
/// falls back to whole-transcript, so the cost of keeping this short is a slightly
/// noisier search rather than a failure.
const STOPWORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "am", "an", "and", "any", "are", "as", "at", "be",
    "been", "but", "by", "can", "did", "do", "does", "for", "from", "get", "had", "has", "have",
    "he", "her", "him", "his", "how", "i", "if", "in", "into", "is", "it", "its", "just", "me",
    "my", "no", "not", "of", "on", "or", "our", "out", "över", "say", "said", "she", "so", "than",
    "that", "the", "their", "them", "then", "there", "these", "they", "this", "to", "up", "us",
    "was", "we", "were", "what", "when", "where", "which", "who", "why", "will", "with", "would",
    "you", "your",
];

/// Shortest term worth searching for.
///
/// One- and two-character tokens are almost always noise ("a", "ok", "hm") and
/// match so much of a transcript that they dilute the bm25 ranking of the terms
/// that matter. Digits are exempt, because a year or a figure is frequently the
/// entire question.
const MIN_TERM_LEN: usize = 3;

/// What was assembled for the model, and how.
pub struct RetrievedContext {
    /// Speaker-prefixed lines, in meeting order.
    pub lines: Vec<String>,
    /// True when the whole transcript was sent rather than a retrieved subset.
    ///
    /// Surfaced because it changes what the model should be told: given the whole
    /// transcript it can say "that was never discussed", but given excerpts it
    /// must not — the absence of something in a retrieved subset is not evidence
    /// of its absence from the meeting.
    pub complete: bool,
    /// Segments the transcript actually has, for the "no transcript yet" case.
    pub total_segments: usize,
}

impl RetrievedContext {
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// Turn a natural-language question into a valid FTS5 MATCH expression.
///
/// Three jobs, in order of how badly getting them wrong hurts:
///
/// 1. **Never emit invalid syntax.** FTS5 raises an error — not an empty result —
///    for bare punctuation, an unbalanced quote, or a trailing `OR`. A question
///    containing `C++`, `"`, or `-` would otherwise fail the whole request.
///    Every term is therefore wrapped in double quotes and any internal quote is
///    doubled, which makes it a string literal FTS5 cannot misread as an operator.
/// 2. **Match any term, not all of them.** `OR` rather than the implicit `AND`,
///    because a question rarely uses the transcript's own wording and requiring
///    every word would return nothing. bm25 ranking is what sorts the results, so
///    breadth costs precision only at the bottom of the list.
/// 3. **Prefix-match.** `"budget"*` also finds "budgets" and "budgeting". This
///    replaces a stemming tokenizer, which would have to be chosen per language
///    and would mangle code-switched text.
///
/// Returns `None` when nothing usable is left, which the caller must treat as "do
/// not search" rather than "search for nothing".
pub fn fts_query(question: &str) -> Option<String> {
    let terms: Vec<String> = question
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .filter(|token| is_useful_term(token))
        // A question that repeats a word should not weight it twice.
        .fold(Vec::new(), |mut acc, token| {
            if !acc.contains(&token) {
                acc.push(token);
            }
            acc
        });

    if terms.is_empty() {
        return None;
    }

    Some(
        terms
            .iter()
            // The tokens are already alphanumeric-only, so the quote-doubling is
            // belt and braces — but it is the thing that makes this function
            // safe by construction rather than safe by the filter above.
            .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

fn is_useful_term(token: &str) -> bool {
    // A number is often the whole question ("what did we say about 2026"), so it
    // is kept regardless of length.
    if token.chars().all(|c| c.is_numeric()) {
        return true;
    }
    token.chars().count() >= MIN_TERM_LEN && !STOPWORDS.contains(&token)
}

/// Row ids from a search result.
pub fn hit_ids(segments: &[MeetingSegment]) -> Vec<i64> {
    segments.iter().map(|segment| segment.id).collect()
}

/// Trim labelled lines to a character budget, keeping the **end**.
///
/// The end rather than the beginning, for two different reasons depending on the
/// caller. For a whole transcript sent to answer a question about a live meeting,
/// the recent conversation is what is being asked about. For a retrieved set, the
/// lines are already bm25-selected, so what is dropped is the lowest-ranked
/// context rather than the answer.
fn clip_to_budget(lines: Vec<String>, budget: usize) -> Vec<String> {
    let mut total = 0usize;
    let mut kept: Vec<String> = Vec::new();
    for line in lines.into_iter().rev() {
        let cost = line.chars().count() + 1;
        if total + cost > budget && !kept.is_empty() {
            break;
        }
        total += cost;
        kept.push(line);
    }
    kept.reverse();
    kept
}

/// Assemble the transcript context for one question.
///
/// Sends the whole transcript when it fits, and otherwise searches. Blocking: this
/// reads SQLite, so callers on the async runtime must go through
/// `spawn_blocking`.
pub fn context_for_question(
    store: &MeetingStore,
    meeting_id: i64,
    question: &str,
) -> Result<RetrievedContext> {
    let speakers = store.speakers(meeting_id)?;
    let all = store.all_segments(meeting_id)?;
    let total_segments = all.len();

    let full_lines = super::summarize::transcript_lines(&label_segments(&all, &speakers));
    let full_size: usize = full_lines.iter().map(|line| line.chars().count() + 1).sum();

    // The whole thing fits, so there is nothing retrieval could add and one less
    // thing that can go wrong.
    if full_size <= CONTEXT_CHAR_BUDGET {
        return Ok(RetrievedContext {
            lines: full_lines,
            complete: true,
            total_segments,
        });
    }

    // Too long to send whole. Search — and fall back to the most recent part of
    // the meeting when the question has no searchable terms in it, because "what
    // did I just miss" is a real question with no keywords.
    let retrieved = match fts_query(question) {
        Some(match_query) => {
            let hits = store.search_segments(meeting_id, &match_query, MAX_HITS)?;
            if hits.is_empty() {
                Vec::new()
            } else {
                store.segments_around(meeting_id, &hit_ids(&hits), CONTEXT_SEGMENTS)?
            }
        }
        None => Vec::new(),
    };

    if retrieved.is_empty() {
        return Ok(RetrievedContext {
            lines: clip_to_budget(full_lines, CONTEXT_CHAR_BUDGET),
            complete: false,
            total_segments,
        });
    }

    let lines = super::summarize::transcript_lines(&label_segments(&retrieved, &speakers));
    Ok(RetrievedContext {
        lines: clip_to_budget(lines, CONTEXT_CHAR_BUDGET),
        complete: false,
        total_segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /* ───────────────────────── the FTS5 query ───────────────────────── */

    #[test]
    fn an_ordinary_question_becomes_an_or_of_prefix_terms() {
        let query = fts_query("What did we decide about the pricing?").unwrap();
        assert!(query.contains("\"decide\"*"));
        assert!(query.contains("\"pricing\"*"));
        assert!(query.contains(" OR "));
        // Stopwords carry no signal and dilute the ranking.
        assert!(!query.contains("\"what\""));
        assert!(!query.contains("\"the\""));
    }

    /// The bug this whole function exists to prevent: FTS5 raises a SQL error for
    /// an unbalanced quote, so a question containing one must not reach MATCH
    /// unescaped.
    #[test]
    fn a_quote_in_the_question_cannot_break_the_query() {
        let query = fts_query("did he say \"yes\" or maybe").unwrap();
        // Quotes only ever appear as the delimiters we added, in balanced pairs.
        assert_eq!(query.matches('"').count() % 2, 0);
        assert!(query.contains("\"maybe\"*"));
    }

    /// Operators and punctuation are FTS5 syntax. Stripped to tokens, they cannot
    /// be misread as an expression.
    #[test]
    fn operators_and_punctuation_are_neutralised() {
        let query = fts_query("C++ AND -NOT (deployment) OR*").unwrap();
        assert!(query.contains("\"deployment\"*"));
        // "AND"/"NOT"/"OR" survive only as quoted literals, never as bare
        // operators — a bare trailing operator is a syntax error.
        assert!(!query.contains("* AND"));
        assert!(!query.trim_end().ends_with("OR"));
    }

    /// A question of nothing but stopwords or punctuation has no usable query, and
    /// the caller must know that rather than search for an empty string.
    #[test]
    fn a_question_with_no_usable_terms_yields_none() {
        assert!(fts_query("what is it?").is_none());
        assert!(fts_query("???").is_none());
        assert!(fts_query("   ").is_none());
        assert!(fts_query("").is_none());
    }

    /// A figure or a year is frequently the entire question, so the length floor
    /// must not eat it.
    #[test]
    fn numbers_survive_the_length_floor() {
        let query = fts_query("was it 42 or 2026").unwrap();
        assert!(query.contains("\"42\"*"));
        assert!(query.contains("\"2026\"*"));
    }

    #[test]
    fn a_repeated_word_is_searched_once() {
        let query = fts_query("pricing pricing PRICING").unwrap();
        assert_eq!(query.matches("\"pricing\"*").count(), 1);
    }

    /// Devanagari is past the ASCII fast paths and must still produce terms — this
    /// app's meetings are routinely code-switched.
    #[test]
    fn non_latin_scripts_produce_terms() {
        let query = fts_query("मीटिंग के नोट्स").unwrap();
        assert!(query.contains("मीटिंग"));
        assert_eq!(query.matches('"').count() % 2, 0);
    }

    /* ───────────────────────────── budgeting ───────────────────────────── */

    #[test]
    fn lines_within_budget_are_untouched() {
        let lines = vec!["Me: hello".to_string(), "Them: hi".to_string()];
        assert_eq!(clip_to_budget(lines.clone(), 1_000), lines);
    }

    /// Trimming keeps the end, which is the recent conversation.
    #[test]
    fn clipping_keeps_the_most_recent_lines() {
        let lines: Vec<String> = (0..100).map(|i| format!("Me: line {i}")).collect();
        let kept = clip_to_budget(lines, 60);
        assert!(kept.len() < 100);
        assert_eq!(kept.last().unwrap(), "Me: line 99");
    }

    /// A single line over the whole budget must still be sent — an empty context
    /// is a guaranteed non-answer, while an oversized one usually still works.
    #[test]
    fn one_oversized_line_is_still_kept() {
        let lines = vec!["Me: ".to_string() + &"word ".repeat(500)];
        assert_eq!(clip_to_budget(lines, 10).len(), 1);
    }

    #[test]
    fn clipping_nothing_yields_nothing() {
        assert!(clip_to_budget(Vec::new(), 100).is_empty());
    }

    /* ───────────────────────────── hit ids ───────────────────────────── */

    #[test]
    fn hit_ids_preserve_order() {
        let segments: Vec<MeetingSegment> = [7i64, 2, 9]
            .iter()
            .map(|id| MeetingSegment {
                id: *id,
                meeting_id: 1,
                source: super::super::SpeakerSource::System,
                speaker_key: None,
                start_ms: 0,
                end_ms: 1,
                text: "x".into(),
                confidence: None,
            })
            .collect();
        assert_eq!(hit_ids(&segments), vec![7, 2, 9]);
    }

    #[test]
    fn no_hits_means_no_ids() {
        assert!(hit_ids(&[]).is_empty());
    }
}
