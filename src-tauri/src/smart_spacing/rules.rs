//! The rules that fit a dictation onto the text already in the field.
//!
//! Everything here is pure: text in, text out. The platform readers decide what
//! the field holds; this module decides what to paste. Only the inserted text is
//! ever shaped. The user's own text is never touched, with one guarded
//! exception: a period SpeakoFlow itself pasted a moment ago, at the end of an
//! unfinished sentence ("I went to the."), which a continuing dictation removes
//! with one Backspace.
//!
//! The rules follow what the established tools converged on (TypeWhisper,
//! OpenSuperWhisper, Talon, Spokenly, Wispr Flow), plus their bug histories:
//! no trailing space unless a word follows the caret, nothing changed when the
//! field could not be read, no spaces in scripts written without them, and no
//! lowercasing of names, acronyms, "I" or German nouns.

/// What is known about the field the dictation goes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context<'a> {
    /// The field was read. `before` is the text before the insertion point (or
    /// before the selection), `after` the text after it, `selected` what the
    /// paste replaces.
    Field {
        before: &'a str,
        after: &'a str,
        selected: &'a str,
    },
    /// The field was read and is empty.
    EmptyField,
    /// The field could not be read, but SpeakoFlow pasted `last` into the same
    /// place moments ago. Only a leading space may be added on this basis:
    /// anything more would be a guess about text nobody has seen.
    Remembered { last: &'a str },
    /// Nothing is known. The dictation is pasted exactly as transcribed.
    Nothing,
}

/// The user's choices and what the caller knows about the previous paste.
#[derive(Debug, Clone, Copy)]
pub struct Options<'a> {
    /// Lowercase the first word when the dictation continues a sentence (and
    /// match the case of a replaced selection).
    pub fix_case: bool,
    /// Remove SpeakoFlow's own stray period when a dictation continues an
    /// unfinished sentence. Requires `previous_is_ours`.
    pub join_sentences: bool,
    /// The caller verified that the text before the caret ends with exactly
    /// what SpeakoFlow pasted last, into this same field, moments ago.
    pub previous_is_ours: bool,
    /// The dictation language as configured (`"auto"`, `"en"`, `"de"`, ...).
    pub language: &'a str,
    /// The user's custom words. A word listed here with a capital is a name and
    /// is never lowercased.
    pub dictionary: &'a [String],
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            fix_case: true,
            join_sentences: true,
            previous_is_ours: false,
            language: "auto",
            dictionary: &[],
        }
    }
}

/// What to paste, and how many characters to delete before the cursor first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fitted {
    pub text: String,
    /// Characters to remove with Backspace before pasting. Only ever 0 or 1:
    /// the stray period SpeakoFlow itself pasted.
    pub erase_before: usize,
    /// Whether anything was known about the field. When not, the text is the
    /// transcript unchanged and the paste behaves as if Smart spacing were off.
    pub known: bool,
    pub decision: Decision,
}

/// What the rules did, for the log. Carries no text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Decision {
    pub leading_space: bool,
    pub trailing_space: bool,
    pub lowercased: bool,
    pub matched_selection_case: bool,
    pub dropped_final_mark: bool,
    pub erased_own_period: bool,
}

/// Fit `transcript` onto the field described by `context`.
pub fn fit(transcript: &str, context: Context<'_>, options: &Options<'_>) -> Fitted {
    let mut fitted = fit_known(transcript, context, options);
    fitted.known = !matches!(context, Context::Nothing);
    fitted
}

/// Whether the text before the caret ends with exactly what SpeakoFlow pasted
/// last: the caret is still right after it, and nobody has typed since.
pub fn ends_with_pasted(before: &str, last: &str) -> bool {
    let last = last.trim();
    let wanted = last.chars().count();
    if wanted == 0 {
        return false;
    }
    let available = before.chars().count();
    // Compare as much of it as the reader returned, but never so little that a
    // coincidence could pass for it.
    let n = wanted.min(available);
    if n < wanted.min(8) {
        return false;
    }
    let tail: String = last.chars().skip(wanted - n).collect();
    before.ends_with(&tail)
}

fn fit_known(transcript: &str, context: Context<'_>, options: &Options<'_>) -> Fitted {
    let unchanged = || Fitted {
        text: transcript.to_string(),
        erase_before: 0,
        known: false,
        decision: Decision::default(),
    };

    let core = trim_hspace(transcript);
    if core.is_empty() {
        return unchanged();
    }

    match context {
        Context::Nothing => unchanged(),
        Context::Remembered { last } => {
            let before: String = last.chars().map(object_to_newline).collect();
            let mut decision = Decision::default();
            let mut text = core.to_string();
            if needs_leading_space(&before, &text) {
                text.insert(0, ' ');
                decision.leading_space = true;
            }
            Fitted {
                text,
                erase_before: 0,
                known: true,
                decision,
            }
        }
        Context::EmptyField => {
            let mut decision = Decision::default();
            let text = match strip_standalone_value_period(core) {
                Some(stripped) => {
                    decision.dropped_final_mark = true;
                    stripped
                }
                None => core.to_string(),
            };
            Fitted {
                text,
                erase_before: 0,
                known: true,
                decision,
            }
        }
        Context::Field {
            before,
            after,
            selected,
        } => fit_field(core, before, after, selected, options),
    }
}

fn fit_field(
    core: &str,
    before: &str,
    after: &str,
    selected: &str,
    options: &Options<'_>,
) -> Fitted {
    // An embedded object (image, mention pill, a paragraph placeholder in a
    // Chromium editor) reads as U+FFFC. On the left it behaves like a line
    // start: no space, no case change. On the right, like a line end.
    let mut before: String = before.chars().map(object_to_newline).collect();
    let after: String = after.chars().map(object_to_newline).collect();
    let mut decision = Decision::default();
    let mut erase_before = 0;

    // Our own stray period, from the previous dictation into this same field.
    if options.join_sentences
        && options.previous_is_ours
        && selected.is_empty()
        && own_period_continues(&before, core, options.language)
    {
        before.pop();
        erase_before = 1;
        decision.erased_own_period = true;
    }

    let state = sentence_state(&before);
    let mut text = core.to_string();

    if options.fix_case {
        if let Some(first) = selected.chars().find(|c| c.is_alphabetic()) {
            if is_cased_script(first) {
                if let Some(matched) = match_case(&text, first.is_uppercase()) {
                    decision.matched_selection_case = matched != text;
                    text = matched;
                }
            }
        } else if state == SentenceState::Mid
            && lowercasing_allowed(options.language, &before, &text)
        {
            if let Some(lowered) = lowercase_first_word(&text, &before, &after, options.dictionary)
            {
                text = lowered;
                decision.lowercased = true;
            }
        }
    }

    if continues_sentence(&after) {
        if let Some(stripped) = drop_final_mark(&text) {
            text = stripped;
            decision.dropped_final_mark = true;
        }
    }

    if needs_leading_space(&before, &text) {
        text.insert(0, ' ');
        decision.leading_space = true;
    }
    if needs_trailing_space(&text, &after) {
        text.push(' ');
        decision.trailing_space = true;
    }

    Fitted {
        text,
        erase_before,
        known: true,
        decision,
    }
}

/* ───────────────────────────── character classes ───────────────────────────── */

fn object_to_newline(c: char) -> char {
    if c == '\u{FFFC}' {
        '\n'
    } else {
        c
    }
}

