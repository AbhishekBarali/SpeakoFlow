use natural::phonetics::soundex;
use once_cell::sync::Lazy;
use regex::Regex;
use strsim::levenshtein;

/// Builds an n-gram string by cleaning and concatenating words
///
/// Strips punctuation from each word, lowercases, and joins without spaces.
/// This allows matching "Charge B" against "ChargeBee".
fn build_ngram(words: &[&str]) -> String {
    words
        .iter()
        .map(|w| build_match_key(w))
        .collect::<Vec<_>>()
        .concat()
}

/// Normalized comparison key for a word: keep only alphanumeric characters,
/// lowercased. Shared by the spoken-text n-gram and the custom-word keys so
/// both sides are normalized the same way.
fn build_match_key(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// A normalized comparison key for a custom word, plus the index of the custom
/// word it came from (so a match maps back to the original for the replacement).
struct CustomWordMatchKey {
    word_index: usize,
    key: String,
}

/// Build the comparison keys for one custom word. Always yields the primary
/// alphanumeric-only key; for words containing `&` (e.g. "R&D") it also yields
/// an expanded key with `&` spelled out as " and " so the spoken form
/// ("R and D") matches too (backport of Handy #1569).
fn build_custom_word_match_keys(word: &str, word_index: usize) -> Vec<CustomWordMatchKey> {
    let primary_key = build_match_key(word);
    let mut keys = Vec::with_capacity(2);

    if is_supported_fuzzy_key(&primary_key) {
        keys.push(CustomWordMatchKey {
            word_index,
            key: primary_key.clone(),
        });
    }

    if word.contains('&') {
        let expanded_key = build_match_key(&word.replace('&', " and "));
        if is_supported_fuzzy_key(&expanded_key) && expanded_key != primary_key {
            keys.push(CustomWordMatchKey {
                word_index,
                key: expanded_key,
            });
        }
    }

    keys
}

/// Whether a normalized key can take part in fuzzy matching.
///
/// The matcher tokenizes on whitespace, so it only makes sense for scripts that
/// separate words with spaces. Chinese and Japanese don't: a whole clause
/// arrives as one "word", and edit distance over it replaces text that merely
/// shares a character with a custom word. Those keys are skipped here (the
/// custom word still reaches models that take a native decode prompt).
///
/// Upstream Handy limits this to ASCII. SpeakoFlow keeps other space-separated
/// scripts (Devanagari, Cyrillic, accented Latin, Hangul…) because a Nepali or
/// Russian name is exactly the kind of word people add; lengths are measured in
/// characters, not bytes, so those scripts are scored on the same scale as
/// English instead of looking three times closer than they are.
fn is_supported_fuzzy_key(key: &str) -> bool {
    !key.is_empty() && !key.chars().any(is_unspaced_script_char)
}

/// Characters from scripts written without spaces between words: Han
/// ideographs (CJK Unified + Extension A + compatibility), Hiragana, Katakana,
/// and Thai/Lao/Khmer/Myanmar.
fn is_unspaced_script_char(c: char) -> bool {
    matches!(
        c as u32,
        0x3040..=0x30FF   // Hiragana, Katakana
        | 0x31F0..=0x31FF // Katakana phonetic extensions
        | 0x3400..=0x4DBF // CJK Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0xFF66..=0xFF9F // Half-width Katakana
        | 0x0E00..=0x0EFF // Thai, Lao
        | 0x1000..=0x109F // Myanmar
        | 0x1780..=0x17FF // Khmer
        | 0x20000..=0x2FA1F // CJK Extensions B+ and compatibility supplement
    )
}

/// Soundex is an English phonetic code: only meaningful for ASCII letters.
/// Numeric or non-Latin keys still use edit distance, but get no phonetic boost.
fn supports_soundex(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphabetic())
}

