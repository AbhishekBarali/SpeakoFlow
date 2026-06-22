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
        // verbatim. The optional surrounding `\s*` (for trim) is part of the
        // match and is therefore removed from the output.
        let core = if rule.is_regex {
            rule.search.clone()
        } else {
            regex::escape(&rule.search)
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

    #[test]
    fn literal_replacement() {
        let rules = vec![rule("teh", "the")];
        assert_eq!(apply_replacements("teh cat", &rules), "the cat");
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
