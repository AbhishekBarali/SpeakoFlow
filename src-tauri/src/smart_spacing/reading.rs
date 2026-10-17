//! What a platform reader found around the insertion point.
//!
//! Kept free of any dependency so the platform readers and the rules can be
//! compiled and tested on their own.

/// How many characters before the insertion point a reader returns. The rules
/// need the last few characters and the current line; more would only cost a
/// bigger cross-process copy.
pub const BEFORE_CHARS: usize = 64;

/// How many characters after the insertion point a reader returns.
pub const AFTER_CHARS: usize = 32;

/// The text around the caret of the field a dictation is about to be pasted into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// The field could be read. `before` is the text before the insertion point
    /// (before the selection, when there is one), `after` the text after it, and
    /// `selected` whatever the paste is about to replace.
    Text {
        before: String,
        after: String,
        selected: String,
    },
    /// The field could be read and is empty.
    Empty,
    /// The field could not be read: the app hides its text, the read timed out,
    /// or the control is not a text field. Nothing is known about it.
    Unknown,
    /// A terminal. Its text is the shell's prompt and output, not the user's
    /// sentence, so it is never read.
    Terminal,
    /// A password field or secure input. Nothing is read and nothing is changed.
    Secret,
}

impl Reading {
    /// Short name for the log. Never includes any of the text.
    pub fn kind(&self) -> &'static str {
        match self {
            Reading::Text { .. } => "text",
            Reading::Empty => "empty",
            Reading::Unknown => "unknown",
            Reading::Terminal => "terminal",
            Reading::Secret => "secret",
        }
    }

    /// Whether this reading says something about the field. A later read that
    /// learned nothing must not replace an earlier one that did.
    pub fn is_informative(&self) -> bool {
        !matches!(self, Reading::Unknown)
    }
}

/// One reading, plus which window or app it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub reading: Reading,
    /// Identifies the window (Windows), app (macOS) or accessible app (Linux)
    /// that was read, so the last-paste memory only ever applies to the same
    /// place. `None` when not even that could be told.
    pub target: Option<String>,
    /// The field trims the spaces at the edges of pasted text (a Chromium
    /// address bar), so a space in front of a dictation must be typed.
    pub trims_pasted_spaces: bool,
}

impl Snapshot {
    pub fn unknown(target: Option<String>) -> Self {
        Self::new(Reading::Unknown, target)
    }

    pub fn new(reading: Reading, target: Option<String>) -> Self {
        Self {
            reading,
            target,
            trims_pasted_spaces: false,
        }
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn trimming_pasted_spaces(mut self, trims: bool) -> Self {
        self.trims_pasted_spaces = trims;
        self
    }
}

/// Normalise text a reader got from another app before the rules see it.
///
/// Line breaks come in several forms (`\r\n` from Win32 edits, a bare `\r` from
/// RichEdit paragraph marks, U+2028/U+2029, a Word manual line break U+000B),
/// and all of them mean the same thing to the rules. NUL and the invisible
/// marks some editors leave at the edges (zero-width space, BOM, direction
/// marks) would otherwise be taken for real characters.
pub fn normalize(text: &str) -> String {
    let unified = text.replace("\r\n", "\n");
    unified
        .chars()
        .filter_map(|c| match c {
            '\r' | '\u{000B}' | '\u{2028}' | '\u{2029}' => Some('\n'),
            '\0' | '\u{200B}' | '\u{200E}' | '\u{200F}' | '\u{2060}' | '\u{FEFF}' => None,
            other => Some(other),
        })
        .collect()
}

/// Keep at most the last `n` characters of `text`.
pub fn tail_chars(text: &str, n: usize) -> String {
    let count = text.chars().count();
    if count <= n {
        return text.to_string();
    }
    text.chars().skip(count - n).collect()
}

/// Keep at most the first `n` characters of `text`.
pub fn head_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// Decode UTF-16 read from the edge of a range.
///
/// Windows and macOS hand text over as UTF-16; AT-SPI on Linux does not. A range boundary can fall in
/// the middle of a surrogate pair, so a lone half at the outer edge is dropped
/// instead of being turned into a replacement character.
#[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
pub fn decode_utf16_edge(units: &[u16], keep_end: bool) -> String {
    let mut slice = units;
    if keep_end {
        // Text before the caret: the outer edge is the start.
        if let Some(first) = slice.first() {
            if (0xDC00..=0xDFFF).contains(first) {
                slice = &slice[1..];
            }
        }
    } else if let Some(last) = slice.last() {
        // Text after the caret: the outer edge is the end.
        if (0xD800..=0xDBFF).contains(last) {
            slice = &slice[..slice.len() - 1];
        }
    }
    String::from_utf16_lossy(slice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_breaks_are_unified() {
        assert_eq!(normalize("a\r\nb\rc\u{2029}d"), "a\nb\nc\nd");
    }

    #[test]
    fn invisible_marks_are_dropped() {
        assert_eq!(normalize("\u{FEFF}hi\u{200B}"), "hi");
    }

    #[test]
    fn a_split_surrogate_at_the_outer_edge_is_dropped() {
        // "x" + U+1F600 encoded, with the high half cut off at the start.
        let units = [0xDE00u16, b'o' as u16, b'k' as u16];
        assert_eq!(decode_utf16_edge(&units, true), "ok");
        let units = [b'o' as u16, b'k' as u16, 0xD83D];
        assert_eq!(decode_utf16_edge(&units, false), "ok");
    }

    #[test]
    fn tail_and_head_count_characters() {
        assert_eq!(tail_chars("h\u{00E9}llo", 3), "llo");
        assert_eq!(head_chars("h\u{00E9}llo", 2), "h\u{00E9}");
    }
}