/// Finds the best matching custom word for a candidate string
///
/// Uses Levenshtein distance and Soundex phonetic matching to find
/// the best match above the given threshold.
///
/// # Arguments
/// * `candidate` - The cleaned/lowercased candidate string to match
/// * `custom_words` - Original custom words (for returning the replacement)
/// * `custom_words_nospace` - Custom words with spaces removed, lowercased (for comparison)
/// * `threshold` - Maximum similarity score to accept
///
/// # Returns
/// The best matching custom word and its score, if any match was found
fn find_best_match<'a>(
    candidate: &str,
    custom_words: &'a [String],
    custom_word_match_keys: &[CustomWordMatchKey],
    threshold: f64,
) -> Option<(&'a String, f64)> {
    if !is_supported_fuzzy_key(candidate) || candidate.chars().count() > 50 {
        return None;
    }

    let mut best_match: Option<&String> = None;
    let mut best_score = f64::MAX;

    for custom_word_key in custom_word_match_keys {
        // Skip if lengths are too different (optimization + prevents over-matching)
        // Use percentage-based check: max 25% length difference (prevents n-grams from
        // matching significantly shorter custom words, e.g., "openaigpt" vs "openai").
        // Lengths are in characters: byte lengths made every non-ASCII word look
        // two to three times longer, so its edit distance normalized to a far
        // smaller score and near-misses in Devanagari or Cyrillic got replaced.
        let candidate_len = candidate.chars().count();
        let custom_word_len = custom_word_key.key.chars().count();
        let len_diff = candidate_len.abs_diff(custom_word_len) as f64;
        let max_len = candidate_len.max(custom_word_len) as f64;
        let max_allowed_diff = (max_len * 0.25).max(2.0); // At least 2 chars difference allowed
        if len_diff > max_allowed_diff {
            continue;
        }

        // Calculate Levenshtein distance (normalized by length)
        let levenshtein_dist = levenshtein(candidate, &custom_word_key.key);
        let levenshtein_score = if max_len > 0.0 {
            levenshtein_dist as f64 / max_len
        } else {
            1.0
        };

        // Soundex is an English/ASCII phonetic algorithm. Other keys can still
        // use edit distance, but must not receive a phonetic boost.
        let phonetic_match = supports_soundex(candidate)
            && supports_soundex(&custom_word_key.key)
            && soundex(candidate, &custom_word_key.key);

        // Combine scores: favor phonetic matches, but also consider string similarity
        let combined_score = if phonetic_match {
            levenshtein_score * 0.3 // Give significant boost to phonetic matches
        } else {
            levenshtein_score
        };

        // Accept if the score is good enough (configurable threshold)
        if combined_score < threshold && combined_score < best_score {
            best_match = Some(&custom_words[custom_word_key.word_index]);
            best_score = combined_score;
        }
    }

    best_match.map(|m| (m, best_score))
}

/// Applies custom word corrections to transcribed text using fuzzy matching
///
/// This function corrects words in the input text by finding the best matches
/// from a list of custom words using a combination of:
/// - Levenshtein distance for string similarity
/// - Soundex phonetic matching for pronunciation similarity
/// - N-gram matching for multi-word speech artifacts (e.g., "Charge B" -> "ChargeBee")
///
/// # Arguments
/// * `text` - The input text to correct
/// * `custom_words` - List of custom words to match against
/// * `threshold` - Maximum similarity score to accept (0.0 = exact match, 1.0 = any match)
///
/// # Returns
/// The corrected text with custom words applied
pub fn apply_custom_words(text: &str, custom_words: &[String], threshold: f64) -> String {
    if custom_words.is_empty() {
        return text.to_string();
    }

    // Pre-compute normalized comparison keys to avoid repeated allocations.
    // Each custom word yields its alphanumeric-only key, plus (for words with
    // '&') an expanded " and " variant so both "R&D" and "R and D" match.
    let custom_word_match_keys: Vec<CustomWordMatchKey> = custom_words
        .iter()
        .enumerate()
        .flat_map(|(index, word)| build_custom_word_match_keys(word, index))
        .collect();

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut result = Vec::new();
    let mut i = 0;

    while i < words.len() {
        let best_match =
            best_ngram_match_at(&words, i, custom_words, &custom_word_match_keys, threshold);

        // A multi-word match that begins with an ordinary word can be beaten by
        // a closer match starting one word later: in "nome è Charge B," the span
        // "è Charge B" scores under the threshold, but "Charge B" alone is far
        // closer to "ChargeBee". Leave the leading word alone in that case so
        // the better match gets its turn instead of the neighbour being eaten.
        if let Some((n, _, score)) = best_match {
            if n > 1 {
                let later = best_ngram_match_at(
                    &words,
                    i + 1,
                    custom_words,
                    &custom_word_match_keys,
                    threshold,
                );
                if let Some((later_n, _, later_score)) = later {
                    if later_score < score && i + 1 + later_n >= i + n {
                        result.push(words[i].to_string());
                        i += 1;
                        continue;
                    }
                }
            }
        }

        if let Some((n, replacement, _)) = best_match {
            let ngram_words = &words[i..i + n];
            // Extract punctuation from first and last words of the n-gram.
            let (prefix, _) = extract_punctuation(ngram_words[0]);
            let (_, suffix) = extract_punctuation(ngram_words[n - 1]);

            // Preserve case from first word.
            let corrected = preserve_case_pattern(ngram_words[0], replacement);

            result.push(format!("{}{}{}", prefix, corrected, suffix));
            i += n;
        } else {
            result.push(words[i].to_string());
            i += 1;
        }
    }

    result.join(" ")
}