/// Horizontal whitespace: everything that separates words on one line.
fn is_hspace(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\u{00A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

fn is_space(c: char) -> bool {
    is_hspace(c) || c == '\n'
}

fn trim_hspace(text: &str) -> &str {
    text.trim_matches(is_hspace)
}

/// Marks that open something; no space follows them.
fn is_opener(c: char) -> bool {
    matches!(
        c,
        '(' | '['
            | '{'
            | '\u{201C}'
            | '\u{2018}'
            | '\u{00AB}'
            | '\u{2039}'
            | '\u{201E}'
            | '\u{201A}'
            | '\u{00BF}'
            | '\u{00A1}'
            | '\u{300C}'
            | '\u{300E}'
            | '\u{3010}'
            | '\u{3008}'
            | '\u{300A}'
            | '\u{3014}'
            | '\u{FF08}'
            | '\u{FF3B}'
            | '\u{FF5B}'
    )
}

/// Marks that close something; looked through when deciding whether a
/// sentence ended ("He said \"stop.\"" ended).
fn is_closer(c: char) -> bool {
    matches!(
        c,
        ')' | ']'
            | '}'
            | '"'
            | '\''
            | '\u{201D}'
            | '\u{2019}'
            | '\u{00BB}'
            | '\u{203A}'
            | '\u{300D}'
            | '\u{300F}'
            | '\u{3011}'
            | '\u{FF09}'
            | '\u{FF3D}'
            | '\u{FF5D}'
    )
}

fn is_currency(c: char) -> bool {
    matches!(
        c,
        '$' | '\u{00A3}'
            | '\u{20AC}'
            | '\u{00A5}'
            | '\u{20A9}'
            | '\u{20BD}'
            | '\u{20B9}'
            | '\u{20A8}'
    )
}

/// Characters that hang on the previous word: no space goes before them.
fn is_hanger(c: char) -> bool {
    matches!(
        c,
        '.' | ','
            | ';'
            | ':'
            | '!'
            | '?'
            | '%'
            | ')'
            | ']'
            | '}'
            | '\u{2026}'
            | '\u{2030}'
            | '\u{201D}'
            | '\u{2019}'
            | '\u{00BB}'
            | '\u{203A}'
            | '\u{0964}'
            | '\u{0965}'
            | '\u{060C}'
            | '\u{061B}'
            | '\u{061F}'
            | '\u{06D4}'
            | '\u{0589}'
            | '\u{1361}'..='\u{1368}'
            | '\u{3000}'..='\u{303F}'
            | '\u{FF01}'..='\u{FF0F}'
            | '\u{FF1A}'..='\u{FF20}'
            | '\u{FF3D}'
            | '\u{FF5D}'
            | '\u{FF61}'..='\u{FF65}'
    )
}

/// Scripts written without spaces between words. No ASCII space is ever put
/// next to them. Hangul is deliberately absent: Korean separates words with
/// spaces.
fn is_spaceless(c: char) -> bool {
    matches!(
        c,
        '\u{0E00}'..='\u{0E7F}'   // Thai
            | '\u{0E80}'..='\u{0EFF}' // Lao
            | '\u{0F00}'..='\u{0FFF}' // Tibetan
            | '\u{1000}'..='\u{109F}' // Myanmar
            | '\u{1780}'..='\u{17FF}' // Khmer
            | '\u{1950}'..='\u{197F}' // Tai Le
            | '\u{1980}'..='\u{19DF}' // New Tai Lue
            | '\u{19E0}'..='\u{19FF}' // Khmer symbols
            | '\u{1A20}'..='\u{1AAF}' // Tai Tham
            | '\u{1B00}'..='\u{1B7F}' // Balinese
            | '\u{1BC0}'..='\u{1BFF}' // Batak
            | '\u{2E80}'..='\u{2FDF}' // CJK radicals
            | '\u{3000}'..='\u{303F}' // CJK punctuation
            | '\u{3040}'..='\u{309F}' // Hiragana
            | '\u{30A0}'..='\u{30FF}' // Katakana
            | '\u{3100}'..='\u{312F}' // Bopomofo
            | '\u{31A0}'..='\u{31BF}' // Bopomofo extended
            | '\u{31F0}'..='\u{31FF}' // Katakana extensions
            | '\u{3400}'..='\u{4DBF}' // CJK extension A
            | '\u{4E00}'..='\u{9FFF}' // CJK unified ideographs
            | '\u{A500}'..='\u{A63F}' // Vai
            | '\u{A980}'..='\u{A9DF}' // Javanese
            | '\u{A9E0}'..='\u{A9FF}' // Myanmar extended B
            | '\u{AA00}'..='\u{AA5F}' // Cham
            | '\u{AA60}'..='\u{AA7F}' // Myanmar extended A
            | '\u{AA80}'..='\u{AADF}' // Tai Viet
            | '\u{F900}'..='\u{FAFF}' // CJK compatibility ideographs
            | '\u{FE10}'..='\u{FE1F}' // vertical forms
            | '\u{FE30}'..='\u{FE4F}' // CJK compatibility forms
            | '\u{FF00}'..='\u{FFEF}' // fullwidth and halfwidth forms
            | '\u{20000}'..='\u{3134F}' // CJK extensions B and later
    )
}

/// Letters whose case a sentence start or a continuation changes. Georgian and
/// Cherokee are left out on purpose: their case mappings produce forms nobody
/// writes in running text.
fn is_cased_script(c: char) -> bool {
    matches!(
        c,
        'A'..='Z'
            | 'a'..='z'
            | '\u{00C0}'..='\u{024F}' // Latin-1 letters, Latin extended A and B
            | '\u{1E00}'..='\u{1EFF}' // Latin extended additional
            | '\u{0370}'..='\u{03FF}' // Greek
            | '\u{1F00}'..='\u{1FFF}' // Greek extended
            | '\u{0400}'..='\u{052F}' // Cyrillic
            | '\u{1C80}'..='\u{1C8F}'
            | '\u{0531}'..='\u{058F}' // Armenian
    ) && c != '\u{00D7}'
        && c != '\u{00F7}'
}

fn is_greek(c: char) -> bool {
    matches!(c, '\u{0370}'..='\u{03FF}' | '\u{1F00}'..='\u{1FFF}')
}

/// Characters that end a sentence, in the scripts SpeakoFlow transcribes.
fn is_terminator(c: char) -> bool {
    matches!(
        c,
        '.' | '!'
            | '?'
            | '\u{0589}'               // Armenian full stop
            | '\u{061D}'..='\u{061F}' // Arabic end of text, question mark
            | '\u{06D4}'              // Arabic / Urdu full stop
            | '\u{0700}'..='\u{0702}' // Syriac
            | '\u{07F9}'              // N'Ko
            | '\u{0964}'..='\u{0965}' // Devanagari danda (Nepali, Hindi, ...)
            | '\u{104A}'..='\u{104B}' // Myanmar
            | '\u{1362}'
            | '\u{1367}'..='\u{1368}' // Ethiopic
            | '\u{166E}'
            | '\u{17D4}'..='\u{17D5}' // Khmer
            | '\u{1803}'
            | '\u{1809}'              // Mongolian
            | '\u{1944}'..='\u{1945}' // Limbu
            | '\u{203C}'..='\u{203D}'
            | '\u{2047}'..='\u{2049}'
            | '\u{2E2E}'
            | '\u{3002}'              // ideographic full stop
            | '\u{A4FF}'
            | '\u{A60E}'..='\u{A60F}'
            | '\u{A6F3}'
            | '\u{A6F7}'
            | '\u{A9C8}'..='\u{A9C9}'
            | '\u{FE12}'
            | '\u{FE15}'..='\u{FE16}'
            | '\u{FE52}'
            | '\u{FE56}'..='\u{FE57}'
            | '\u{FF01}'
            | '\u{FF0E}'
            | '\u{FF1F}'
            | '\u{FF61}'
            | '\u{2024}'
    )
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/* ───────────────────────────── leading and trailing space ───────────────────────────── */

const ELISIONS: &[&str] = &[
    "l", "d", "j", "m", "n", "s", "t", "c", "qu", "jusqu", "lorsqu", "puisqu", "quoiqu", "presqu",
    "dell", "all", "nell", "sull", "dall", "un",
];

/// Whether a straight quote at the end of `before` opens a quotation.
fn straight_quote_opens(before: &str) -> bool {
    let mut chars = before.chars().rev();
    let _quote = chars.next();
    match chars.next() {
        None => true,
        Some(c) => {
            is_space(c) || is_opener(c) || c == '-' || c == '"' || c == '\'' || c == ':' || c == '='
        }
    }
}

/// Whether `before` ends with a French or Italian elision such as "l'".
fn ends_with_elision(before: &str) -> bool {
    let Some(quote) = before.chars().last() else {
        return false;
    };
    if quote != '\'' && quote != '\u{2019}' {
        return false;
    }
    let body = &before[..before.len() - quote.len_utf8()];
    let word: String = body
        .chars()
        .rev()
        .take_while(|c| c.is_alphabetic())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if word.is_empty() {
        return false;
    }
    let preceded_by_word = body[..body.len() - word.len()]
        .chars()
        .last()
        .is_some_and(is_word_char);
    !preceded_by_word && ELISIONS.contains(&word.to_lowercase().as_str())
}

/// The last line of `before`, when it holds only a list, heading, task or quote
/// marker, says how the next text starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    /// "- ", "1. ", "## ", "> ", "- [ ] " and the like: the item starts here.
    WithSpace,
    /// "-", "*", "+" alone, without its space yet.
    Bare,
}

fn marker_line(before: &str) -> Option<Marker> {
    let line = before.rsplit('\n').next().unwrap_or(before);
    if line.chars().count() > 24 || line.trim().is_empty() {
        return None;
    }
    if matches!(line.trim_start(), "-" | "*" | "+" | "\u{2022}") {
        return Some(Marker::Bare);
    }
    if !line.ends_with(' ') {
        return None;
    }
    let mut rest = line.trim_start();
    let mut found = false;
    while !rest.is_empty() {
        let token_end = rest.find(' ').unwrap_or(rest.len());
        let token = &rest[..token_end];
        let is_marker =
            matches!(
                token,
                "-" | "*" | "+" | "\u{2022}" | ">" | "[" | "]" | "[ ]" | "[x]" | "[X]"
            ) || (!token.is_empty() && token.len() <= 6 && token.chars().all(|c| c == '#'))
                || (token.len() >= 2
                // The last character is checked first: it is ASCII, so slicing
                // it off cannot split a multi-byte character.
                && matches!(token.chars().last(), Some('.') | Some(')'))
                && token[..token.len() - 1].chars().all(|c| c.is_ascii_digit()));
        if !is_marker {
            return None;
        }
        found = true;
        rest = rest[token_end..].trim_start();
    }
    found.then_some(Marker::WithSpace)
}

fn needs_leading_space(before: &str, text: &str) -> bool {
    let Some(last) = before.chars().last() else {
        return false;
    };
    let Some(first) = text.chars().next() else {
        return false;
    };
    if is_space(last) || is_space(first) {
        return false;
    }
    if is_spaceless(last) || is_spaceless(first) {
        return false;
    }
    if marker_line(before) == Some(Marker::Bare) {
        return true;
    }

    // What sits at the end of the field.
    if is_opener(last) {
        return false;
    }
    if (last == '"' || last == '\'') && straight_quote_opens(before) {
        return false;
    }
    if ends_with_elision(before) {
        return false;
    }
    if matches!(last, '-' | '\u{2013}' | '\u{2014}') {
        // A dash with a space in front is a separator; one without is a hyphen.
        let spaced = before.chars().rev().nth(1).is_some_and(is_hspace);
        if !spaced {
            return false;
        }
    }
    if matches!(last, '@' | '#' | '/' | '\\' | '_') {
        return false;
    }
    if is_currency(last) && !before.chars().rev().nth(1).is_some_and(is_word_char) {
        return false;
    }

    // What the dictation starts with.
    if is_hanger(first) {
        let mut rest = text.chars().skip(1);
        // ":-)" and ";-)" are smileys, not punctuation.
        let smiley = matches!(first, ':' | ';')
            && rest.next() == Some('-')
            && matches!(rest.next(), Some(')') | Some('('));
        if !smiley {
            return false;
        }
    }
    if first == '"' || first == '\'' {
        let next = text.chars().nth(1);
        let closes = match next {
            None => true,
            Some(c) => is_space(c) || is_hanger(c) || is_closer(c),
        };
        if closes {
            return false;
        }
        // Possessive: "'s" hangs on the previous word.
        if first == '\''
            && matches!(next, Some('s') | Some('S'))
            && !text.chars().nth(2).is_some_and(is_word_char)
        {
            return false;
        }
    }
    if first == '-' && text.chars().nth(1).is_some_and(|c| c.is_alphanumeric()) {
        return false;
    }

    // Numbers and addresses that continue across the seam.
    if first.is_ascii_digit() && matches!(last, '.' | ',' | ':') {
        if before
            .chars()
            .rev()
            .nth(1)
            .is_some_and(|c| c.is_ascii_digit())
        {
            return false;
        }
    }
    let last_token = before.rsplit(is_space).next().unwrap_or("");
    if (last_token.contains("://") || last_token.contains('@')) && !text.chars().any(is_space) {
        if matches!(last, '.' | '/' | ':' | '?' | '=' | '&' | '#') {
            return false;
        }
    }
    true
}

fn needs_trailing_space(text: &str, after: &str) -> bool {
    let Some(next) = after.chars().next() else {
        return false;
    };
    let Some(last) = text.chars().last() else {
        return false;
    };
    if is_space(last) || is_space(next) {
        return false;
    }
    if is_spaceless(last) || is_spaceless(next) {
        return false;
    }
    if is_opener(last) || matches!(last, '@' | '#' | '/' | '\\' | '_' | '-') {
        return false;
    }
    let next_opens = is_opener(next)
        || ((next == '"' || next == '\'')
            && after.chars().nth(1).is_some_and(|c| c.is_alphanumeric()));
    next.is_alphanumeric() || next_opens
}

/* ───────────────────────────── sentences and case ───────────────────────────── */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SentenceState {
    /// A new sentence starts at the caret: the dictation keeps its capital.
    Start,
    /// After an ellipsis or a title ("Dr."): what follows is often a name, so
    /// the case is left alone.
    Neutral,
    /// The caret is inside a sentence.
    Mid,
}

const NON_FINAL_ABBREVIATIONS: &[&str] = &[
    "e.g.", "i.e.", "cf.", "vs.", "viz.", "approx.", "ca.", "bzw.", "z.b.", "d.h.",
];
const TITLES: &[&str] = &[
    "mr.", "mrs.", "ms.", "mx.", "dr.", "prof.", "st.", "sr.", "jr.", "mt.", "rev.", "capt.",
    "lt.", "col.", "sgt.", "gen.", "gov.", "sen.", "rep.", "mme.", "mlle.", "mgr.", "pr.", "hr.",
    "fr.", "nr.", "no.", "etc.",
];

fn last_word(text: &str) -> &str {
    text.rsplit(|c: char| is_space(c) || is_opener(c))
        .next()
        .unwrap_or("")
}

fn sentence_state(before: &str) -> SentenceState {
    if before.is_empty() {
        return SentenceState::Start;
    }
    if marker_line(before).is_some() {
        return SentenceState::Start;
    }
    let trimmed = before.trim_end_matches(is_hspace);
    if trimmed.is_empty() || trimmed.ends_with('\n') {
        return SentenceState::Start;
    }
    // Look through closing quotes and brackets, and opening ones ("Hola. \u{BF}").
    let core = trimmed.trim_end_matches(|c: char| is_closer(c) || is_opener(c) || is_hspace(c));
    if core.is_empty() {
        return SentenceState::Start;
    }
    if core.ends_with('\n') {
        return SentenceState::Start;
    }
    if core.ends_with("...") || core.ends_with('\u{2026}') {
        return SentenceState::Neutral;
    }
    let last = core.chars().last().unwrap_or(' ');
    let word = last_word(core).to_lowercase();
    if last == '.' {
        if NON_FINAL_ABBREVIATIONS.contains(&word.as_str()) {
            return SentenceState::Mid;
        }
        if TITLES.contains(&word.as_str()) {
            return SentenceState::Neutral;
        }
        // A single-letter initial: "J." before a surname.
        let letters: Vec<char> = word.trim_end_matches('.').chars().collect();
        if letters.len() == 1 && letters[0].is_alphabetic() {
            return SentenceState::Neutral;
        }
        return SentenceState::Start;
    }
    if is_terminator(last) {
        return SentenceState::Start;
    }
    // The Greek question mark is normalised to ';'.
    if last == ';' && core.chars().rev().nth(1).is_some_and(is_greek) {
        return SentenceState::Start;
    }
    // Only a word, a number or sentence-internal punctuation says the caret is
    // inside a sentence. After an emoji, a symbol or a percent sign it may as
    // well be a new one, so the case is left alone there.
    if last.is_alphanumeric()
        || matches!(
            last,
            ',' | ';' | ':' | '-' | '\u{2013}' | '\u{2014}' | '\'' | '\u{2019}'
        )
    {
        return SentenceState::Mid;
    }
    SentenceState::Neutral
}

/// Languages whose own rules make lowercasing a continuation wrong: German
/// capitalises every noun, and Turkish and Azeri have a dotted and a dotless I.
fn lowercasing_allowed(language: &str, before: &str, text: &str) -> bool {
    let base = language
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(base.as_str(), "de" | "lb" | "gsw" | "tr" | "az") {
        return false;
    }
    if base == "auto" || base.is_empty() {
        // Without a set language, look for signs of German or Turkish around the
        // caret rather than lowercase a German noun.
        let sample = format!(" {} {} ", before.to_lowercase(), text.to_lowercase());
        if sample.chars().any(|c| {
            matches!(
                c,
                '\u{00E4}'
                    | '\u{00F6}'
                    | '\u{00FC}'
                    | '\u{00DF}'
                    | '\u{0131}'
                    | '\u{015F}'
                    | '\u{011F}'
            )
        }) {
            return false;
        }
        const GERMAN: &[&str] = &[
            " der ", " die ", " das ", " und ", " ich ", " nicht ", " ist ", " ein ", " eine ",
            " mit ", " den ", " dem ",
        ];
        if GERMAN.iter().any(|w| sample.contains(w)) {
            return false;
        }
    }
    true
}

/// Split off the first word, after a leading inverted mark.
fn first_word(text: &str) -> (usize, &str) {
    let start = text
        .char_indices()
        .find(|(_, c)| !matches!(c, '\u{00BF}' | '\u{00A1}'))
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let rest = &text[start..];
    let end = rest.find(is_space).unwrap_or(rest.len());
    (start, &rest[..end])
}

/// "Paris", "Monday", "Marie", "A": capitalised, then lowercase letters only.
fn is_titlecase(word: &str) -> bool {
    let letters: Vec<char> = word.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() || !letters[0].is_uppercase() {
        return false;
    }
    letters[1..].iter().all(|c| c.is_lowercase())
}

fn strip_word_punct(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric())
}

/// Words written with a capital in the middle of a sentence around the caret
/// or in the dictation: names, as far as anything here can tell.
fn names_in(texts: &[&str]) -> Vec<String> {
    let mut names = Vec::new();
    for text in texts {
        let mut previous: Option<char> = None;
        for word in text.split(is_space) {
            let stripped = strip_word_punct(word);
            let sentence_start = match previous {
                None => true,
                Some(c) => is_terminator(c) || c == '\n',
            };
            if !sentence_start && is_titlecase(stripped) {
                names.push(stripped.to_lowercase());
            }
            if let Some(c) = word.trim_end_matches(is_closer).chars().last() {
                previous = Some(c);
            }
        }
    }
    names
}

fn lowercase_first_word(
    text: &str,
    before: &str,
    after: &str,
    dictionary: &[String],
) -> Option<String> {
    let (start, word) = first_word(text);
    let first = word.chars().next()?;
    if !first.is_uppercase() || !is_cased_script(first) {
        return None;
    }
    if !is_titlecase(word) {
        return None;
    }
    let bare = strip_word_punct(word);
    // "I" and its contractions, with either apostrophe.
    let i_forms = [
        "I",
        "I'm",
        "I'll",
        "I'd",
        "I've",
        "I\u{2019}m",
        "I\u{2019}ll",
        "I\u{2019}d",
        "I\u{2019}ve",
    ];
    if i_forms.contains(&bare)
        || i_forms.contains(&word.trim_end_matches(|c: char| !c.is_alphanumeric()))
    {
        return None;
    }
    // A title or initial before a name: "Mr. Smith", "J. K. Rowling".
    if word.ends_with('.') {
        let next_word = text[start + word.len()..]
            .split(is_space)
            .find(|w| !w.is_empty());
        if next_word
            .and_then(|w| w.chars().next())
            .is_some_and(|c| c.is_uppercase())
        {
            return None;
        }
    }
    let lowered_bare = bare.to_lowercase();
    if dictionary.iter().any(|entry| {
        entry.to_lowercase() == lowered_bare
            && entry.chars().next().is_some_and(|c| c.is_uppercase())
    }) {
        return None;
    }
    let rest_of_text = &text[start + word.len()..];
    if names_in(&[before, after, rest_of_text]).contains(&lowered_bare) {
        return None;
    }
    let mut lower = first.to_lowercase();
    if lower.len() != 1 {
        return None;
    }
    let lower = lower.next()?;
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..start]);
    out.push(lower);
    out.push_str(&text[start + first.len_utf8()..]);
    Some(out)
}

