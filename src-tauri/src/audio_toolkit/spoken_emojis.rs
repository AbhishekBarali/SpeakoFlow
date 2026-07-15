//! Offline expansion of explicit spoken emoji commands.
//!
//! The grammar is deliberately narrow: a known name or alias followed by a
//! word close to "emoji" (for example, "happy emoji" or "thumbs up emoji").
//! Keeping the suffix mandatory prevents ordinary words such as "happy" or
//! "fire" from changing unexpectedly. Matching is deterministic and local.

use once_cell::sync::Lazy;
use regex::Regex;
use strsim::{damerau_levenshtein, normalized_damerau_levenshtein};

#[derive(Debug, Clone, Copy)]
struct Word<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

#[derive(Debug, Clone, Copy)]
struct EmojiAlias {
    phrase: &'static str,
    symbol: &'static str,
}

// Common conversational emoji first, with natural names people tend to say.
// A few frequent speech-to-text spellings (hapy/hart/fier) are explicit because
// very short words are intentionally excluded from fuzzy matching.
const ALIASES: &[EmojiAlias] = &[
    EmojiAlias {
        phrase: "smile",
        symbol: "😊",
    },
    EmojiAlias {
        phrase: "smiley",
        symbol: "😊",
    },
    EmojiAlias {
        phrase: "smiley face",
        symbol: "😊",
    },
    EmojiAlias {
        phrase: "smiling",
        symbol: "😊",
    },
    EmojiAlias {
        phrase: "smiling face",
        symbol: "😊",
    },
    EmojiAlias {
        phrase: "happy",
        symbol: "😊",
    },
    EmojiAlias {
        phrase: "hapy",
        symbol: "😊",
    },
    EmojiAlias {
        phrase: "happy face",
        symbol: "😊",
    },
    EmojiAlias {
        phrase: "grin",
        symbol: "😁",
    },
    EmojiAlias {
        phrase: "grinning",
        symbol: "😁",
    },
    EmojiAlias {
        phrase: "laugh",
        symbol: "😂",
    },
    EmojiAlias {
        phrase: "laughing",
        symbol: "😂",
    },
    EmojiAlias {
        phrase: "laughing face",
        symbol: "😂",
    },
    EmojiAlias {
        phrase: "tears of joy",
        symbol: "😂",
    },
    EmojiAlias {
        phrase: "joy",
        symbol: "😂",
    },
    EmojiAlias {
        phrase: "lol",
        symbol: "😂",
    },
    EmojiAlias {
        phrase: "rolling laughing",
        symbol: "🤣",
    },
    EmojiAlias {
        phrase: "rolling on the floor laughing",
        symbol: "🤣",
    },
    EmojiAlias {
        phrase: "rofl",
        symbol: "🤣",
    },
    EmojiAlias {
        phrase: "sad",
        symbol: "😢",
    },
    EmojiAlias {
        phrase: "sad face",
        symbol: "😢",
    },
    EmojiAlias {
        phrase: "tear",
        symbol: "😢",
    },
    EmojiAlias {
        phrase: "cry",
        symbol: "😭",
    },
    EmojiAlias {
        phrase: "crying",
        symbol: "😭",
    },
    EmojiAlias {
        phrase: "sobbing",
        symbol: "😭",
    },
    EmojiAlias {
        phrase: "loudly crying",
        symbol: "😭",
    },
    EmojiAlias {
        phrase: "heart",
        symbol: "❤️",
    },
    EmojiAlias {
        phrase: "hart",
        symbol: "❤️",
    },
    EmojiAlias {
        phrase: "red heart",
        symbol: "❤️",
    },
    EmojiAlias {
        phrase: "love",
        symbol: "❤️",
    },
    EmojiAlias {
        phrase: "love heart",
        symbol: "❤️",
    },
    EmojiAlias {
        phrase: "orange heart",
        symbol: "🧡",
    },
    EmojiAlias {
        phrase: "yellow heart",
        symbol: "💛",
    },
    EmojiAlias {
        phrase: "green heart",
        symbol: "💚",
    },
    EmojiAlias {
        phrase: "blue heart",
        symbol: "💙",
    },
    EmojiAlias {
        phrase: "purple heart",
        symbol: "💜",
    },
    EmojiAlias {
        phrase: "black heart",
        symbol: "🖤",
    },
    EmojiAlias {
        phrase: "white heart",
        symbol: "🤍",
    },
    EmojiAlias {
        phrase: "broken heart",
        symbol: "💔",
    },
    EmojiAlias {
        phrase: "heartbreak",
        symbol: "💔",
    },
    EmojiAlias {
        phrase: "heart eyes",
        symbol: "😍",
    },
    EmojiAlias {
        phrase: "love eyes",
        symbol: "😍",
    },
    EmojiAlias {
        phrase: "smiling hearts",
        symbol: "🥰",
    },
    EmojiAlias {
        phrase: "hearts face",
        symbol: "🥰",
    },
    EmojiAlias {
        phrase: "thumbs up",
        symbol: "👍",
    },
    EmojiAlias {
        phrase: "thumb up",
        symbol: "👍",
    },
    EmojiAlias {
        phrase: "like",
        symbol: "👍",
    },
    EmojiAlias {
        phrase: "thumbs down",
        symbol: "👎",
    },
    EmojiAlias {
        phrase: "thumb down",
        symbol: "👎",
    },
    EmojiAlias {
        phrase: "dislike",
        symbol: "👎",
    },
    EmojiAlias {
        phrase: "fire",
        symbol: "🔥",
    },
    EmojiAlias {
        phrase: "fier",
        symbol: "🔥",
    },
    EmojiAlias {
        phrase: "flame",
        symbol: "🔥",
    },
    EmojiAlias {
        phrase: "hot",
        symbol: "🔥",
    },
    EmojiAlias {
        phrase: "rocket",
        symbol: "🚀",
    },
    EmojiAlias {
        phrase: "launch",
        symbol: "🚀",
    },
    EmojiAlias {
        phrase: "party",
        symbol: "🎉",
    },
    EmojiAlias {
        phrase: "party popper",
        symbol: "🎉",
    },
    EmojiAlias {
        phrase: "celebration",
        symbol: "🎉",
    },
    EmojiAlias {
        phrase: "celebrate",
        symbol: "🎉",
    },
    EmojiAlias {
        phrase: "party face",
        symbol: "🥳",
    },
    EmojiAlias {
        phrase: "partying face",
        symbol: "🥳",
    },
    EmojiAlias {
        phrase: "thinking",
        symbol: "🤔",
    },
    EmojiAlias {
        phrase: "thinking face",
        symbol: "🤔",
    },
    EmojiAlias {
        phrase: "hmm",
        symbol: "🤔",
    },
    EmojiAlias {
        phrase: "wink",
        symbol: "😉",
    },
    EmojiAlias {
        phrase: "winking",
        symbol: "😉",
    },
    EmojiAlias {
        phrase: "winky",
        symbol: "😉",
    },
    EmojiAlias {
        phrase: "kiss",
        symbol: "😘",
    },
    EmojiAlias {
        phrase: "kissing",
        symbol: "😘",
    },
    EmojiAlias {
        phrase: "kiss face",
        symbol: "😘",
    },
    EmojiAlias {
        phrase: "cool",
        symbol: "😎",
    },
    EmojiAlias {
        phrase: "sunglasses",
        symbol: "😎",
    },
    EmojiAlias {
        phrase: "cool face",
        symbol: "😎",
    },
    EmojiAlias {
        phrase: "nervous laugh",
        symbol: "😅",
    },
    EmojiAlias {
        phrase: "sweat smile",
        symbol: "😅",
    },
    EmojiAlias {
        phrase: "surprised",
        symbol: "😮",
    },
    EmojiAlias {
        phrase: "surprise",
        symbol: "😮",
    },
    EmojiAlias {
        phrase: "shocked",
        symbol: "😮",
    },
    EmojiAlias {
        phrase: "wow",
        symbol: "😮",
    },
    EmojiAlias {
        phrase: "scream",
        symbol: "😱",
    },
    EmojiAlias {
        phrase: "screaming",
        symbol: "😱",
    },
    EmojiAlias {
        phrase: "scared",
        symbol: "😱",
    },
    EmojiAlias {
        phrase: "angry",
        symbol: "😠",
    },
    EmojiAlias {
        phrase: "angry face",
        symbol: "😠",
    },
    EmojiAlias {
        phrase: "mad",
        symbol: "😠",
    },
    EmojiAlias {
        phrase: "swearing",
        symbol: "🤬",
    },
    EmojiAlias {
        phrase: "cursing",
        symbol: "🤬",
    },
    EmojiAlias {
        phrase: "pleading",
        symbol: "🥺",
    },
    EmojiAlias {
        phrase: "puppy eyes",
        symbol: "🥺",
    },
    EmojiAlias {
        phrase: "sleeping",
        symbol: "😴",
    },
    EmojiAlias {
        phrase: "sleepy",
        symbol: "😴",
    },
    EmojiAlias {
        phrase: "sick",
        symbol: "🤢",
    },
    EmojiAlias {
        phrase: "nauseous",
        symbol: "🤢",
    },
    EmojiAlias {
        phrase: "mind blown",
        symbol: "🤯",
    },
    EmojiAlias {
        phrase: "exploding head",
        symbol: "🤯",
    },
    EmojiAlias {
        phrase: "shrug",
        symbol: "🤷",
    },
    EmojiAlias {
        phrase: "shrugging",
        symbol: "🤷",
    },
    EmojiAlias {
        phrase: "face palm",
        symbol: "🤦",
    },
    EmojiAlias {
        phrase: "facepalm",
        symbol: "🤦",
    },
    EmojiAlias {
        phrase: "eye roll",
        symbol: "🙄",
    },
    EmojiAlias {
        phrase: "rolling eyes",
        symbol: "🙄",
    },
    EmojiAlias {
        phrase: "neutral",
        symbol: "😐",
    },
    EmojiAlias {
        phrase: "straight face",
        symbol: "😐",
    },
    EmojiAlias {
        phrase: "confused",
        symbol: "😕",
    },
    EmojiAlias {
        phrase: "tongue",
        symbol: "😛",
    },
    EmojiAlias {
        phrase: "tongue out",
        symbol: "😛",
    },
    EmojiAlias {
        phrase: "hug",
        symbol: "🤗",
    },
    EmojiAlias {
        phrase: "hugging",
        symbol: "🤗",
    },
    EmojiAlias {
        phrase: "angel",
        symbol: "😇",
    },
    EmojiAlias {
        phrase: "halo",
        symbol: "😇",
    },
    EmojiAlias {
        phrase: "devil",
        symbol: "😈",
    },
    EmojiAlias {
        phrase: "poop",
        symbol: "💩",
    },
    EmojiAlias {
        phrase: "poo",
        symbol: "💩",
    },
    EmojiAlias {
        phrase: "skull",
        symbol: "💀",
    },
    EmojiAlias {
        phrase: "dead",
        symbol: "💀",
    },
    EmojiAlias {
        phrase: "eyes",
        symbol: "👀",
    },
    EmojiAlias {
        phrase: "looking",
        symbol: "👀",
    },
    EmojiAlias {
        phrase: "clap",
        symbol: "👏",
    },
    EmojiAlias {
        phrase: "clapping",
        symbol: "👏",
    },
    EmojiAlias {
        phrase: "applause",
        symbol: "👏",
    },
    EmojiAlias {
        phrase: "pray",
        symbol: "🙏",
    },
    EmojiAlias {
        phrase: "praying",
        symbol: "🙏",
    },
    EmojiAlias {
        phrase: "folded hands",
        symbol: "🙏",
    },
    EmojiAlias {
        phrase: "please",
        symbol: "🙏",
    },
    EmojiAlias {
        phrase: "raised hands",
        symbol: "🙌",
    },
    EmojiAlias {
        phrase: "praise",
        symbol: "🙌",
    },
    EmojiAlias {
        phrase: "hooray",
        symbol: "🙌",
    },
    EmojiAlias {
        phrase: "wave",
        symbol: "👋",
    },
    EmojiAlias {
        phrase: "waving",
        symbol: "👋",
    },
    EmojiAlias {
        phrase: "hello",
        symbol: "👋",
    },
    EmojiAlias {
        phrase: "goodbye",
        symbol: "👋",
    },
    EmojiAlias {
        phrase: "handshake",
        symbol: "🤝",
    },
    EmojiAlias {
        phrase: "deal",
        symbol: "🤝",
    },
    EmojiAlias {
        phrase: "okay hand",
        symbol: "👌",
    },
    EmojiAlias {
        phrase: "ok hand",
        symbol: "👌",
    },
    EmojiAlias {
        phrase: "okay",
        symbol: "👌",
    },
    EmojiAlias {
        phrase: "muscle",
        symbol: "💪",
    },
    EmojiAlias {
        phrase: "flex",
        symbol: "💪",
    },
    EmojiAlias {
        phrase: "strong",
        symbol: "💪",
    },
    EmojiAlias {
        phrase: "crossed fingers",
        symbol: "🤞",
    },
    EmojiAlias {
        phrase: "fingers crossed",
        symbol: "🤞",
    },
    EmojiAlias {
        phrase: "luck",
        symbol: "🤞",
    },
    EmojiAlias {
        phrase: "check",
        symbol: "✅",
    },
    EmojiAlias {
        phrase: "check mark",
        symbol: "✅",
    },
    EmojiAlias {
        phrase: "tick",
        symbol: "✅",
    },
    EmojiAlias {
        phrase: "done",
        symbol: "✅",
    },
    EmojiAlias {
        phrase: "cross",
        symbol: "❌",
    },
    EmojiAlias {
        phrase: "cross mark",
        symbol: "❌",
    },
    EmojiAlias {
        phrase: "x mark",
        symbol: "❌",
    },
    EmojiAlias {
        phrase: "wrong",
        symbol: "❌",
    },
    EmojiAlias {
        phrase: "warning",
        symbol: "⚠️",
    },
    EmojiAlias {
        phrase: "caution",
        symbol: "⚠️",
    },
    EmojiAlias {
        phrase: "hundred",
        symbol: "💯",
    },
    EmojiAlias {
        phrase: "one hundred",
        symbol: "💯",
    },
    EmojiAlias {
        phrase: "sparkle",
        symbol: "✨",
    },
    EmojiAlias {
        phrase: "sparkles",
        symbol: "✨",
    },
    EmojiAlias {
        phrase: "star",
        symbol: "⭐",
    },
    EmojiAlias {
        phrase: "light bulb",
        symbol: "💡",
    },
    EmojiAlias {
        phrase: "idea",
        symbol: "💡",
    },
    EmojiAlias {
        phrase: "pin",
        symbol: "📌",
    },
    EmojiAlias {
        phrase: "push pin",
        symbol: "📌",
    },
    EmojiAlias {
        phrase: "location pin",
        symbol: "📍",
    },
    EmojiAlias {
        phrase: "map pin",
        symbol: "📍",
    },
    EmojiAlias {
        phrase: "target",
        symbol: "🎯",
    },
    EmojiAlias {
        phrase: "bullseye",
        symbol: "🎯",
    },
    EmojiAlias {
        phrase: "trophy",
        symbol: "🏆",
    },
    EmojiAlias {
        phrase: "winner",
        symbol: "🏆",
    },
    EmojiAlias {
        phrase: "cake",
        symbol: "🎂",
    },
    EmojiAlias {
        phrase: "birthday cake",
        symbol: "🎂",
    },
    EmojiAlias {
        phrase: "gift",
        symbol: "🎁",
    },
    EmojiAlias {
        phrase: "present",
        symbol: "🎁",
    },
    EmojiAlias {
        phrase: "rose",
        symbol: "🌹",
    },
    EmojiAlias {
        phrase: "rainbow",
        symbol: "🌈",
    },
    EmojiAlias {
        phrase: "sun",
        symbol: "☀️",
    },
    EmojiAlias {
        phrase: "sunny",
        symbol: "☀️",
    },
    EmojiAlias {
        phrase: "moon",
        symbol: "🌙",
    },
    EmojiAlias {
        phrase: "coffee",
        symbol: "☕",
    },
    EmojiAlias {
        phrase: "beer",
        symbol: "🍺",
    },
    EmojiAlias {
        phrase: "pizza",
        symbol: "🍕",
    },
    EmojiAlias {
        phrase: "siren",
        symbol: "🚨",
    },
    EmojiAlias {
        phrase: "alarm",
        symbol: "🚨",
    },
];