/// The closest custom-word match for an n-gram (up to three words) starting at
/// `words[i]`, as `(n, replacement, score)`.
///
/// Chooses the *closest* match rather than the longest one. Longest-first let a
/// phrase swallow the ordinary word after it whenever both scored under the
/// threshold: "ask ChatGPT to do it" became "ask ChatGPT do it", because
/// "chatgptto" shares ChatGPT's Soundex code. Backport of Handy 2203a826.
fn best_ngram_match_at<'a>(
    words: &[&str],
    i: usize,
    custom_words: &'a [String],
    custom_word_match_keys: &[CustomWordMatchKey],
    threshold: f64,
) -> Option<(usize, &'a String, f64)> {
    let mut best_match: Option<(usize, &'a String, f64)> = None;
    if i >= words.len() {
        return None;
    }

    for n in (1..=3).rev() {
        if i + n > words.len() {
            continue;
        }

        let ngram_words = &words[i..i + n];
        // Do not consume across a punctuation boundary. In
        // "Charge B, che", the comma closes the candidate at "B,".
        if ngram_words[..n - 1]
            .iter()
            .any(|word| !extract_punctuation(word).1.is_empty())
        {
            continue;
        }
        let ngram = build_ngram(ngram_words);

        if let Some((replacement, score)) =
            find_best_match(&ngram, custom_words, custom_word_match_keys, threshold)
        {
            let is_better = best_match
                .as_ref()
                .is_none_or(|(_, _, best_score)| score < *best_score);
            if is_better {
                best_match = Some((n, replacement, score));
            }
        }
    }

    best_match
}

/// Preserves the case pattern of the original word when applying a replacement
fn preserve_case_pattern(original: &str, replacement: &str) -> String {
    if original.chars().all(|c| c.is_uppercase()) {
        replacement.to_uppercase()
    } else if original.chars().next().map_or(false, |c| c.is_uppercase()) {
        let mut chars: Vec<char> = replacement.chars().collect();
        if let Some(first_char) = chars.get_mut(0) {
            *first_char = first_char.to_uppercase().next().unwrap_or(*first_char);
        }
        chars.into_iter().collect()
    } else {
        replacement.to_string()
    }
}

/// Extracts punctuation prefix and suffix from a word
fn extract_punctuation(word: &str) -> (&str, &str) {
    // String slices use byte offsets. Derive both boundaries from char_indices
    // so multibyte punctuation such as `。`, `「」`, `…`, `—` or the Devanagari
    // danda `।` can never be split. Counting characters and slicing by that
    // count panicked on all of them, which lost the whole dictation.
    let prefix_end = word
        .char_indices()
        .find(|(_, c)| c.is_alphanumeric())
        .map(|(index, _)| index)
        .unwrap_or(word.len());
    let suffix_start = word
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_alphanumeric())
        .map(|(index, c)| index + c.len_utf8())
        .unwrap_or(0);

    let prefix = if prefix_end > 0 {
        &word[..prefix_end]
    } else {
        ""
    };

    let suffix = if suffix_start < word.len() {
        &word[suffix_start..]
    } else {
        ""
    };

    (prefix, suffix)
}

/// Filler tokens that are not a word in any language the speech models output,
/// so removing them cannot corrupt text whatever the spoken language is. Kept
/// deliberately conservative: anything that is a real word somewhere ("um" in
/// Portuguese and German, "ha" in Spanish, "ah"/"eh" interjections, "mm" for
/// millimetres) belongs in the language-gated lists instead.
/// Mirrors Handy #1738's universal tier.
const UNIVERSAL_FILLER_WORDS: &[&str] = &[
    "uh", "uhm", "umm", "uhh", "uhhh", "ehh", "ehm", "ahm", "hmm", "hm", "mmm", "хм", "ммм",
];

/// Filler words that are only safe to remove when the *spoken* language is
/// known, because the same token is a real word elsewhere (Portuguese "um" =
/// "a/an", German "um" = "at/around", Spanish "ha" = "has").
///
/// "ha" is not an English filler either: the pattern is case-insensitive, so
/// "Ha Long Bay" became "Long Bay" (Handy #2156). "mm" is gone for the same
/// reason — "the screw is 5 mm long" lost its unit.
fn gated_filler_words_for_language(lang: &str) -> &'static [&'static str] {
    let base_lang = lang.split(&['-', '_'][..]).next().unwrap_or(lang);

    match base_lang {
        "en" => &["um", "ah", "eh"],
        "de" => &["äh", "ähm"],
        "fr" => &["euh"],
        _ => &[],
    }
}

/// Common function words used to recognise the spoken language from the text
/// itself when the user has left the transcription language on "auto". Only the
/// languages with a gated filler list need an entry.
const LANGUAGE_MARKERS: &[(&str, &[&str])] = &[
    (
        "en",
        &[
            "the", "and", "is", "are", "was", "to", "of", "it", "that", "this", "you", "i", "we",
            "they", "have", "with", "for", "not", "what", "so", "just", "my", "your", "do",
        ],
    ),
    (
        "de",
        &[
            "der", "die", "das", "und", "ist", "ich", "nicht", "ein", "eine", "zu", "mit", "wir",
            "sie", "es", "auf", "den", "dem", "sind", "auch", "aber",
        ],
    ),
    (
        "fr",
        &[
            "le", "la", "les", "et", "est", "je", "pas", "une", "des", "que", "qui", "nous",
            "vous", "il", "elle", "dans", "pour", "sur", "avec", "mais",
        ],
    ),
];