/// Give the dictation's first letter the case of the selection it replaces.
fn match_case(text: &str, upper: bool) -> Option<String> {
    let (start, word) = first_word(text);
    let first = word.chars().next()?;
    if !is_cased_script(first) || first.is_uppercase() == upper {
        return Some(text.to_string());
    }
    if !upper && !is_titlecase(word) {
        // Never lowercase an acronym or a mixed-case name.
        return Some(text.to_string());
    }
    let mut mapped = if upper {
        first.to_uppercase().collect::<String>()
    } else {
        first.to_lowercase().collect::<String>()
    };
    if mapped.chars().count() != 1 {
        mapped = first.to_string();
    }
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..start]);
    out.push_str(&mapped);
    out.push_str(&text[start + first.len_utf8()..]);
    Some(out)
}

/* ───────────────────────────── final marks ───────────────────────────── */

/// Whether the text after the caret continues the sentence the dictation is
/// inserted into, so the dictation's own final mark would sit in the middle.
fn continues_sentence(after: &str) -> bool {
    let rest = after.trim_start_matches(|c: char| is_hspace(c) || is_closer(c));
    match rest.chars().next() {
        None => false,
        Some(c) => c.is_lowercase() || matches!(c, ',' | ';' | ':' | '.' | '!' | '?' | '\u{2026}'),
    }
}