const MAX_ALIAS_WORDS: usize = 5;

static WORD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?u)[\p{L}\p{N}]+(?:['’][\p{L}\p{N}]+)*")
        .expect("spoken emoji word regex must compile")
});

/// Expands recognized spoken emoji commands while preserving all surrounding
/// punctuation and spacing. Unknown names are returned unchanged.
pub fn expand_spoken_emojis(text: &str) -> String {
    let words: Vec<Word<'_>> = WORD_RE
        .find_iter(text)
        .map(|found| Word {
            start: found.start(),
            end: found.end(),
            text: found.as_str(),
        })
        .collect();

    let mut output = String::with_capacity(text.len());
    let mut copied_until = 0;
    let mut changed = false;

    for (emoji_index, word) in words.iter().enumerate() {
        if word.start < copied_until || !is_emoji_keyword(word.text) {
            continue;
        }

        let Some((alias_start, symbol)) =
            find_alias_before(text, &words, emoji_index, copied_until)
        else {
            continue;
        };

        output.push_str(&text[copied_until..alias_start]);
        output.push_str(symbol);
        copied_until = word.end;
        changed = true;
    }

    if !changed {
        return text.to_string();
    }

    output.push_str(&text[copied_until..]);
    output
}

fn is_emoji_keyword(word: &str) -> bool {
    let normalized = word.to_lowercase();
    let char_count = normalized.chars().count();
    (4..=7).contains(&char_count) && damerau_levenshtein(&normalized, "emoji") <= 1
}