/// Best guess at the spoken language of `text`, or `None` without clear
/// evidence. Deliberately fails closed: a short or ambiguous transcript yields
/// `None`, which keeps only the universal fillers, so a Portuguese "um" is never
/// deleted on a guess.
pub fn detect_filler_language(text: &str) -> Option<&'static str> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphabetic() || *c == '\'')
                .flat_map(|c| c.to_lowercase())
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() < 3 {
        return None;
    }

    let mut best: Option<(&'static str, usize)> = None;
    let mut runner_up = 0usize;
    for (lang, markers) in LANGUAGE_MARKERS {
        let hits = words
            .iter()
            .filter(|w| markers.contains(&w.as_str()))
            .count();
        match best {
            Some((_, top)) if hits > top => {
                runner_up = top;
                best = Some((lang, hits));
            }
            Some(_) => runner_up = runner_up.max(hits),
            None => best = Some((lang, hits)),
        }
    }

    let (lang, hits) = best?;
    // At least two markers, a clear lead over the next language, and a
    // meaningful share of the words — one stray "die" in an English sentence
    // must not flip it to German.
    let share = hits as f64 / words.len() as f64;
    (hits >= 2 && hits > runner_up && share >= 0.15).then_some(lang)
}

static MULTI_SPACE_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s{2,}").unwrap());

/// Two commas left adjacent by a deletion between them ("just, uh, fixing").
static DOUBLED_COMMA_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r",(\s*,)+").unwrap());

/// A comma stranded immediately after sentence-ending punctuation, which happens
/// when the filler that owned it opened the sentence ("right? Uh, for" → "right? ,for").
static ORPHAN_COMMA_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"([.?!])\s*,\s*").unwrap());

/// Whitespace before a comma or full stop, left by a deletion in front of it.
static SPACE_BEFORE_PUNCT_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+([,.])").unwrap());

/// A lowercase word opening a sentence after `?` or `!`. Only those two, never
/// `.`: a full stop is ambiguous ("e.g. foo", "vs. the") and capitalising after
/// it would corrupt abbreviations, while `?` and `!` always end a sentence.
static LOWERCASE_AFTER_TERMINAL_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"([?!]\s+)(\p{Ll})").unwrap());

/// Repairs the punctuation and casing seams that removing a word leaves behind.
///
/// Deleting a filler is not a clean excision: the filler often owned the comma
/// after it and sat at the start of a sentence, so a naive removal turns
/// "right? Uh, for example" into "right? for example" — a sentence opening in
/// lowercase — and "just, uh, um, fixing" into "just, , fixing". The text is
/// then pasted straight into the user's document, so these seams are the visible
/// output, not an intermediate.
///
/// Only unambiguous repairs are made. Casing is fixed after `?` and `!` but never
/// after `.`, because a full stop is also an abbreviation mark and
/// "e.g. foo" must not become "e.g. Foo".
///
/// Shared with AI cleanup (`actions::sanitize_post_process_output`) rather than
/// kept private to the local filter, because an LLM leaves the same seams when it
/// deletes a filler and forgets the comma that came with it. One implementation,
/// one set of tests, both paths.
pub(crate) fn repair_removal_seams(text: &str) -> String {
    let mut out = DOUBLED_COMMA_PATTERN.replace_all(text, ",").to_string();
    out = ORPHAN_COMMA_PATTERN.replace_all(&out, "$1 ").to_string();
    out = SPACE_BEFORE_PUNCT_PATTERN
        .replace_all(&out, "$1")
        .to_string();
    // A deletion at the very start can leave the text opening on its old
    // separator.
    out = out.trim_start_matches([',', '.', ' ']).to_string();
    out = LOWERCASE_AFTER_TERMINAL_PATTERN
        .replace_all(&out, |caps: &regex::Captures| {
            format!("{}{}", &caps[1], caps[2].to_uppercase())
        })
        .to_string();
    // Deliberately NOT capitalising the first character of the transcript.
    // Dictation is frequently a continuation — the user is mid-sentence in a
    // document with the cursor after "and then," — so forcing an initial capital
    // would corrupt more text than it fixed. Only an interior sentence boundary
    // is unambiguous.
    out
}

/// Collapses repeated words (3+ repetitions) to a single instance.
/// E.g., "wh wh wh wh" -> "wh", "I I I I" -> "I"
fn collapse_stutters(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return text.to_string();
    }

    let mut result: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < words.len() {
        let word = words[i];
        let word_lower = word.to_lowercase();

        if word_lower.chars().all(|c| c.is_alphabetic()) {
            // Count consecutive repetitions (case-insensitive)
            let mut count = 1;
            while i + count < words.len() && words[i + count].to_lowercase() == word_lower {
                count += 1;
            }

            // If 3+ repetitions, collapse to single instance
            if count >= 3 {
                result.push(word);
                i += count;
            } else {
                result.push(word);
                i += 1;
            }
        } else {
            result.push(word);
            i += 1;
        }
    }

    result.join(" ")
}