/// Drop one final ".", "!" or "?" from the dictation, unless it belongs to an
/// ellipsis, a doubled mark, an abbreviation or an initial.
fn drop_final_mark(text: &str) -> Option<String> {
    let trimmed = text.trim_end_matches(is_hspace);
    let mark = trimmed.chars().last()?;
    if !matches!(mark, '.' | '!' | '?') {
        return None;
    }
    let body = &trimmed[..trimmed.len() - 1];
    if body.ends_with(['.', '!', '?']) {
        return None;
    }
    let body_core = body.trim_end_matches(is_hspace);
    if body_core.is_empty() || !body_core.chars().any(|c| c.is_alphanumeric()) {
        return None;
    }
    if mark == '.' {
        let word = last_word(body_core);
        if word.contains('.') {
            return None; // "a.m.", "e.g", "U.S"
        }
        let letters: Vec<char> = word.chars().collect();
        if letters.len() == 1 && letters[0].is_alphabetic() {
            return None; // "Plan B."
        }
        if TITLES.contains(&format!("{}.", word.to_lowercase()).as_str()) {
            return None;
        }
    }
    Some(body_core.to_string())
}

/// "name@example.com." typed into an empty field: the period is the speech
/// model's, not part of the address. Addresses with a path, dates and
/// abbreviations keep theirs.
fn strip_standalone_value_period(text: &str) -> Option<String> {
    let body = text.strip_suffix('.')?;
    if body.is_empty() || body.ends_with('.') || body.chars().any(is_space) {
        return None;
    }
    let is_email = {
        let mut parts = body.split('@');
        let (local, domain) = (parts.next()?, parts.next());
        parts.next().is_none()
            && !local.is_empty()
            && domain.is_some_and(|d| d.contains('.') && !d.starts_with('.') && !d.ends_with('.'))
    };
    let is_bare_url = {
        let host = body
            .strip_prefix("https://")
            .or_else(|| body.strip_prefix("http://"))
            .unwrap_or(body);
        let host = host.strip_suffix('/').unwrap_or(host);
        !host.contains('/')
            && !host.contains('@')
            && host.contains('.')
            && host.split('.').all(|label| {
                !label.is_empty() && label.chars().all(|c| c.is_alphanumeric() || c == '-')
            })
            && host
                .rsplit('.')
                .next()
                .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_alphabetic()))
    };
    let is_number = {
        let digits = body.strip_prefix(['-', '+']).unwrap_or(body);
        !digits.is_empty()
            && digits.chars().next().is_some_and(|c| c.is_ascii_digit())
            && digits
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
            && digits.matches('.').count() <= 1
    };
    (is_email || is_bare_url || is_number).then(|| body.to_string())
}

