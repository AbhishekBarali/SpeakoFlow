//! Deciding what a user's edit taught us.
//!
//! When someone dictates "email brali about it" and then fixes `brali` to `Barali`,
//! they have just supplied the single highest-quality piece of vocabulary the app can
//! get: a word the recogniser got wrong, and the exact spelling it should have used.
//! It cost them nothing extra, and they did it anyway because they wanted the text
//! right. Learning it means the next dictation gets it right on its own.
//!
//! This module is the decision half of that, and it is pure — pasted text and edited
//! text in, candidate words out. No accessibility APIs, no clock, no settings. The
//! reading half is [`super::watch`].
//!
//! # Why the guards are the feature
//!
//! Detecting *a* difference between two strings is trivial. Detecting a **correction**
//! is not, and the difference matters because every false positive is a permanent
//! entry in the user's dictionary that then corrupts future transcriptions — the
//! fuzzy custom-word pass will start bending correct words toward a garbage one.
//!
//! So the bar is deliberately high, and most edits are rejected:
//!
//! * A rewrite is not a correction. If someone replaces "cat" with "elephant" they
//!   changed their mind; if they replace "brali" with "Barali" they fixed a
//!   mishearing. [`MIN_SIMILARITY`] is what separates those, and it is the single
//!   most important number here.
//! * Only substitutions of **one word for one word**. An insertion, a deletion, or a
//!   two-words-for-one change is editing, not correcting.
//! * The original must have come from *our* transcript. Text the user typed
//!   themselves teaches nothing about what the recogniser mishears.
//! * A capitalisation-only change **is** accepted, and deliberately: `api` -> `API`
//!   and `priya` -> `Priya` are the most common corrections of all, and they are
//!   exactly what a dictionary fixes.
//! * A heavily edited paragraph teaches nothing. Past [`MAX_EDIT_RATIO`] the user
//!   rewrote the text rather than corrected it, and the whole comparison is discarded.

use std::collections::HashSet;

/// How similar a word and its replacement must be to count as a correction.
///
/// Normalised Levenshtein similarity in `[0, 1]`. This is the line between "the
/// recogniser misheard me" and "I changed my mind", and it is the number to reach for
/// if this feature ever learns something stupid.
///
/// 0.55 rather than something stricter, because real mishearings are often further
/// from the truth than they look: "Barali" -> "brali" is 0.67, but "Abhishek" ->
/// "a bee shake" is far worse, and speech-to-text splits and merges words freely.
/// Since every other guard here is already conservative, this one is allowed to be
/// the loosest of them.
pub const MIN_SIMILARITY: f64 = 0.55;

/// Shortest word worth learning.
///
/// Below four characters the similarity metric is meaningless — any two three-letter
/// words are "similar" — and short words are where a mistaken entry does the most
/// damage, because the fuzzy pass will match them everywhere.
pub const MIN_WORD_LEN: usize = 4;

/// Longest word worth learning. Past this it is not a word.
pub const MAX_WORD_LEN: usize = 40;

/// Most corrections taken from one dictation.
///
/// Someone who changed six words was editing. Learning all six would fill the
/// dictionary with half a paragraph.
pub const MAX_PER_DICTATION: usize = 3;

/// Above this fraction of words changed, the edit is a rewrite and teaches nothing.
pub const MAX_EDIT_RATIO: f64 = 0.4;

/// One word the user corrected, and what they corrected it to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Correction {
    /// What the recogniser produced.
    pub from: String,
    /// What the user replaced it with. This is what gets learned.
    pub to: String,
}