/// Whether a transcription carries no speech at all.
///
/// An empty string is the obvious case, but it is not the common one: a local
/// engine handed silence does not answer with nothing. whisper.cpp emits
/// `[BLANK_AUDIO]`; Whisper models also produce bracketed or parenthesised
/// annotations (`[silence]`, `(music)`), a musical note for non-speech audio, or
/// bare punctuation like `...` when there is nothing to hear. Every one of those
/// arrives at the output pipeline as ordinary text, so it gets pasted into
/// whatever the user was typing into — and with AI cleanup on, it first spends the
/// entire cleanup budget, plus a cold engine start on top, asking a language model
/// to tidy up a string with no content in it. From the user's side that is
/// "nothing happens for ages and then it types garbage".
///
/// The test is deliberately about *content*, not about matching a list of known
/// markers: text is speechless when nothing outside a bracketed group is
/// alphanumeric. That covers annotations this code has never seen, in any script,
/// while a real dictation — which always has a word in it — is never caught.
pub fn is_speechless_transcription(text: &str) -> bool {
    let mut square = 0usize;
    let mut round = 0usize;
    for ch in text.chars() {
        match ch {
            '[' => square += 1,
            ']' => square = square.saturating_sub(1),
            '(' => round += 1,
            ')' => round = round.saturating_sub(1),
            _ if square == 0 && round == 0 && ch.is_alphanumeric() => return false,
            _ => {}
        }
    }
    true
}

