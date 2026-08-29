//! Deterministic, rule-based text replacements.
//!
//! A fast, offline, deterministic find/replace pass over a transcript. It
//! complements (does not duplicate) the optional LLM post-processing: rules are
//! literal or regex substitutions plus a small set of extensible "magic
//! commands" that expand inside the replacement text.
//!
//! Supported magic commands (used inside a rule's `replace` field):
//! - `[date]`       → current local date, `YYYY-MM-DD`
//! - `[time]`       → current local time, `HH:MM`
//! - `[uppercase]`  → UPPERCASE the rule output (alias: `[upper]`)
//! - `[lowercase]`  → lowercase the rule output (alias: `[lower]`)
//! - `[capitalize]` → Capitalize the first character of the output
//! - `[nospace]`    → remove all whitespace from the output

use crate::settings::{Capitalization, Replacement};
use chrono::Local;
use log::warn;
use regex::Regex;

/// Escape a literal search term so it matches verbatim, except that its **first
/// character may match in either case**, and anchor it to word boundaries.
///
/// **Case.** Only the first character, and only for literal rules. A dictated
/// phrase gets capitalized purely by position — at the start of an utterance,
/// after a sentence boundary, or by an AI-cleanup pass that punctuates the text
/// — and a fully case-sensitive match then silently stops firing. Measured on
/// the real cleanup model, exactly this made the difference between a rule
/// landing and vanishing on 4 of 11 dictations.
///
/// Full case-insensitivity is deliberately NOT used: it would make an all-caps
/// acronym rule ("US" → "United States") start rewriting the ordinary word
/// "us". Leading-character-only keeps the mechanical case change working
/// without turning every acronym into a footgun.
///
/// **Boundaries.** A literal rule is a *spoken phrase*, so it must match whole
/// words. Unanchored, "US" → "United States" turned "USA today" into
/// "United StatesA today", and "clod" → "claude" turned "unclod" into
/// "unclaude". The anchors are added only on a side that actually begins/ends
/// with a word character, so a rule for punctuation or an emoticon still works.
fn literal_pattern(search: &str) -> String {
    let mut chars = search.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let rest = regex::escape(chars.as_str());
    let lower: String = first.to_lowercase().collect();
    let upper: String = first.to_uppercase().collect();

    let body = if lower == upper {
        format!("{}{}", regex::escape(&lower), rest)
    } else {
        // Alternation rather than a character class, so a character whose case
        // mapping is multi-character (ß → SS) still works.
        format!(
            "(?:{}|{}){}",
            regex::escape(&lower),
            regex::escape(&upper),
            rest
        )
    };

    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let open = if is_word(first) { r"\b" } else { "" };
    let close = match search.chars().last() {
        Some(last) if is_word(last) => r"\b",
        _ => "",
    };
    format!("{open}{body}{close}")
}

/// Applies an ordered list of [`Replacement`] rules to `text`.
///
/// Rules are applied in order, so later rules see the output of earlier ones.
/// Disabled rules and rules with an empty `search` are skipped. Invalid regex
/// patterns are skipped (with a warning) rather than aborting the whole pass.
pub fn apply_replacements(text: &str, replacements: &[Replacement]) -> String {
    let mut result = text.to_string();

    for rule in replacements {
        if !rule.enabled || rule.search.is_empty() {
            continue;
        }

        // Literal searches are regex-escaped so special characters match
        // verbatim, with a leading-character case allowance (see
        // `literal_pattern`). The optional surrounding `\s*` (for trim) is part
        // of the match and is therefore removed from the output.
        let core = if rule.is_regex {
            rule.search.clone()
        } else {
            literal_pattern(&rule.search)
        };
        let prefix = if rule.trim_before { r"\s*" } else { "" };
        let suffix = if rule.trim_after { r"\s*" } else { "" };
        let pattern = format!("{}(?:{}){}", prefix, core, suffix);

        let re = match Regex::new(&pattern) {
            Ok(re) => re,
            Err(e) => {
                warn!(
                    "Skipping invalid replacement rule (search={:?}): {}",
                    rule.search, e
                );
                continue;
            }
        };

        // The expanded replacement is identical for every match within a rule,
        // so compute it once. `NoExpand` makes the regex engine treat `$` in the
        // replacement literally instead of as a capture-group reference.
        let replacement = expand_replacement(&rule.replace, rule.capitalization);
        result = re
            .replace_all(&result, regex::NoExpand(replacement.as_str()))
            .into_owned();
    }

    result
}