fn find_alias_before<'a>(
    text: &str,
    words: &[Word<'a>],
    emoji_index: usize,
    copied_until: usize,
) -> Option<(usize, &'static str)> {
    if emoji_index == 0 {
        return None;
    }

    let max_words = emoji_index.min(MAX_ALIAS_WORDS);

    // Exact aliases are authoritative, and longer phrases win over their
    // shorter suffixes ("red heart" before "heart").
    for word_count in (1..=max_words).rev() {
        let start_index = emoji_index - word_count;
        if words[start_index].start < copied_until
            || !has_soft_separators(text, &words[start_index..=emoji_index])
        {
            continue;
        }

        let candidate = normalized_phrase(&words[start_index..emoji_index]);
        if let Some(alias) = ALIASES.iter().find(|alias| alias.phrase == candidate) {
            return Some((words[start_index].start, alias.symbol));
        }
    }

    // Fuzzy matching is intentionally conservative: short words are too easy
    // to confuse ("bad" vs "sad"), edits are capped, and an ambiguous result
    // is rejected. This catches typical ASR forms like "smily", "thumps up",
    // and transposed letters in "rokcet" without guessing unknown commands.
    let mut best: Option<(usize, &'static str, f64)> = None;
    let mut second_best_other_symbol = 0.0_f64;

    for word_count in 1..=max_words {
        let start_index = emoji_index - word_count;
        if words[start_index].start < copied_until
            || !has_soft_separators(text, &words[start_index..=emoji_index])
        {
            continue;
        }

        let candidate = normalized_phrase(&words[start_index..emoji_index]);
        let compact_len = candidate.chars().filter(|c| !c.is_whitespace()).count();
        if compact_len < 5 {
            continue;
        }

        let max_edits = if compact_len >= 12 { 2 } else { 1 };
        for alias in ALIASES
            .iter()
            .filter(|alias| alias.phrase.split_whitespace().count() == word_count)
        {
            if damerau_levenshtein(&candidate, alias.phrase) > max_edits {
                continue;
            }
            let score = normalized_damerau_levenshtein(&candidate, alias.phrase);
            if score < 0.80 {
                continue;
            }

            match best {
                None => best = Some((start_index, alias.symbol, score)),
                Some((_, best_symbol, best_score)) if alias.symbol == best_symbol => {
                    if score > best_score {
                        best = Some((start_index, alias.symbol, score));
                    }
                }
                Some((_, _, best_score)) if score > best_score => {
                    second_best_other_symbol = second_best_other_symbol.max(best_score);
                    best = Some((start_index, alias.symbol, score));
                }
                Some(_) => {
                    second_best_other_symbol = second_best_other_symbol.max(score);
                }
            }
        }
    }

    best.and_then(|(start_index, symbol, score)| {
        let is_unambiguous =
            second_best_other_symbol == 0.0 || score - second_best_other_symbol >= 0.08;
        is_unambiguous.then_some((words[start_index].start, symbol))
    })
}

fn normalized_phrase(words: &[Word<'_>]) -> String {
    words
        .iter()
        .map(|word| word.text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_soft_separators(text: &str, words: &[Word<'_>]) -> bool {
    words.windows(2).all(|pair| {
        text[pair[0].end..pair[1].start]
            .chars()
            .all(|c| c.is_whitespace() || matches!(c, '-' | ','))
    })
}

#[cfg(test)]
mod tests {
    use super::expand_spoken_emojis;

    #[test]
    fn expands_common_and_multiple_commands() {
        assert_eq!(
            expand_spoken_emojis("That worked fire emoji rocket emoji!"),
            "That worked 🔥 🚀!"
        );
    }

    #[test]
    fn uses_longest_exact_alias_and_preserves_punctuation() {
        assert_eq!(
            expand_spoken_emojis("Sending a red heart emoji, not a broken heart emoji."),
            "Sending a ❤️, not a 💔."
        );
    }

    #[test]
    fn is_case_insensitive_and_accepts_soft_separators() {
        assert_eq!(
            expand_spoken_emojis("HAPPY, EMOJI and thumbs-up emoji"),
            "😊 and 👍"
        );
    }

    #[test]
    fn tolerates_minor_alias_and_keyword_transcription_errors() {
        assert_eq!(
            expand_spoken_emojis("smily emogi thumps up emoji rokcet emojy"),
            "😊 👍 🚀"
        );
    }

    #[test]
    fn recognizes_natural_aliases() {
        assert_eq!(
            expand_spoken_emojis("puppy eyes emoji facepalm emoji idea emoji"),
            "🥺 🤦 💡"
        );
    }

    #[test]
    fn leaves_unknown_or_ambiguous_names_unchanged() {
        let text = "bad emoji and custom mascot emoji";
        assert_eq!(expand_spoken_emojis(text), text);
    }

    #[test]
    fn does_not_cross_sentence_boundaries() {
        let text = "I feel happy. Emoji are useful.";
        assert_eq!(expand_spoken_emojis(text), text);
    }

    #[test]
    fn leaves_non_command_words_untouched() {
        let text = "The fire alarm has an emojiology label.";
        assert_eq!(expand_spoken_emojis(text), text);
    }

    #[test]
    fn keeps_words_before_a_short_alias() {
        assert_eq!(
            expand_spoken_emojis("I am very happy emoji"),
            "I am very 😊"
        );
    }
}