/* ───────────────────────────── our own stray period ───────────────────────────── */

/// Words a sentence cannot end on: determiners, possessives, conjunctions.
/// "I went to the." is unfinished, so a period after it is the speech model's.
fn never_final(language: &str) -> &'static [&'static str] {
    let base = language
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match base.as_str() {
        "fr" => &[
            "le", "la", "les", "un", "une", "des", "du", "au", "aux", "et", "ou", "mais", "donc",
            "car", "mon", "ton", "son", "ma", "ta", "sa", "mes", "tes", "ses", "notre", "votre",
            "leur", "leurs", "ce", "cette", "ces",
        ],
        "es" => &[
            "el", "los", "las", "un", "una", "unos", "unas", "y", "pero", "del", "al", "mi", "tu",
            "su", "mis", "tus", "sus", "nuestro", "nuestra",
        ],
        "de" => &[
            "der", "das", "den", "dem", "des", "ein", "eine", "einen", "einem", "einer", "und",
            "oder", "aber", "mein", "meine", "dein", "deine", "sein", "seine", "ihr", "ihre",
        ],
        "it" => &[
            "il", "lo", "gli", "un", "una", "e", "ma", "del", "della", "dei", "degli", "mio",
            "mia", "tuo", "tua", "suo", "sua",
        ],
        "pt" => &[
            "o", "os", "um", "uma", "uns", "umas", "e", "mas", "do", "da", "dos", "das", "meu",
            "minha", "teu", "tua", "seu", "sua",
        ],
        _ => &[
            "a", "an", "the", "my", "your", "our", "their", "its", "his", "every", "and", "or",
            "nor", "but", "because", "although", "though", "whereas", "unless", "than", "of",
            "very", "to", "with", "for", "at", "in", "on", "by", "about", "from", "into", "onto",
            "upon",
        ],
    }
}