/// Expands the magic commands inside a replacement template.
///
/// Transform tokens are detected and stripped first, then value-producing
/// tokens (`[date]`, `[time]`) are expanded, then the recorded transforms are
/// applied, and finally the rule's `capitalization` option is applied on top.
fn expand_replacement(template: &str, capitalization: Capitalization) -> String {
    let mut working = template.to_string();

    // 1. Detect + strip transform tokens (they may appear anywhere; they act on
    //    the whole rule output regardless of position).
    let uppercase = working.contains("[uppercase]") || working.contains("[upper]");
    working = working.replace("[uppercase]", "").replace("[upper]", "");
    let lowercase = working.contains("[lowercase]") || working.contains("[lower]");
    working = working.replace("[lowercase]", "").replace("[lower]", "");
    let capitalize = working.contains("[capitalize]");
    working = working.replace("[capitalize]", "");
    let nospace = working.contains("[nospace]");
    working = working.replace("[nospace]", "");

    // 2. Expand value-producing tokens.
    if working.contains("[date]") || working.contains("[time]") {
        let now = Local::now();
        working = working.replace("[date]", &now.format("%Y-%m-%d").to_string());
        working = working.replace("[time]", &now.format("%H:%M").to_string());
    }

    // 3. Apply the recorded transforms.
    if lowercase {
        working = working.to_lowercase();
    }
    if uppercase {
        working = working.to_uppercase();
    }
    if capitalize {
        working = capitalize_first(&working);
    }
    if nospace {
        working = working.chars().filter(|c| !c.is_whitespace()).collect();
    }

    // 4. Apply the per-rule capitalization option last.
    match capitalization {
        Capitalization::None => working,
        Capitalization::Uppercase => working.to_uppercase(),
        Capitalization::Lowercase => working.to_lowercase(),
        Capitalization::Capitalize => capitalize_first(&working),
    }
}