/// Find the corrections between what was pasted and what is there now.
///
/// `pasted` is the app's own output; `edited` is the field's current contents, which
/// may contain text from before and after the paste. The comparison is scoped to the
/// pasted region — see [`locate_region`] — so surrounding text the user wrote
/// themselves is neither compared nor learned from.
///
/// Returns at most [`MAX_PER_DICTATION`] corrections, in the order they appear.
pub fn detect(pasted: &str, edited: &str, already_known: &HashSet<String>) -> Vec<Correction> {
    let original = tokenize(pasted);
    let edited_tokens = tokenize(edited);

    let Some((start, end)) = locate_region(&original, &edited_tokens) else {
        // The pasted text is gone, or unrecognisable: the user deleted it, or this is
        // a different field than the one written to. Either way there is nothing to
        // compare and nothing to learn.
        return Vec::new();
    };
    let current = &edited_tokens[start..end];

    let ops = align(&original, current);

    // A rewrite teaches nothing, and the ratio is what tells a fix from a rewrite.
    let changed = ops
        .iter()
        .filter(|op| !matches!(op, Op::Keep(_, _)))
        .count();
    if changed as f64 / original.len() as f64 > MAX_EDIT_RATIO {
        return Vec::new();
    }

    let mut corrections = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for op in &ops {
        // Only one-for-one substitutions. An insertion or deletion is editing.
        let Op::Replace(from, to) = op else { continue };
        let Some(candidate) = evaluate(from, to) else {
            continue;
        };
        // Already in the dictionary, in any casing: learning it again is a no-op that
        // would still show up in the reviewable list as noise.
        if already_known.contains(&candidate.to.to_lowercase()) {
            continue;
        }
        if !seen.insert(candidate.to.to_lowercase()) {
            continue;
        }
        corrections.push(candidate);
        if corrections.len() >= MAX_PER_DICTATION {
            break;
        }
    }

    corrections
}

/// Whether one word replacing another is a correction worth learning.
///
/// Separate from [`detect`] so every rejection reason can be tested in isolation.
pub fn evaluate(from: &str, to: &str) -> Option<Correction> {
    let from_trimmed = trim_word(from);
    let to_trimmed = trim_word(to);

    if to_trimmed.chars().count() < MIN_WORD_LEN || to_trimmed.chars().count() > MAX_WORD_LEN {
        return None;
    }
    if from_trimmed.chars().count() < MIN_WORD_LEN {
        return None;
    }
    // Identical apart from surrounding punctuation: the user changed a comma, not a
    // word.
    if from_trimmed == to_trimmed {
        return None;
    }
    if !is_learnable_word(to_trimmed) {
        return None;
    }
    // The replaced word has to look like a word too, or a correction of "..." into a
    // name would be learned as though the name had been misheard.
    if !is_learnable_word(from_trimmed) {
        return None;
    }

    // A capitalisation-only change is a correction, and one of the most valuable:
    // `api` -> `API`, `priya` -> `Priya`. It bypasses the similarity check, which it
    // would trivially pass anyway.
    let case_only = from_trimmed.to_lowercase() == to_trimmed.to_lowercase();
    if !case_only && similarity(from_trimmed, to_trimmed) < MIN_SIMILARITY {
        return None;
    }

    Some(Correction {
        from: from_trimmed.to_string(),
        to: to_trimmed.to_string(),
    })
}

/// Normalised Levenshtein similarity in `[0, 1]`, case-insensitive.
///
/// Case-insensitive because casing is handled separately: `Barali` and `barali` are
/// the same word for the purpose of asking "did they fix a mishearing or change the
/// word", and treating a capital as a difference would make short corrections look
/// like rewrites.
pub fn similarity(a: &str, b: &str) -> f64 {
    strsim::normalized_levenshtein(&a.to_lowercase(), &b.to_lowercase())
}

/// Whether a token is the kind of thing a dictionary should hold.
///
/// Letters, with internal apostrophes and hyphens allowed because real names have
/// them (`O'Brien`, `Jean-Luc`). Digits are rejected outright: a number is not
/// vocabulary, and "2026" in a dictionary would make the fuzzy pass rewrite other
/// years.
fn is_learnable_word(word: &str) -> bool {
    let mut has_letter = false;
    for (index, ch) in word.chars().enumerate() {
        if ch.is_alphabetic() {
            has_letter = true;
            continue;
        }
        // Only between letters, never leading or trailing.
        let joiner = ch == '\'' || ch == '\u{2019}' || ch == '-';
        if joiner && index > 0 && index < word.chars().count() - 1 {
            continue;
        }
        return false;
    }
    has_letter
}

/// Strip surrounding punctuation, keeping the word.
fn trim_word(word: &str) -> &str {
    word.trim_matches(|ch: char| {
        !ch.is_alphanumeric() && ch != '\'' && ch != '\u{2019}' && ch != '-'
    })
}