/// Filters transcription output by removing filler words and stutter artifacts.
///
/// This function cleans up raw transcription text by:
/// 1. Removing filler words: the universal tier always, plus the gated tier for
///    the spoken language (or the user's custom list instead of both)
/// 2. Collapsing repeated word stutters (e.g., "wh wh wh" -> "wh")
/// 3. Cleaning up excess whitespace
///
/// # Arguments
/// * `text` - The raw transcription text to filter
/// * `lang` - The language that was **spoken** (e.g., "en", "pt-BR"). Not the
///   app's UI language: that defaults to the OS locale, so a Portuguese speaker
///   on an English UI had every "um" (= "a") deleted. Pass `""` or `"auto"` when
///   unknown; the language is then detected from the text, and without clear
///   evidence only the universal fillers are removed.
/// * `custom_filler_words` - Optional user-provided filler word list. `Some(vec)` overrides
///   language defaults; `Some(empty vec)` disables filtering; `None` uses language defaults.
///
/// # Returns
/// The filtered text with filler words and stutters removed
pub fn filter_transcription_output(
    text: &str,
    lang: &str,
    custom_filler_words: &Option<Vec<String>>,
) -> String {
    let mut filtered = text.to_string();

    let spoken_language = match lang.trim() {
        "" | "auto" => detect_filler_language(text),
        known => Some(known),
    };

    // Build filler patterns from custom list or the built-in tiers
    let patterns: Vec<Regex> = match custom_filler_words {
        Some(words) => words
            .iter()
            .filter_map(|word| Regex::new(&format!(r"(?i)\b{}\b[,.]?", regex::escape(word))).ok())
            .collect(),
        None => UNIVERSAL_FILLER_WORDS
            .iter()
            .chain(
                spoken_language
                    .map(gated_filler_words_for_language)
                    .unwrap_or_default(),
            )
            .map(|word| Regex::new(&format!(r"(?i)\b{}\b[,.]?", regex::escape(word))).unwrap())
            .collect(),
    };

    // Remove filler words
    let mut removed_any = false;
    for pattern in &patterns {
        if pattern.is_match(&filtered) {
            removed_any = true;
            filtered = pattern.replace_all(&filtered, "").to_string();
        }
    }

    // Repair the seams the deletions left, but only if something was deleted —
    // text that was never cut has no seams, and must pass through untouched.
    if removed_any {
        filtered = repair_removal_seams(&filtered);
    }

    // Collapse repeated 1-2 letter words (stutter artifacts like "wh wh wh wh")
    filtered = collapse_stutters(&filtered);

    // Clean up multiple spaces to single space
    filtered = MULTI_SPACE_PATTERN.replace_all(&filtered, " ").to_string();

    // Trim leading/trailing whitespace
    filtered.trim().to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn silence_markers_from_a_local_engine_count_as_no_speech() {
        // The bug this closes: a recording with nothing in it does not arrive as
        // an empty string, so it sailed past the empty check into AI cleanup —
        // burning the whole cleanup budget (and a cold engine start) before
        // pasting a marker the user never said.
        for text in [
            "",
            "   ",
            "\n\t ",
            "[BLANK_AUDIO]",
            " [BLANK_AUDIO] ",
            "[blank_audio]\n[BLANK_AUDIO]",
            "[silence]",
            "[ Silence ]",
            "(silence)",
            "(music)",
            "[MUSIC PLAYING]",
            "...",
            ".",
            " . . . ",
            "-",
            "♪",
            "♪♪♪",
            "[BLANK_AUDIO].",
        ] {
            assert!(
                super::is_speechless_transcription(text),
                "expected no speech in {text:?}"
            );
        }
    }

    #[test]
    fn a_real_dictation_is_never_mistaken_for_silence() {
        for text in [
            "Hello.",
            "yes",
            "7",
            "ok [BLANK_AUDIO]",
            "[BLANK_AUDIO] but then I kept talking",
            "See the note (below) for details.",
            "Ship it.",
            "a",
            // Any script, because the test is for alphanumeric content rather
            // than for a list of English markers.
            "こんにちは",
            "привет",
            "नमस्ते",
            "  spaced out  ",
            // An unbalanced bracket must not swallow the words around it.
            "Hello [world",
            ") stray close",
        ] {
            assert!(
                !super::is_speechless_transcription(text),
                "expected speech in {text:?}"
            );
        }
    }
    use super::*;

    #[test]
    fn test_apply_custom_words_exact_match() {
        let text = "hello world";
        let custom_words = vec!["Hello".to_string(), "World".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn test_apply_custom_words_fuzzy_match() {
        let text = "helo wrold";
        let custom_words = vec!["hello".to_string(), "world".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_preserve_case_pattern() {
        assert_eq!(preserve_case_pattern("HELLO", "world"), "WORLD");
        assert_eq!(preserve_case_pattern("Hello", "world"), "World");
        assert_eq!(preserve_case_pattern("hello", "WORLD"), "WORLD");
    }

    #[test]
    fn test_extract_punctuation() {
        assert_eq!(extract_punctuation("hello"), ("", ""));
        assert_eq!(extract_punctuation("!hello?"), ("!", "?"));
        assert_eq!(extract_punctuation("...hello..."), ("...", "..."));
    }

    #[test]
    fn test_empty_custom_words() {
        let text = "hello world";
        let custom_words = vec![];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_filter_filler_words() {
        let text = "So uhm I was thinking uh about this";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "So I was thinking about this");
    }

    /// Removing a filler that opened a sentence used to leave the next word in
    /// lowercase: "right? Uh, for example" became "right? for example". Observed
    /// on a real cloud transcript.
    #[test]
    fn removing_a_sentence_opening_filler_keeps_the_sentence_capitalised() {
        let text = "doing a few basic things, right? Uh, for example, making good sentences.";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(
            result,
            "doing a few basic things, right? For example, making good sentences."
        );
    }

    /// The same seam after an exclamation mark.
    #[test]
    fn capitalisation_repair_also_applies_after_an_exclamation() {
        let result = filter_transcription_output("Stop! Um, wait a second.", "en", &None);
        assert_eq!(result, "Stop! Wait a second.");
    }

    /// A full stop is also an abbreviation mark, so casing after it is left
    /// alone — "e.g. foo" must not become "e.g. Foo".
    #[test]
    fn casing_after_a_full_stop_is_left_alone() {
        let result = filter_transcription_output("Use a tool, uh, e.g. ripgrep here.", "en", &None);
        assert_eq!(result, "Use a tool, e.g. ripgrep here.");
    }

    /// Two fillers in a row each owned a comma, which used to leave ", ,".
    #[test]
    fn consecutive_fillers_do_not_leave_a_doubled_comma() {
        let text = "Sorry, I mean just, uh, um, fixing some small mistakes.";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Sorry, I mean just, fixing some small mistakes.");
        assert!(!result.contains(", ,"));
        assert!(!result.contains(",,"));
    }

    /// Text with no fillers must come back byte-for-byte: the repair pass only
    /// runs when something was actually cut, so it can never touch clean text.
    #[test]
    fn text_without_fillers_is_untouched_by_the_repair() {
        let text = "Why? because it works. And e.g. this stays lowercase.";
        assert_eq!(filter_transcription_output(text, "en", &None), text);
    }

    /// The documented way to turn filtering off entirely.
    #[test]
    fn an_empty_custom_list_disables_filler_removal() {
        let text = "So uh I was, um, thinking";
        let result = filter_transcription_output(text, "en", &Some(vec![]));
        assert_eq!(result, text);
    }

    #[test]
    fn test_filter_filler_words_case_insensitive() {
        let text = "UHM this is UH a test";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "this is a test");
    }

    #[test]
    fn test_filter_filler_words_with_punctuation() {
        let text = "Well, uhm, I think, uh. that's right";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Well, I think, that's right");
    }

    #[test]
    fn test_filter_cleans_whitespace() {
        let text = "Hello    world   test";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Hello world test");
    }

    #[test]
    fn test_filter_trims() {
        let text = "  Hello world  ";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn test_filter_combined() {
        let text = "  Uhm, so I was, uh, thinking about this  ";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "so I was, thinking about this");
    }

    #[test]
    fn test_filter_preserves_valid_text() {
        let text = "This is a completely normal sentence.";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "This is a completely normal sentence.");
    }

    #[test]
    fn test_filter_stutter_collapse() {
        let text = "w wh wh wh wh wh wh wh wh wh why";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "w wh why");
    }

    #[test]
    fn test_filter_stutter_short_words() {
        let text = "I I I I think so so so so";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "I think so");
    }

    #[test]
    fn test_filter_stutter_longer_words() {
        let text = "Check data doc doc doc doc documentation.";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "Check data doc documentation.");
    }

    #[test]
    fn test_filter_stutter_mixed_case() {
        let text = "No NO no NO no";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "No");
    }

    #[test]
    fn test_filter_stutter_preserves_two_repetitions() {
        let text = "no no is fine";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "no no is fine");
    }

    #[test]
    fn test_filter_english_removes_um() {
        let text = "um I think um this is good";
        let result = filter_transcription_output(text, "en", &None);
        assert_eq!(result, "I think this is good");
    }

    #[test]
    fn test_filter_portuguese_preserves_um() {
        // "um" means "a/an" in Portuguese
        let text = "um gato bonito";
        let result = filter_transcription_output(text, "pt", &None);
        assert_eq!(result, "um gato bonito");
    }

    #[test]
    fn test_filter_spanish_preserves_ha() {
        // "ha" means "has" in Spanish
        let text = "ha sido un buen día";
        let result = filter_transcription_output(text, "es", &None);
        assert_eq!(result, "ha sido un buen día");
    }

    #[test]
    fn test_filter_language_code_with_region() {
        // "pt-BR" should normalize to "pt"
        let text = "um gato bonito";
        let result = filter_transcription_output(text, "pt-BR", &None);
        assert_eq!(result, "um gato bonito");
    }

    #[test]
    fn test_filter_custom_filler_words_override() {
        let custom = Some(vec!["okay".to_string(), "right".to_string()]);
        let text = "okay so I think right this works";
        let result = filter_transcription_output(text, "en", &custom);
        assert_eq!(result, "so I think this works");
    }

    #[test]
    fn test_filter_custom_filler_words_empty_disables() {
        let custom = Some(vec![]);
        let text = "So uhm I was thinking uh about this";
        let result = filter_transcription_output(text, "en", &custom);
        // No filler words removed since custom list is empty
        assert_eq!(result, "So uhm I was thinking uh about this");
    }

    #[test]
    fn test_filter_unknown_language_uses_fallback() {
        let text = "uh I think uhm this works";
        let result = filter_transcription_output(text, "xx", &None);
        assert_eq!(result, "I think this works");
    }

    #[test]
    fn test_filter_fallback_does_not_remove_um() {
        // Fallback (unknown language) should not remove "um" since it's a real word in some languages
        let text = "um I think this works";
        let result = filter_transcription_output(text, "xx", &None);
        assert_eq!(result, "um I think this works");
    }

    #[test]
    fn test_apply_custom_words_ngram_two_words() {
        let text = "il cui nome è Charge B, che permette";
        let custom_words = vec!["ChargeBee".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("ChargeBee,"));
        assert!(!result.contains("Charge B"));
    }

    #[test]
    fn test_apply_custom_words_ngram_three_words() {
        let text = "use Chat G P T for this";
        let custom_words = vec!["ChatGPT".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("ChatGPT"));
    }

    #[test]
    fn test_apply_custom_words_prefers_longer_ngram() {
        let text = "Open AI GPT model";
        let custom_words = vec!["OpenAI".to_string(), "GPT".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert_eq!(result, "OpenAI GPT model");
    }

    #[test]
    fn test_apply_custom_words_ngram_preserves_case() {
        let text = "CHARGE B is great";
        let custom_words = vec!["ChargeBee".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("CHARGEBEE"));
    }

    #[test]
    fn test_apply_custom_words_ngram_with_spaces_in_custom() {
        // Custom word with space should also match against split words
        let text = "using Mac Book Pro";
        let custom_words = vec!["MacBook Pro".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        assert!(result.contains("MacBook"));
    }

    #[test]
    fn test_apply_custom_words_trailing_number_not_doubled() {
        // Verify that trailing non-alpha chars (like numbers) aren't double-counted
        // between build_ngram stripping them and extract_punctuation capturing them
        let text = "use GPT4 for this";
        let custom_words = vec!["GPT-4".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.5);
        // Should NOT produce "GPT-44" (double-counting the trailing 4)
        assert!(
            !result.contains("GPT-44"),
            "got double-counted result: {}",
            result
        );
    }

    #[test]
    fn test_apply_custom_words_matches_ampersand_word() {
        let text = "send it to RD for review";
        let custom_words = vec!["R&D".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.18);
        assert_eq!(result, "send it to R&D for review");
    }

    #[test]
    fn test_apply_custom_words_matches_spoken_ampersand_word() {
        let text = "send it to R and D for review";
        let custom_words = vec!["R&D".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.18);
        assert_eq!(result, "send it to R&D for review");
    }

    #[test]
    fn test_apply_custom_words_preserves_ampersand_word() {
        let text = "send it to R&D for review";
        let custom_words = vec!["R&D".to_string()];
        let result = apply_custom_words(text, &custom_words, 0.18);
        assert_eq!(result, "send it to R&D for review");
    }

    // ---- Handy 2203a826: closest match, char boundaries, no panics ----------

    #[test]
    fn test_extract_punctuation_uses_unicode_boundaries() {
        assert_eq!(extract_punctuation("你好。"), ("", "。"));
        assert_eq!(extract_punctuation("「你好」"), ("「", "」"));
        assert_eq!(extract_punctuation("你好！"), ("", "！"));
        assert_eq!(extract_punctuation("…word—"), ("…", "—"));
        assert_eq!(extract_punctuation("अभिषेक।"), ("", "।"));
    }

    #[test]
    fn test_apply_custom_words_handles_unicode_punctuation() {
        let result = apply_custom_words("「Handee。」", &["Handy".to_string()], 0.5);
        assert_eq!(result, "「Handy。」");
    }

    #[test]
    fn test_apply_custom_words_skips_cjk_fuzzy_matching() {
        // Chinese has no spaces, so a whole clause is one "word"; edit distance
        // over it would rewrite text that merely shares a character.
        let text = "你好。";
        let result = apply_custom_words(text, &["你号".to_string()], 1.0);
        assert_eq!(result, text);
    }

    #[test]
    fn test_apply_custom_words_does_not_swallow_the_next_word() {
        let words = vec!["ChatGPT".to_string()];
        assert_eq!(
            apply_custom_words("ask ChatGPT to do it", &words, 0.18),
            "ask ChatGPT to do it"
        );
        let words = vec!["SpeakoFlow".to_string()];
        assert_eq!(
            apply_custom_words("Tell SpeakoFlow to stop", &words, 0.18),
            "Tell SpeakoFlow to stop"
        );
    }

    #[test]
    fn test_apply_custom_words_stops_at_punctuation_and_keeps_neighbours() {
        let words = vec!["ChargeBee".to_string()];
        let text = "il cui nome è Charge B, che permette";
        for threshold in [0.18, 0.5] {
            assert_eq!(
                apply_custom_words(text, &words, threshold),
                "il cui nome è ChargeBee, che permette",
                "threshold {threshold}"
            );
        }
    }

    #[test]
    fn test_apply_custom_words_devanagari_is_matched_by_characters() {
        // Non-ASCII words stay eligible (SpeakoFlow keeps them on), scored by
        // character count: one substitution in a six-character name is 1/6.
        let words = vec!["अभिषेक".to_string()];
        assert_eq!(
            apply_custom_words("मेरो नाम अभिशेक।", &words, 0.18),
            "मेरो नाम अभिषेक।"
        );
        // An exact match next to a danda is left intact (and doesn't panic).
        assert_eq!(
            apply_custom_words("मेरो नाम अभिषेक।", &words, 0.18),
            "मेरो नाम अभिषेक।"
        );
    }

    // ---- Filler words follow the spoken language (Handy #1738 / #2156) -----

    #[test]
    fn test_filter_keeps_ha_and_mm_in_english() {
        assert_eq!(
            filter_transcription_output("Ha Long Bay is beautiful.", "en", &None),
            "Ha Long Bay is beautiful."
        );
        assert_eq!(
            filter_transcription_output("the screw is 5 mm long", "en", &None),
            "the screw is 5 mm long"
        );
    }

    #[test]
    fn test_filter_auto_language_keeps_ambiguous_words() {
        // No evidence the speaker used English, so "um" (Portuguese "a") stays.
        assert_eq!(
            filter_transcription_output("eu vi um carro", "auto", &None),
            "eu vi um carro"
        );
        assert_eq!(
            filter_transcription_output("uhh bueno hmm creo que um ha llegado", "auto", &None),
            "bueno creo que um ha llegado"
        );
        assert_eq!(
            filter_transcription_output("хм я думаю ммм это работает", "auto", &None),
            "я думаю это работает"
        );
    }

    #[test]
    fn test_filter_auto_language_detects_english_text() {
        assert_eq!(
            filter_transcription_output("um so I think the build is fine", "auto", &None),
            "so I think the build is fine"
        );
    }

    #[test]
    fn test_detect_filler_language_fails_closed() {
        assert_eq!(detect_filler_language("um okay"), None);
        assert_eq!(detect_filler_language("eu vi um carro"), None);
        assert_eq!(
            detect_filler_language("I think this is the one we want"),
            Some("en")
        );
        assert_eq!(
            detect_filler_language("ich glaube das ist nicht so einfach"),
            Some("de")
        );
    }
}