/// Whether the period that ends `before` is SpeakoFlow's own, left in the
/// middle of a sentence the new dictation continues.
fn own_period_continues(before: &str, transcript: &str, language: &str) -> bool {
    let Some(body) = before.strip_suffix('.') else {
        return false;
    };
    if body.ends_with('.') || body.ends_with(is_space) {
        return false;
    }
    let word = last_word(body);
    if word.is_empty()
        || word.contains('.')
        || !word.chars().all(|c| c.is_alphabetic() || c == '\'')
    {
        return false;
    }
    let lower = word.to_lowercase();
    // "Mr." and "e.g." are abbreviations, not sentence ends.
    let dotted = format!("{lower}.");
    if TITLES.contains(&dotted.as_str()) || NON_FINAL_ABBREVIATIONS.contains(&dotted.as_str()) {
        return false;
    }
    // "Oh my." is a sentence.
    if lower == "my" {
        let previous = last_word(body[..body.len() - word.len()].trim_end()).to_lowercase();
        if matches!(previous.as_str(), "oh" | "my") {
            return false;
        }
    }
    let first = transcript
        .chars()
        .find(|c| !matches!(c, '"' | '\'' | '\u{00BF}' | '\u{00A1}'));
    // A dictation that starts lowercase was itself heard as a continuation.
    let continues_lowercase = first.is_some_and(|c| c.is_lowercase() && is_cased_script(c));
    let starts_new_sentence_word = first.is_some_and(|c| !c.is_alphanumeric());
    if starts_new_sentence_word {
        return false;
    }
    never_final(language).contains(&lower.as_str()) || continues_lowercase
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(before: &str, after: &str, transcript: &str) -> String {
        fit(
            transcript,
            Context::Field {
                before,
                after,
                selected: "",
            },
            &Options::default(),
        )
        .text
    }

    fn with(before: &str, after: &str, transcript: &str, options: Options<'_>) -> Fitted {
        fit(
            transcript,
            Context::Field {
                before,
                after,
                selected: "",
            },
            &options,
        )
    }

    fn selected(before: &str, sel: &str, after: &str, transcript: &str) -> String {
        fit(
            transcript,
            Context::Field {
                before,
                after,
                selected: sel,
            },
            &Options::default(),
        )
        .text
    }

    fn lang(language: &str) -> Options<'_> {
        Options {
            language,
            ..Options::default()
        }
    }

    fn ours() -> Options<'static> {
        Options {
            previous_is_ours: true,
            ..Options::default()
        }
    }

    /* ── continuing a sentence ── */

    #[test]
    fn continuation_mid_sentence_lowercases_and_drops_the_period_before_more_text() {
        assert_eq!(
            field("The review is ", " if that suits you.", "Tomorrow at noon."),
            "tomorrow at noon"
        );
        assert_eq!(
            field("The review is ", "", "Tomorrow at noon."),
            "tomorrow at noon."
        );
    }

    #[test]
    fn a_new_sentence_keeps_its_capital() {
        assert_eq!(field("", "", "Hello there."), "Hello there.");
        assert_eq!(field("We ate. ", "", "Then we left."), "Then we left.");
        assert_eq!(field("Really? ", "", "Yes."), "Yes.");
        assert_eq!(field("First line,\n", "", "Second line."), "Second line.");
        assert_eq!(
            field("He said \"stop.\" ", "", "Then he left."),
            "Then he left."
        );
        assert_eq!(field("Tu viens\u{202F}? ", "", "Oui."), "Oui.");
        assert_eq!(
            field("Hi John\n", "", "Thanks for that."),
            "Thanks for that."
        );
    }

    #[test]
    fn after_an_ellipsis_the_case_is_left_alone() {
        assert_eq!(field("Well\u{2026} ", "", "Yes."), "Yes.");
    }

    #[test]
    fn list_and_heading_markers_start_an_item() {
        assert_eq!(field("- ", "", "Buy milk."), "Buy milk.");
        assert_eq!(field("Steps\n12) ", "", "Done."), "Done.");
        assert_eq!(field("- [ ] ", "", "Call Anna."), "Call Anna.");
        assert_eq!(field("Notes\n-", "", "Buy milk."), " Buy milk.");
        assert_eq!(field("## ", "", "Overview"), "Overview");
    }

    #[test]
    fn glue_marks_take_no_space() {
        assert_eq!(field("#", "", "Release."), "Release.");
        assert_eq!(field("Very self-", "", "Aware."), "aware.");
        assert_eq!(field("see https://example.com/", "", "docs"), "docs");
        assert_eq!(field("mail bob@", "", "example.com"), "example.com");
        assert_eq!(field("/", "", "ok"), "ok");
        assert_eq!(field("and-", "", "or"), "or");
    }

    #[test]
    fn a_spaced_dash_is_a_separator() {
        assert_eq!(field("This is it -", "", "Or not."), " or not.");
    }

    #[test]
    fn commas_colons_and_brackets_continue_the_sentence() {
        assert_eq!(field("Item 1) ", "", "Then left."), "then left.");
        assert_eq!(
            field("I'd love to, ", "", "But not today."),
            "but not today."
        );
        assert_eq!(field("We need: ", "", "Bread and milk."), "bread and milk.");
        assert_eq!(field("It rained", "", "And then some."), " and then some.");
        assert_eq!(field("Yes,", "", "But no."), " but no.");
    }

    #[test]
    fn accented_and_cyrillic_letters_are_lowercased_too() {
        assert_eq!(
            field("On se voit ", "", "\u{00C0} demain."),
            "\u{00E0} demain."
        );
        assert_eq!(
            field(
                "\u{041C}\u{044B} \u{0438}\u{0434}\u{0451}\u{043C}",
                "",
                "\u{0414}\u{043E}\u{043C}\u{043E}\u{0439}."
            ),
            " \u{0434}\u{043E}\u{043C}\u{043E}\u{0439}."
        );
    }

    /* ── what is never lowercased ── */

    #[test]
    fn i_acronyms_and_mixed_case_stay() {
        assert_eq!(field("Well, ", "", "I think so."), "I think so.");
        assert_eq!(field("and then", "", "I'm sure"), " I'm sure");
        assert_eq!(field("Our ", "", "API keys rotate."), "API keys rotate.");
        assert_eq!(
            field("Apparently ", "", "GitHub is down."),
            "GitHub is down."
        );
        assert_eq!(field("we use", "", "NASA tools."), " NASA tools.");
        assert_eq!(field("about", "", "TypeWhisper"), " TypeWhisper");
        assert_eq!(field("We bought ", "", "3 apples."), "3 apples.");
    }

    #[test]
    fn names_seen_capitalised_mid_sentence_stay() {
        assert_eq!(
            field("We moved to Paris last year and ", "", "Paris is lovely."),
            "Paris is lovely."
        );
        assert_eq!(field("Then ", "", "Mr. Smith came."), "Mr. Smith came.");
        assert_eq!(
            field("I think ", "", "J. K. Rowling wrote it."),
            "J. K. Rowling wrote it."
        );
        assert_eq!(field("Ask the ", "", "Dr. who knows."), "dr. who knows.");
    }

    #[test]
    fn dictionary_names_stay() {
        let words = vec!["Marie".to_string()];
        let options = Options {
            dictionary: &words,
            ..Options::default()
        };
        assert_eq!(with("Thanks @", "", "Marie.", options).text, "Marie.");
        assert_eq!(
            with("I talked to ", "", "Marie today.", options).text,
            "Marie today."
        );
    }

    #[test]
    fn german_and_turkish_are_never_lowercased() {
        assert_eq!(
            with("Ich habe den ", "", "Termin morgen.", lang("de")).text,
            "Termin morgen."
        );
        assert_eq!(
            with("Ich habe den ", "", "Termin morgen.", lang("auto")).text,
            "Termin morgen."
        );
        assert_eq!(
            with(
                "Tamam, ",
                "",
                "I\u{015F}\u{0131}k yand\u{0131}.",
                lang("tr")
            )
            .text,
            "I\u{015F}\u{0131}k yand\u{0131}."
        );
    }

    #[test]
    fn fixing_case_can_be_turned_off() {
        let options = Options {
            fix_case: false,
            ..Options::default()
        };
        assert_eq!(
            with("I went to the", "", "Store today", options).text,
            " Store today"
        );
    }

    /* ── final marks ── */

    #[test]
    fn the_final_mark_goes_only_when_the_sentence_continues() {
        assert_eq!(field("It was ", " good", "Really?"), "really");
        assert_eq!(field("Not today but ", ", sorry", "Tomorrow."), "tomorrow");
        assert_eq!(
            field("I'll come (", ") tomorrow", "If it's sunny."),
            "if it's sunny"
        );
        assert_eq!(field("Done. (", ") Then", "It was late."), "It was late.");
        assert_eq!(
            field("Not today but ", " Then we go.", "Tomorrow."),
            "tomorrow."
        );
        assert_eq!(field("Not today but ", "\nnext", "Tomorrow."), "tomorrow.");
        assert_eq!(field("C'est ", " bizarre", "Vraiment ?"), "vraiment");
        assert_eq!(
            field(
                "start",
                ", I will begin",
                "Dictation in the middle of a sentence before the comma."
            ),
            " dictation in the middle of a sentence before the comma"
        );
    }

    #[test]
    fn ellipses_abbreviations_and_initials_keep_their_period() {
        assert_eq!(field("It was ", " fine", "Well..."), "well...");
        assert_eq!(field("Meet ", " sharp", "At 10 a.m."), "at 10 a.m.");
        assert_eq!(field("Go with ", " then", "Plan B."), "plan B.");
        assert_eq!(field("I'm at ", " now", "Level 5."), "level 5");
    }

    #[test]
    fn standalone_values_in_an_empty_field_lose_the_model_period() {
        let empty = |t: &str| fit(t, Context::EmptyField, &Options::default()).text;
        assert_eq!(empty("name@example.com."), "name@example.com");
        assert_eq!(empty("example.com."), "example.com");
        assert_eq!(empty("3.14."), "3.14");
        assert_eq!(
            empty("https://example.com/docs."),
            "https://example.com/docs."
        );
        assert_eq!(empty("19.04.2026."), "19.04.2026.");
        assert_eq!(empty("Dr."), "Dr.");
        assert_eq!(empty("Hello there."), "Hello there.");
    }

    /* ── spaces ── */

    #[test]
    fn a_space_goes_between_words_and_before_a_new_sentence() {
        assert_eq!(field("We ate.", "", "Then we left."), " Then we left.");
        assert_eq!(field("first", "", "second."), " second.");
        assert_eq!(field("Hello", ",", "friend"), " friend");
        assert_eq!(field("paid", "", "$50"), " $50");
    }

    #[test]
    fn no_space_before_punctuation_or_after_openers() {
        assert_eq!(field("It works", "", ", right?"), ", right?");
        assert_eq!(field("s", "", ", and"), ", and");
        assert_eq!(field("(", "", ")"), ")");
        assert_eq!(field("costs $", "", "50"), "50");
        assert_eq!(field("about 50", "", "%"), "%");
        assert_eq!(field("mary", "", "-kate"), "-kate");
    }

    #[test]
    fn straight_quotes_open_and_close_by_their_neighbours() {
        assert_eq!(field("He said \"hi\"", "", "And left."), " and left.");
        assert_eq!(field("The students'", "", "Were there."), " were there.");
        assert_eq!(
            field("Then", "", "\"Hello,\" she said."),
            " \"Hello,\" she said."
        );
        assert_eq!(field("said", "", "'hello"), " 'hello");
        assert_eq!(field("He said \"", "", "Hello"), "hello");
        assert_eq!(field("{\"key\":\"", "", "value"), "value");
        assert_eq!(field("Bob", "", "'s car"), "'s car");
    }

    #[test]
    fn french_elisions_take_no_space() {
        assert_eq!(field("J'ai pris l'", "", "Avion."), "avion.");
        assert_eq!(field("J\u{2019}ai pris l\u{2019}", "", "Avion."), "avion.");
    }

    #[test]
    fn a_trailing_space_only_when_a_word_follows() {
        assert_eq!(field("coffee", "machine", "strong"), " strong ");
        assert_eq!(field("Meet ", "tomorrow", "At noon."), "at noon ");
        assert_eq!(field("Meet ", "Then", "At noon."), "at noon. ");
        assert_eq!(field("Meet ", "(sharp)", "At noon"), "at noon ");
        assert_eq!(field("Meet ", ", ok", "At noon"), "at noon");
        assert_eq!(field("Meet ", "", "At noon. "), "at noon.");
        assert_eq!(field("It is", "good", "Very"), " very ");
        assert_eq!(field("Done.", "Next", "Another item"), " Another item ");
        assert_eq!(field("will ", "begin", "Immediately."), "immediately ");
        assert_eq!(field("coffee", "machine", "Really?"), " really ");
        assert_eq!(field("", "Hello", "Hi there"), "Hi there ");
        assert_eq!(field("Hello", "world", "friend"), " friend ");
        assert_eq!(field("Hello", " world", "friend"), " friend");
    }

    #[test]
    fn numbers_continue_across_the_seam() {
        assert_eq!(field("version 2.", "", "5"), "5");
        assert_eq!(field("at 10:", "", "30"), "30");
    }

    #[test]
    fn line_starts_and_embedded_objects_take_no_space() {
        assert_eq!(
            field("Hello\u{FFFC}", "", "Next paragraph."),
            "Next paragraph."
        );
        assert_eq!(field("Hi team\n", "", "Quick update."), "Quick update.");
        assert_eq!(field("a\u{FFFC}", "", "Hi"), "Hi");
    }

    /* ── scripts ── */

    #[test]
    fn scripts_without_spaces_get_none() {
        assert_eq!(
            field("\u{6211}\u{8BF4}", "", "\u{4F60}\u{597D}\u{3002}"),
            "\u{4F60}\u{597D}\u{3002}"
        );
        assert_eq!(
            field("Say", "", "\u{3053}\u{3093}\u{306B}\u{3061}\u{306F}"),
            "\u{3053}\u{3093}\u{306B}\u{3061}\u{306F}"
        );
        assert_eq!(field("A", "B", "\u{4E2D}"), "\u{4E2D}");
        assert_eq!(
            field(
                "\u{0E2A}\u{0E27}\u{0E31}\u{0E2A}\u{0E14}\u{0E35}",
                "",
                "\u{0E04}\u{0E23}\u{0E31}\u{0E1A}"
            ),
            "\u{0E04}\u{0E23}\u{0E31}\u{0E1A}"
        );
        assert_eq!(field("\u{30C7}\u{30FC}", "", "\u{30BF}"), "\u{30BF}");
    }

    #[test]
    fn korean_keeps_its_spaces() {
        assert_eq!(
            field(
                "\u{C548}\u{B155}\u{D558}\u{C138}\u{C694}.",
                "",
                "\u{BC18}\u{AC11}\u{C2B5}\u{B2C8}\u{B2E4}."
            ),
            " \u{BC18}\u{AC11}\u{C2B5}\u{B2C8}\u{B2E4}."
        );
    }

    #[test]
    fn devanagari_and_arabic_sentence_marks() {
        // Nepali: a sentence ending in a danda, then a new one.
        let before = "\u{0928}\u{092E}\u{0938}\u{094D}\u{0924}\u{0947}\u{0964}";
        let text = "\u{0906}\u{092A} \u{0915}\u{0938}\u{094D}\u{0924}\u{094B} \u{091B}?";
        assert_eq!(field(before, "", text), format!(" {text}"));
        // No space before a danda or an Arabic question mark.
        assert_eq!(
            field("\u{0939}\u{093E}\u{0901}", "", "\u{0964}"),
            "\u{0964}"
        );
        assert_eq!(
            field(
                "\u{0643}\u{064A}\u{0641} \u{062D}\u{0627}\u{0644}\u{0643}",
                "",
                "\u{061F}"
            ),
            "\u{061F}"
        );
        // A space after one.
        assert_eq!(
            field(
                "\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}\u{061F}",
                "",
                "\u{0634}\u{0643}\u{0631}\u{0627}"
            ),
            " \u{0634}\u{0643}\u{0631}\u{0627}"
        );
    }

    #[test]
    fn spanish_inverted_marks() {
        assert_eq!(field("Hola. ", "", "\u{00BF}Vienes?"), "\u{00BF}Vienes?");
        assert_eq!(
            field("Oye,", "", "\u{00BF}Vienes ma\u{00F1}ana?"),
            " \u{00BF}vienes ma\u{00F1}ana?"
        );
        assert_eq!(field("Oye, \u{00BF}", "?", "Vienes"), "vienes");
    }

    #[test]
    fn georgian_case_is_never_changed() {
        let text = "\u{10D2}\u{10D0}\u{10DB}\u{10D0}\u{10E0}\u{10EF}\u{10DD}\u{10D1}\u{10D0}";
        assert_eq!(field("and", "", text), format!(" {text}"));
    }

    /* ── selections ── */

    #[test]
    fn a_replaced_selection_lends_its_case() {
        assert_eq!(
            selected("", "Tomorrow", " is fine.", "next week."),
            "Next week"
        );
        assert_eq!(
            selected("Meet on ", "Monday", " at noon.", "Friday."),
            "Friday"
        );
        assert_eq!(selected("a ", "old word", " b", "New."), "new");
    }

    /* ── the last-paste memory ── */

    #[test]
    fn a_remembered_paste_only_adds_the_space() {
        let remembered =
            |last: &str, t: &str| fit(t, Context::Remembered { last }, &Options::default()).text;
        assert_eq!(remembered("goes.", "Ok, now"), " Ok, now");
        assert_eq!(remembered("I went to the", "Store"), " Store");
        assert_eq!(
            remembered("\u{4F60}\u{597D}", "\u{4E16}\u{754C}"),
            "\u{4E16}\u{754C}"
        );
        assert_eq!(remembered("done ", "Next"), "Next");
    }

    #[test]
    fn nothing_known_changes_nothing() {
        assert_eq!(
            fit("Hello there.", Context::Nothing, &Options::default()).text,
            "Hello there."
        );
    }

    /* ── our own stray period ── */

    #[test]
    fn our_own_period_after_an_unfinished_sentence_is_removed() {
        let fitted = with("I went to the.", "", "Store.", ours());
        assert_eq!((fitted.text.as_str(), fitted.erase_before), (" store.", 1));
        let fitted = with("We need an.", "", "Umbrella.", ours());
        assert_eq!(
            (fitted.text.as_str(), fitted.erase_before),
            (" umbrella.", 1)
        );
        let fitted = with("...at the chunk.", "", "boundaries this time", ours());
        assert_eq!(
            (fitted.text.as_str(), fitted.erase_before),
            (" boundaries this time", 1)
        );
    }

    #[test]
    fn a_finished_sentence_keeps_its_period() {
        let fitted = with("It works.", "", "The next part starts here", ours());
        assert_eq!(
            (fitted.text.as_str(), fitted.erase_before),
            (" The next part starts here", 0)
        );
        let fitted = with("Oh my.", "", "That hurt.", ours());
        assert_eq!(
            (fitted.text.as_str(), fitted.erase_before),
            (" That hurt.", 0)
        );
        let fitted = with("I spoke to Mr.", "", "smith about it", ours());
        assert_eq!(
            (fitted.text.as_str(), fitted.erase_before),
            (" smith about it", 0)
        );
        let fitted = with("I finished the report.", "", "Then I left.", ours());
        assert_eq!(
            (fitted.text.as_str(), fitted.erase_before),
            (" Then I left.", 0)
        );
    }

    #[test]
    fn a_period_that_is_not_ours_is_never_removed() {
        let fitted = with("I went to the.", "", "Store.", Options::default());
        assert_eq!((fitted.text.as_str(), fitted.erase_before), (" Store.", 0));
    }

    #[test]
    fn joining_can_be_turned_off() {
        let options = Options {
            join_sentences: false,
            previous_is_ours: true,
            ..Options::default()
        };
        let fitted = with("I went to the.", "", "Store.", options);
        assert_eq!((fitted.text.as_str(), fitted.erase_before), (" Store.", 0));
    }

    #[test]
    fn the_whole_stop_and_continue_example() {
        // "I went to the" / "store and bought milk" / "Then I went home".
        let first = fit("I went to the.", Context::EmptyField, &Options::default());
        assert_eq!(first.text, "I went to the.");
        let second = with("I went to the.", "", "Store and bought milk.", ours());
        assert_eq!(
            (second.text.as_str(), second.erase_before),
            (" store and bought milk.", 1)
        );
        let third = with(
            "I went to the store and bought milk.",
            "",
            "Then I went home.",
            ours(),
        );
        assert_eq!(
            (third.text.as_str(), third.erase_before),
            (" Then I went home.", 0)
        );
    }

    #[test]
    fn our_paste_is_recognised_at_the_end_of_the_field() {
        assert!(ends_with_pasted("Notes: I went to the.", " I went to the."));
        assert!(ends_with_pasted("I went to the.", "I went to the."));
        assert!(!ends_with_pasted("I went to the. And", " I went to the."));
        assert!(!ends_with_pasted("the.", " I went to the."));
        assert!(!ends_with_pasted("anything", ""));
        let last = "a".repeat(40) + " and then we went to the.";
        let before: String = last.chars().skip(last.chars().count() - 30).collect();
        assert!(ends_with_pasted(&before, &last));
    }

    #[test]
    fn known_says_whether_anything_was_known() {
        assert!(!fit("Hi.", Context::Nothing, &Options::default()).known);
        assert!(fit("Hi.", Context::EmptyField, &Options::default()).known);
        assert!(
            fit(
                "Hi.",
                Context::Remembered { last: "x" },
                &Options::default()
            )
            .known
        );
    }

    #[test]
    fn empty_or_blank_transcripts_are_left_alone() {
        assert_eq!(field("word", "", ""), "");
        assert_eq!(field("word", "", "   "), "   ");
    }

    #[test]
    fn two_dictations_in_a_row_get_exactly_one_space() {
        assert_eq!(field("Done.", "", "Hello world."), " Hello world.");
        assert_eq!(field("Done. ", "", "Hello world."), "Hello world.");
        assert_eq!(field("word ", "", "  next"), "next");
    }

    #[test]
    fn a_dictated_line_break_is_kept_and_takes_no_space() {
        assert_eq!(field("Hi,", "", "\nThanks."), "\nThanks.");
    }

    #[test]
    fn after_an_emoji_or_a_symbol_the_case_is_left_alone() {
        assert_eq!(
            field("Great job \u{1F44D}", "", "Thanks again."),
            " Thanks again."
        );
        assert_eq!(field("up 50%", "", "Then it fell."), " Then it fell.");
    }

    #[test]
    fn abbreviations_that_do_not_end_a_sentence() {
        assert_eq!(field("fruit, e.g.", "", "Apples."), " apples.");
        assert_eq!(field("Ask Dr.", "", "Smith today."), " Smith today.");
    }

    #[test]
    fn single_capital_letters_are_lowercased_but_not_i() {
        assert_eq!(field("I bought", "", "A new phone."), " a new phone.");
        assert_eq!(field("so", "", "I left."), " I left.");
    }
}