/// Split into whitespace-separated tokens.
fn tokenize(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

/// Find the run of `edited` tokens that corresponds to the pasted text.
///
/// The field may contain text from before and after the paste — someone dictating
/// into a half-written email — and comparing against the whole field would report
/// every surrounding word as an insertion, blow past [`MAX_EDIT_RATIO`], and learn
/// nothing. Worse, without scoping, a correction the user made *elsewhere in the
/// document* would be attributed to our transcript.
///
/// Implemented as a best-window search rather than by anchoring on the first and last
/// few words. Anchoring was tried first and is wrong twice over: the anchor can
/// contain the very word that was corrected (a name at the start of a sentence is the
/// most likely thing to be misheard *and* the most likely thing to be at position
/// one), and a token carrying trailing punctuation never matches its bare
/// counterpart. Both failures land the same way — the anchor is not found, the whole
/// field is compared, the ratio guard rejects everything, and the feature silently
/// never learns anything.
///
/// So every plausible window is scored by word-level edit distance and the cheapest
/// one wins. That is inherently robust to which words changed and how many.
///
/// Returns a token index range into `edited`, or `None` when nothing resembles the
/// pasted text closely enough to be it.
fn locate_region(pasted: &[&str], edited: &[&str]) -> Option<(usize, usize)> {
    if pasted.is_empty() || edited.is_empty() {
        return None;
    }
    // A whole document is not something to scan word by word on a background tick.
    // Past this the paste cannot be located reliably anyway.
    if edited.len() > MAX_FIELD_TOKENS {
        return None;
    }

    // The window may be shorter or longer than the paste, since a correction can
    // merge or split words. Bounded, because a window far from the paste's length is
    // not the paste.
    let lower = (pasted.len() * 3 / 5).max(1);
    let upper = (pasted.len() * 7 / 5 + 1).min(edited.len());

    let mut best: Option<(usize, usize, usize)> = None;
    for start in 0..edited.len() {
        for length in lower..=upper {
            let end = start + length;
            if end > edited.len() {
                break;
            }
            let distance = word_distance(pasted, &edited[start..end]);
            if best.is_none_or(|(_, _, current)| distance < current) {
                best = Some((start, end, distance));
            }
            // Exact match: nothing can beat it, so stop looking.
            if distance == 0 {
                return Some((start, end));
            }
        }
    }

    let (start, end, distance) = best?;
    // Too different to be our text at all: the user deleted the transcript, or this
    // is a different field than the one written to. The threshold is generous —
    // half the words may have changed — because the edit-ratio guard downstream is
    // the one that decides whether an edit was a correction or a rewrite. This only
    // decides whether we are looking at the right text.
    if distance * 2 > pasted.len() {
        return None;
    }
    Some((start, end))
}

/// Above this many tokens in the target field, give up rather than scan.
const MAX_FIELD_TOKENS: usize = 4_000;

/// Word-level edit distance, comparing tokens by their [`compare_key`].
fn word_distance(a: &[&str], b: &[&str]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        current[0] = i;
        for j in 1..=b.len() {
            let same = compare_key(a[i - 1]) == compare_key(b[j - 1]);
            current[j] = (previous[j - 1] + usize::from(!same))
                .min(previous[j] + 1)
                .min(current[j - 1] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// The form of a token used for comparison.
///
/// Surrounding punctuation is stripped so `invoice` and `invoice.` are the same word —
/// otherwise a sentence-final full stop counts as a changed word and eats the edit
/// budget. Case is **kept**, because a capitalisation fix is one of the corrections
/// most worth learning and lowercasing here would make it invisible.
fn compare_key(token: &str) -> &str {
    trim_word(token)
}

/* ─────────────────────────────── alignment ─────────────────────────────── */

/// One step of the alignment between the original and edited word sequences.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Op {
    Keep(usize, usize),
    Replace(String, String),
    Insert(String),
    Delete(String),
}

/// Align two word sequences, cheapest edit script first.
///
/// A standard Levenshtein table over *words* rather than characters. Word-level is
/// the whole point: a correction is a word swapped for another word, and a
/// character-level diff would report `brali` -> `Barali` as "insert B, insert a" and
/// lose the pairing that makes it learnable.
///
/// Tokens are compared by [`compare_key`], so attached punctuation does not count as
/// a change — but the *original* tokens are what the ops carry, because that is what
/// gets learned.
///
/// Substitution is weighted equal to an insert-plus-delete pair so the table has no
/// preference between them; the tie is broken toward substitution in the traceback,
/// which is what surfaces corrections rather than hiding them as adjacent
/// insert/delete pairs.
fn align(original: &[&str], current: &[&str]) -> Vec<Op> {
    let rows = original.len() + 1;
    let cols = current.len() + 1;
    let mut cost = vec![vec![0usize; cols]; rows];

    for i in 0..rows {
        cost[i][0] = i;
    }
    for j in 0..cols {
        cost[0][j] = j;
    }
    for i in 1..rows {
        for j in 1..cols {
            let same = compare_key(original[i - 1]) == compare_key(current[j - 1]);
            let substitute = cost[i - 1][j - 1] + usize::from(!same);
            cost[i][j] = substitute.min(cost[i - 1][j] + 1).min(cost[i][j - 1] + 1);
        }
    }

    let mut ops = Vec::new();
    let mut i = original.len();
    let mut j = current.len();
    while i > 0 || j > 0 {
        if i > 0 && j > 0 {
            let same = compare_key(original[i - 1]) == compare_key(current[j - 1]);
            if cost[i][j] == cost[i - 1][j - 1] + usize::from(!same) {
                ops.push(if same {
                    Op::Keep(i - 1, j - 1)
                } else {
                    Op::Replace(original[i - 1].to_string(), current[j - 1].to_string())
                });
                i -= 1;
                j -= 1;
                continue;
            }
        }
        if i > 0 && cost[i][j] == cost[i - 1][j] + 1 {
            ops.push(Op::Delete(original[i - 1].to_string()));
            i -= 1;
            continue;
        }
        ops.push(Op::Insert(current[j - 1].to_string()));
        j -= 1;
    }

    ops.reverse();
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(words: &[&str]) -> HashSet<String> {
        words.iter().map(|word| word.to_lowercase()).collect()
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    /* ─────────────────────── the case this exists for ─────────────────────── */

    /// The motivating example. A misheard name, fixed by the user, learned by the app.
    #[test]
    fn a_corrected_name_is_learned() {
        let corrections = detect(
            "email brali about the invoice",
            "email Barali about the invoice",
            &none(),
        );
        assert_eq!(
            corrections,
            vec![Correction {
                from: "brali".into(),
                to: "Barali".into()
            }]
        );
    }

    /// Capitalisation is the most common correction of all, and exactly what a
    /// dictionary fixes. It must not be filtered out as "the same word".
    #[test]
    fn a_capitalisation_fix_is_learned() {
        assert_eq!(
            detect(
                "call the kubernetes team",
                "call the Kubernetes team",
                &none()
            ),
            vec![Correction {
                from: "kubernetes".into(),
                to: "Kubernetes".into()
            }]
        );
    }

    /// Nothing changed: no lesson, and specifically not an empty-region failure.
    #[test]
    fn an_untouched_transcript_teaches_nothing() {
        assert!(detect(
            "the invoice is due Friday",
            "the invoice is due Friday",
            &none()
        )
        .is_empty());
    }

    /* ───────────────────────── rejecting a rewrite ───────────────────────── */

    /// The most important rejection. Changing your mind is not a correction, and
    /// learning "elephant" because it replaced "cat" would put a wrong word in the
    /// dictionary forever.
    #[test]
    fn a_changed_mind_is_not_a_correction() {
        assert!(detect(
            "we should book the venue",
            "we should book the hotel",
            &none()
        )
        .is_empty());
    }

    /// A heavily edited paragraph is a rewrite. Nothing in it is attributable to a
    /// mishearing.
    #[test]
    fn a_rewritten_sentence_teaches_nothing() {
        assert!(detect(
            "one two three four five six",
            "alpha bravo charlie four delta echo",
            &none()
        )
        .is_empty());
    }

    /// Words added or removed are editing, not correcting.
    #[test]
    fn insertions_and_deletions_are_not_corrections() {
        assert!(detect(
            "send the invoice tomorrow",
            "send the invoice tomorrow please",
            &none()
        )
        .is_empty());
        assert!(detect("send the invoice tomorrow", "send the invoice", &none()).is_empty());
    }

    /* ──────────────────────────── word filters ──────────────────────────── */

    /// Below four characters the similarity metric is meaningless — any two
    /// three-letter words are "similar" — and short words are where a mistaken entry
    /// does the most damage, because the fuzzy pass matches them everywhere.
    #[test]
    fn short_words_are_not_learned() {
        assert!(evaluate("cat", "bat").is_none());
        assert!(evaluate("the", "teh").is_none());
        // Rejected on the *replacement's* length too, not only the original's.
        assert!(evaluate("thing", "hng").is_none());
    }

    /// A known and accepted limitation, asserted so it is a decision rather than a
    /// surprise: nothing here can distinguish "the recogniser misheard me and I fixed
    /// it" from "I introduced a typo". Both look like one similar word replacing
    /// another.
    ///
    /// It is accepted because the alternative is rejecting real corrections, and
    /// because the mitigation is structural rather than algorithmic: learned words go
    /// to `settings.learned_words` and never into the user's own `custom_words`, so
    /// they stay visible, individually removable, and clearable in one action.
    #[test]
    fn a_typo_is_indistinguishable_from_a_correction() {
        // Reads as the user fixing a mishearing.
        assert!(evaluate("kubernets", "kubernetes").is_some());
        // Reads identically, but is the user introducing a typo. Learned anyway.
        assert!(evaluate("kubernetes", "kubernets").is_some());
    }

    #[test]
    fn absurdly_long_words_are_not_learned() {
        let long = "a".repeat(MAX_WORD_LEN + 1);
        assert!(evaluate("aaaa", &long).is_none());
    }

    /// A number is not vocabulary, and "2026" in a dictionary would make the fuzzy
    /// pass rewrite other years.
    #[test]
    fn numbers_are_not_learned() {
        assert!(evaluate("2025", "2026").is_none());
        assert!(evaluate("v1beta", "v1beta2").is_none());
    }

    /// Real names carry these, so they must survive.
    #[test]
    fn apostrophes_and_hyphens_survive_inside_a_word() {
        assert!(evaluate("obrien", "O'Brien").is_some());
        // One token, not two: `evaluate` scores a single word against a single word,
        // and a two-words-for-one change is an edit rather than a correction.
        assert!(evaluate("jeanluc", "Jean-Luc").is_some());
        // But a joiner may not be the whole token, or lead, or trail.
        assert!(evaluate("word", "-----").is_none());
        assert!(evaluate("word", "-lead").is_none());
        assert!(evaluate("word", "trail-").is_none());
    }

    /// Only the punctuation around a word changed, so no word was corrected.
    #[test]
    fn a_punctuation_only_change_is_not_a_correction() {
        assert_eq!(evaluate("invoice,", "invoice"), None);
        assert_eq!(evaluate("invoice", "invoice."), None);
    }

    #[test]
    fn punctuation_is_stripped_from_what_is_learned() {
        let correction = evaluate("brali,", "Barali.").expect("still a correction");
        assert_eq!(correction.to, "Barali");
        assert_eq!(correction.from, "brali");
    }

    /* ────────────────────────── already known ────────────────────────── */

    /// Learning a word twice is a no-op that would still clutter the reviewable list.
    #[test]
    fn a_word_already_in_the_dictionary_is_not_learned_again() {
        assert!(detect(
            "email brali about it",
            "email Barali about it",
            &known(&["barali"])
        )
        .is_empty());
    }

    #[test]
    fn the_known_check_ignores_casing() {
        assert!(detect(
            "email brali about it",
            "email Barali about it",
            &known(&["BARALI"])
        )
        .is_empty());
    }

    /* ─────────────────────────── the cap ─────────────────────────── */

    /// Someone who changed several words was editing. The cap bounds what a single
    /// dictation can add even when every change passes the other guards.
    ///
    /// The fixture has to stay *under* [`MAX_EDIT_RATIO`] to reach the cap at all —
    /// four changes in twelve words is 0.33 — which is itself worth asserting: the two
    /// guards compose, and a fixture over the ratio would test nothing.
    #[test]
    fn no_more_than_the_cap_is_learned_from_one_dictation() {
        let pasted = "kubernetes prometheus grafana terraform alpha bravo charlie delta echo foxtrot golf hotel";
        let edited = "Kubernetes Prometheus Grafana Terraform alpha bravo charlie delta echo foxtrot golf hotel";
        let corrections = detect(pasted, edited, &none());
        assert_eq!(corrections.len(), MAX_PER_DICTATION);
    }

    #[test]
    fn the_same_correction_twice_is_learned_once() {
        let corrections = detect(
            "brali and brali again here",
            "Barali and Barali again here",
            &none(),
        );
        assert_eq!(corrections.len(), 1);
    }

    /* ─────────────────────── scoping to the pasted region ─────────────── */

    /// Dictating into a half-written email. Surrounding text the user typed must not
    /// be read as insertions, or the edit ratio rejects everything.
    #[test]
    fn surrounding_text_does_not_defeat_detection() {
        let pasted = "please email brali about the invoice";
        let edited =
            "Hi team, I wrote this myself. please email Barali about the invoice. Thanks, me.";
        assert_eq!(
            detect(pasted, edited, &none()),
            vec![Correction {
                from: "brali".into(),
                to: "Barali".into()
            }]
        );
    }

    /// The pasted text is gone: the user deleted it, or this is a different field
    /// than the one written to. Nothing to compare.
    #[test]
    fn a_deleted_transcript_teaches_nothing() {
        assert!(detect(
            "email brali about the invoice",
            "completely different content entirely unrelated",
            &none()
        )
        .is_empty());
    }

    #[test]
    fn empty_input_teaches_nothing() {
        assert!(detect("", "anything", &none()).is_empty());
        assert!(detect("something", "", &none()).is_empty());
        assert!(detect("", "", &none()).is_empty());
    }

    /* ──────────────────────────── similarity ──────────────────────────── */

    #[test]
    fn similarity_is_case_insensitive() {
        assert_eq!(similarity("Barali", "barali"), 1.0);
    }

    #[test]
    fn a_mishearing_scores_above_the_floor() {
        assert!(similarity("brali", "Barali") >= MIN_SIMILARITY);
        assert!(similarity("kubernetes", "kubernets") >= MIN_SIMILARITY);
        assert!(similarity("prometheus", "promethius") >= MIN_SIMILARITY);
    }

    #[test]
    fn an_unrelated_word_scores_below_the_floor() {
        assert!(similarity("venue", "hotel") < MIN_SIMILARITY);
        assert!(similarity("invoice", "elephant") < MIN_SIMILARITY);
    }

    /* ──────────────────────── multibyte safety ──────────────────────── */

    /// The target use case is code-switched Hindi/English, so every length check and
    /// every trim has to be a char operation. A byte slice through Devanagari panics.
    #[test]
    fn devanagari_does_not_panic() {
        let corrections = detect("मीटिंग के नोट्स लिखो", "मीटिंग के नोट्स लिखें", &none());
        // Whatever it decides, it must not panic and must not produce garbage.
        for correction in &corrections {
            assert!(!correction.to.is_empty());
        }
    }

    #[test]
    fn a_devanagari_word_can_be_learned() {
        // Devanagari is alphabetic, so it is learnable vocabulary like any other.
        assert!(evaluate("अभिषेक", "अभिषेकजी").is_some());
    }

    /* ──────────────────────────── alignment ──────────────────────────── */

    #[test]
    fn alignment_reports_a_substitution_as_a_replace() {
        let ops = align(&["a", "brali", "c"], &["a", "Barali", "c"]);
        assert!(ops
            .iter()
            .any(|op| matches!(op, Op::Replace(from, to) if from == "brali" && to == "Barali")));
    }

    #[test]
    fn alignment_of_identical_sequences_is_all_keeps() {
        let ops = align(&["a", "b"], &["a", "b"]);
        assert!(ops.iter().all(|op| matches!(op, Op::Keep(_, _))));
    }

    #[test]
    fn alignment_handles_empty_sequences() {
        assert!(align(&[], &[]).is_empty());
        assert_eq!(align(&["a"], &[]).len(), 1);
        assert_eq!(align(&[], &["a"]).len(), 1);
    }
}