/// Capitalizes the first character of a string, leaving the rest untouched.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::apply_replacements;
    use crate::settings::{Capitalization, Replacement};
    use regex::Regex;

    /// Builds a simple literal, enabled rule.
    fn rule(search: &str, replace: &str) -> Replacement {
        Replacement {
            search: search.to_string(),
            replace: replace.to_string(),
            is_regex: false,
            enabled: true,
            trim_before: false,
            trim_after: false,
            capitalization: Capitalization::None,
        }
    }

    // A literal rule is a spoken *phrase*, so it must not fire inside a longer
    // word. Unanchored, "US" -> "United States" turned "USA today" into
    // "United StatesA today".
    #[test]
    fn literal_rule_matches_whole_words_only() {
        let acronym = vec![rule("US", "United States")];
        assert_eq!(
            apply_replacements("USA today", &acronym),
            "USA today",
            "a rule must not fire inside a longer word"
        );
        assert_eq!(
            apply_replacements("the US economy", &acronym),
            "the United States economy"
        );

        let short = vec![rule("clod", "claude")];
        assert_eq!(apply_replacements("unclod it", &short), "unclod it");
        assert_eq!(
            apply_replacements("clods everywhere", &short),
            "clods everywhere"
        );
        assert_eq!(apply_replacements("ask clod now", &short), "ask claude now");
        // Punctuation next to the phrase is still a boundary.
        assert_eq!(
            apply_replacements("ask clod, then wait", &short),
            "ask claude, then wait"
        );
    }

    // A rule whose search is punctuation has no word boundary to anchor to, so
    // anchors must be omitted rather than making it unmatchable.
    #[test]
    fn punctuation_rules_are_not_boundary_anchored() {
        let rules = vec![rule(":)", "🙂")];
        assert_eq!(apply_replacements("nice :) ok", &rules), "nice 🙂 ok");
        assert_eq!(apply_replacements("nice:)ok", &rules), "nice🙂ok");
    }

    #[test]
    fn literal_replacement() {
        let rules = vec![rule("teh", "the")];
        assert_eq!(apply_replacements("teh cat", &rules), "the cat");
    }

    // Measured against the real cleanup model: a rule trigger at the start of an
    // utterance, or after a sentence boundary, comes back capitalized — from the
    // speaker, the ASR, or the AI-cleanup pass that punctuates the text. A fully
    // case-sensitive literal match silently stopped firing there, so the user's
    // substitution just vanished with no error anywhere.
    #[test]
    fn literal_rule_tolerates_a_capitalized_first_character() {
        let rules = vec![rule("my name", "Alex Rivera")];
        assert_eq!(
            apply_replacements("my name is on the contract", &rules),
            "Alex Rivera is on the contract"
        );
        assert_eq!(
            apply_replacements("My name is on the contract", &rules),
            "Alex Rivera is on the contract",
            "positional capitalization must not break the rule"
        );
        assert_eq!(
            apply_replacements("The contract is signed. My name is on page four.", &rules),
            "The contract is signed. Alex Rivera is on page four.",
            "capitalization after a sentence boundary must not break the rule"
        );
    }

    // The reason this is leading-character-only rather than a blanket `(?i)`:
    // an acronym rule must not start rewriting an ordinary lowercase word.
    #[test]
    fn only_the_first_character_is_case_flexible() {
        let acronym = vec![rule("US", "United States")];
        assert_eq!(
            apply_replacements("between us and them", &acronym),
            "between us and them",
            "a lowercase word must not match an all-caps acronym rule"
        );
        assert_eq!(
            apply_replacements("US exports rose", &acronym),
            "United States exports rose",
            "the rule still matches its own spelling"
        );

        // Interior case still has to match exactly.
        let rules = vec![rule("my Name", "X")];
        assert_eq!(apply_replacements("my name here", &rules), "my name here");
    }

    #[test]
    fn case_flexibility_handles_non_alphabetic_and_wide_case_mappings() {
        // A leading character with no case (digit/punctuation) must still match.
        assert_eq!(
            apply_replacements("call 4030 now", &[rule("4030", "5150")]),
            "call 5150 now"
        );
        // ß uppercases to the two-character "SS", which would break a character
        // class; the pattern uses alternation for exactly this reason.
        let rules = vec![rule("ßeta", "beta")];
        assert_eq!(
            apply_replacements("the ßeta build", &rules),
            "the beta build"
        );
        assert_eq!(
            apply_replacements("the SSeta build", &rules),
            "the beta build"
        );
    }

    #[test]
    fn literal_is_not_treated_as_regex() {
        // Parentheses are regex metacharacters but must match literally here.
        let rules = vec![rule("(c)", "©")];
        assert_eq!(apply_replacements("foo (c) bar", &rules), "foo © bar");
    }

    #[test]
    fn replacement_with_dollar_is_literal() {
        // `$1` must be inserted verbatim, not treated as a capture reference.
        let rules = vec![rule("price", "$1")];
        assert_eq!(apply_replacements("the price", &rules), "the $1");
    }

    #[test]
    fn disabled_rule_is_skipped() {
        let mut r = rule("teh", "the");
        r.enabled = false;
        assert_eq!(apply_replacements("teh cat", &[r]), "teh cat");
    }

    #[test]
    fn empty_search_is_skipped() {
        let rules = vec![rule("", "x")];
        assert_eq!(apply_replacements("hello", &rules), "hello");
    }

    #[test]
    fn regex_replacement() {
        let mut r = rule(r"\bcolour\b", "color");
        r.is_regex = true;
        assert_eq!(
            apply_replacements("my favourite colour", &[r]),
            "my favourite color"
        );
    }

    #[test]
    fn invalid_regex_is_skipped_not_panicking() {
        let mut r = rule("(unclosed", "x");
        r.is_regex = true;
        assert_eq!(
            apply_replacements("(unclosed group", &[r]),
            "(unclosed group"
        );
    }

    #[test]
    fn rules_apply_in_order() {
        let rules = vec![rule("a", "b"), rule("b", "c")];
        // "a" -> "b" -> "c"
        assert_eq!(apply_replacements("a", &rules), "c");
    }

    #[test]
    fn trim_before_removes_preceding_whitespace() {
        let mut r = rule(",", ",");
        r.trim_before = true;
        assert_eq!(apply_replacements("hello , world", &[r]), "hello, world");
    }

    #[test]
    fn trim_after_removes_following_whitespace() {
        let mut r = rule("(", "(");
        r.trim_after = true;
        assert_eq!(apply_replacements("foo (  bar)", &[r]), "foo (bar)");
    }

    #[test]
    fn magic_uppercase_transform() {
        let rules = vec![rule("acme", "[uppercase]acme")];
        assert_eq!(apply_replacements("acme corp", &rules), "ACME corp");
    }

    #[test]
    fn magic_lowercase_transform() {
        let rules = vec![rule("SHOUT", "[lowercase]quiet")];
        assert_eq!(apply_replacements("SHOUT now", &rules), "quiet now");
    }

    #[test]
    fn magic_capitalize_transform() {
        let rules = vec![rule("name", "[capitalize]john")];
        assert_eq!(apply_replacements("name here", &rules), "John here");
    }

    #[test]
    fn magic_nospace_transform() {
        let rules = vec![rule("brand", "[nospace]my brand")];
        assert_eq!(apply_replacements("brand", &rules), "mybrand");
    }

    #[test]
    fn capitalization_field_applies() {
        let mut r = rule("ceo", "ceo");
        r.capitalization = Capitalization::Uppercase;
        assert_eq!(apply_replacements("the ceo spoke", &[r]), "the CEO spoke");
    }

    #[test]
    fn date_token_is_expanded() {
        let rules = vec![rule("today", "[date]")];
        let out = apply_replacements("today", &rules);
        assert!(!out.contains("[date]"), "token should be expanded: {out}");
        let re = Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
        assert!(re.is_match(&out), "unexpected date format: {out}");
    }

    #[test]
    fn time_token_is_expanded() {
        let rules = vec![rule("now", "[time]")];
        let out = apply_replacements("now", &rules);
        assert!(!out.contains("[time]"));
        let re = Regex::new(r"^\d{2}:\d{2}$").unwrap();
        assert!(re.is_match(&out), "unexpected time format: {out}");
    }
}
