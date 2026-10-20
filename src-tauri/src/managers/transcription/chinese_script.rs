//! Simplified / Traditional script for Chinese output.
//!
//! Two settings meet here. `selected_language` can itself name a script
//! (`zh-Hans`, `zh-Hant`), which the language picker and the cloud providers
//! depend on, and that choice has always converted every transcript. The
//! `chinese_script` setting (upstream Handy #2186) covers the rest: Chinese
//! that comes out of an auto-detected run, or of a plain `zh` or Cantonese
//! one, is rewritten into the script the user writes in, and anything that is
//! not Chinese is left alone, so a mixed-language user on auto only has their
//! Chinese touched.
//!
//! Upstream recognises Chinese output with `whatlang`. This keeps to what the
//! text itself shows instead, which needs no new dependency and answers the
//! one question asked: Chinese is the only language written mostly in Han
//! characters with no kana and no Hangul.

use std::sync::OnceLock;

use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use log::error;

use crate::managers::language;
use crate::settings::ChineseScript;

/// The Chinese language a transcript is in. Picks the regional conversion
/// tables: Taiwan's for Mandarin, Hong Kong's for Cantonese.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChineseVariety {
    Mandarin,
    Cantonese,
}

impl ChineseVariety {
    /// `zh`, `zh-Hans`, `zh-CN` are Mandarin and `yue` is Cantonese; any other
    /// language is not Chinese.
    pub fn from_language(code: &str) -> Option<Self> {
        match language::base_language(code).as_str() {
            "zh" => Some(Self::Mandarin),
            "yue" => Some(Self::Cantonese),
            _ => None,
        }
    }
}

/// The script a transcript should end up in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Simplified,
    Traditional,
}

impl Script {
    fn from_setting(setting: ChineseScript) -> Option<Self> {
        match setting {
            ChineseScript::Auto => None,
            ChineseScript::Simplified => Some(Self::Simplified),
            ChineseScript::Traditional => Some(Self::Traditional),
        }
    }
}

/// What happens to one run's output, decided before its text is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptRule {
    /// Left as the model wrote it.
    Keep,
    /// Known to be Chinese: always converted.
    Convert(ChineseVariety, Script),
    /// The language was detected: converted if the text turns out Chinese.
    IfChinese(ChineseVariety, Script),
}

/// The rule for a run.
///
/// `intent` is the user's `selected_language`, `effective` what the model
/// actually uses for it (`managers::language::effective_language`), and
/// `model_languages` what the model lists (empty when unknown, as on a cloud
/// provider).
pub fn script_rule(
    intent: &str,
    effective: &str,
    setting: ChineseScript,
    model_languages: &[String],
) -> ScriptRule {
    // A script chosen as the language has always converted every transcript.
    match intent {
        "zh-Hans" => return ScriptRule::Convert(ChineseVariety::Mandarin, Script::Simplified),
        "zh-Hant" => return ScriptRule::Convert(ChineseVariety::Mandarin, Script::Traditional),
        _ => {}
    }
    let Some(script) = Script::from_setting(setting) else {
        return ScriptRule::Keep;
    };
    if let Some(variety) = ChineseVariety::from_language(effective) {
        return ScriptRule::Convert(variety, script);
    }
    if effective != language::AUTO {
        // Another language was asked for and the model has it.
        return ScriptRule::Keep;
    }
    let lists = |code: &str| {
        model_languages
            .iter()
            .any(|listed| language::languages_match(listed, code))
    };
    let (mandarin, cantonese) = (lists("zh"), lists("yue"));
    if !model_languages.is_empty() && !mandarin && !cantonese {
        // This model cannot write Chinese, so kanji in its output are not.
        return ScriptRule::Keep;
    }
    let variety = if cantonese && !mandarin {
        ChineseVariety::Cantonese
    } else {
        ChineseVariety::Mandarin
    };
    ScriptRule::IfChinese(variety, script)
}

/// Whether text reads as Chinese: Han characters, no kana (Japanese) and no
/// Hangul (Korean), and Han at least half of its letters, so a Latin sentence
/// quoting one Chinese name is left alone while Chinese quoting "iPhone" is
/// not.
pub fn looks_chinese(text: &str) -> bool {
    let (mut han, mut other_letters) = (0usize, 0usize);
    for c in text.chars() {
        match c as u32 {
            // Hiragana, Katakana, their extensions, half-width Katakana.
            0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => return false,
            // Hangul syllables and jamo.
            0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => return false,
            // CJK Unified (+ Extension A), compatibility, Extension B and on.
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F => han += 1,
            _ if c.is_alphabetic() => other_letters += 1,
            _ => {}
        }
    }
    han >= 1 && han >= other_letters
}

/// Text decides an [`ScriptRule::IfChinese`] rule only once it has this many
/// characters: one Han character reads as Chinese, but Japanese often opens
/// with kanji before any kana.
const LIVE_DETECTION_MIN_CHARS: usize = 6;

/// The conversion a rule calls for on `text`, or `None` to leave it.
fn conversion(rule: ScriptRule, text: &str) -> Option<(ChineseVariety, Script)> {
    match rule {
        ScriptRule::Keep => None,
        ScriptRule::Convert(variety, script) => Some((variety, script)),
        ScriptRule::IfChinese(variety, script) => looks_chinese(text).then_some((variety, script)),
    }
}

/// The transcript in the script its rule calls for. `None` when nothing
/// applies, so a caller can tell a conversion from an unchanged text.
pub fn apply(rule: ScriptRule, text: &str) -> Option<String> {
    let (variety, script) = conversion(rule, text)?;
    Some(convert(text, variety, script))
}

/// The live preview's conversion, so the overlay shows the script that will
/// be pasted. The final transcript is converted on its own; this only has to
/// look right while speaking.
pub struct LiveScript {
    rule: ScriptRule,
    /// Settled once the text has decided an `IfChinese` rule. It sticks, so
    /// the preview never flips script mid-sentence.
    decided: Option<Option<(ChineseVariety, Script)>>,
}

impl LiveScript {
    pub fn new(rule: ScriptRule) -> Self {
        let decided = match rule {
            ScriptRule::Keep => Some(None),
            ScriptRule::Convert(variety, script) => Some(Some((variety, script))),
            ScriptRule::IfChinese(..) => None,
        };
        Self { rule, decided }
    }

    /// The preview's committed and tentative text, converted. Always given
    /// the model's own text, never an earlier conversion.
    pub fn apply(&mut self, committed: &str, tentative: &str) -> (String, String) {
        if self.decided.is_none() {
            let text = format!("{committed}{tentative}");
            if text.chars().filter(|c| !c.is_whitespace()).count() >= LIVE_DETECTION_MIN_CHARS {
                self.decided = Some(conversion(self.rule, &text));
            }
        }
        match self.decided.flatten() {
            Some((variety, script)) => (
                convert(committed, variety, script),
                convert(tentative, variety, script),
            ),
            None => (committed.to_string(), tentative.to_string()),
        }
    }
}

/// The OpenCC converter for a variety and script, built on first use and
/// kept: building one parses its dictionaries, too slow to repeat on every
/// live-preview update. A failed build is kept as `None`.
fn converter(variety: ChineseVariety, script: Script) -> Option<&'static OpenCC> {
    static MANDARIN_SIMPLIFIED: OnceLock<Option<OpenCC>> = OnceLock::new();
    static MANDARIN_TRADITIONAL: OnceLock<Option<OpenCC>> = OnceLock::new();
    static CANTONESE_SIMPLIFIED: OnceLock<Option<OpenCC>> = OnceLock::new();
    static CANTONESE_TRADITIONAL: OnceLock<Option<OpenCC>> = OnceLock::new();

    let (cell, config) = match (variety, script) {
        (ChineseVariety::Mandarin, Script::Simplified) => {
            (&MANDARIN_SIMPLIFIED, BuiltinConfig::Tw2sp)
        }
        (ChineseVariety::Mandarin, Script::Traditional) => {
            (&MANDARIN_TRADITIONAL, BuiltinConfig::S2tw)
        }
        (ChineseVariety::Cantonese, Script::Simplified) => {
            (&CANTONESE_SIMPLIFIED, BuiltinConfig::Hk2s)
        }
        (ChineseVariety::Cantonese, Script::Traditional) => {
            (&CANTONESE_TRADITIONAL, BuiltinConfig::S2hk)
        }
    };
    cell.get_or_init(|| match OpenCC::from_config(config) {
        Ok(converter) => Some(converter),
        Err(e) => {
            error!(
                "Failed to initialize OpenCC converter: {}. Keeping the original script.",
                e
            );
            None
        }
    })
    .as_ref()
}

/// `text` rewritten into `script`. Fails open: without a converter the text
/// comes back unchanged.
pub fn convert(text: &str, variety: ChineseVariety, script: Script) -> String {
    match converter(variety, script) {
        Some(converter) => converter.convert(text),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn langs(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|c| c.to_string()).collect()
    }

    use ChineseScript::{Auto, Simplified, Traditional};
    use ChineseVariety::{Cantonese, Mandarin};

    #[test]
    fn a_script_chosen_as_the_language_always_converts() {
        let whisper = langs(&["en", "zh", "yue", "ja"]);
        for setting in [Auto, Simplified, Traditional] {
            assert_eq!(
                script_rule("zh-Hans", "zh-Hans", setting, &whisper),
                ScriptRule::Convert(Mandarin, Script::Simplified)
            );
            assert_eq!(
                script_rule("zh-Hant", "zh-Hant", setting, &[]),
                ScriptRule::Convert(Mandarin, Script::Traditional)
            );
        }
    }

    #[test]
    fn auto_leaves_every_other_run_alone() {
        let whisper = langs(&["en", "zh", "yue"]);
        for (intent, effective) in [("auto", "auto"), ("zh", "zh"), ("yue", "yue")] {
            assert_eq!(
                script_rule(intent, effective, Auto, &whisper),
                ScriptRule::Keep
            );
        }
    }

    #[test]
    fn a_preference_converts_plain_chinese_and_cantonese() {
        let whisper = langs(&["en", "zh", "yue"]);
        assert_eq!(
            script_rule("zh", "zh", Traditional, &whisper),
            ScriptRule::Convert(Mandarin, Script::Traditional)
        );
        assert_eq!(
            script_rule("yue", "yue", Simplified, &whisper),
            ScriptRule::Convert(Cantonese, Script::Simplified)
        );
    }

    #[test]
    fn a_detected_language_converts_only_chinese_text() {
        let whisper = langs(&["en", "zh", "yue", "ja"]);
        let rule = script_rule("auto", "auto", Traditional, &whisper);
        assert_eq!(rule, ScriptRule::IfChinese(Mandarin, Script::Traditional));
        // A model whose only Chinese is Cantonese writes Cantonese.
        assert_eq!(
            script_rule("auto", "auto", Simplified, &langs(&["en", "yue"])),
            ScriptRule::IfChinese(Cantonese, Script::Simplified)
        );
        // An intent the model does not have, detected instead.
        assert_eq!(
            script_rule("ko", "auto", Simplified, &langs(&["en", "zh"])),
            ScriptRule::IfChinese(Mandarin, Script::Simplified)
        );
        // A cloud provider lists nothing and can write anything.
        assert_eq!(
            script_rule("auto", "auto", Simplified, &[]),
            ScriptRule::IfChinese(Mandarin, Script::Simplified)
        );
    }

    #[test]
    fn a_model_without_chinese_or_another_language_is_never_converted() {
        assert_eq!(
            script_rule("auto", "auto", Simplified, &langs(&["en", "ja"])),
            ScriptRule::Keep
        );
        assert_eq!(
            script_rule("ja", "ja", Traditional, &langs(&["en", "ja", "zh"])),
            ScriptRule::Keep
        );
        // Chinese asked of an English-only model became English.
        assert_eq!(
            script_rule("zh", "en", Traditional, &langs(&["en"])),
            ScriptRule::Keep
        );
    }

    #[test]
    fn chinese_text_is_told_from_japanese_korean_and_latin() {
        assert!(looks_chinese("今天天气很好。"));
        assert!(looks_chinese("我们今天用iPhone开会"));
        assert!(!looks_chinese("今日はいい天気です。"));
        assert!(!looks_chinese("안녕하세요 반갑습니다"));
        assert!(!looks_chinese("I met 王 at the station."));
        assert!(!looks_chinese("Hello there."));
        assert!(!looks_chinese(""));
    }

    #[test]
    fn conversion_follows_the_rule() {
        let traditional = ScriptRule::IfChinese(Mandarin, Script::Traditional);
        assert_eq!(apply(traditional, "简体中文").as_deref(), Some("簡體中文"));
        assert_eq!(apply(traditional, "今日はいい天気です。"), None);
        assert_eq!(apply(ScriptRule::Keep, "简体中文"), None);
        assert_eq!(
            apply(
                ScriptRule::Convert(Mandarin, Script::Simplified),
                "簡體中文"
            )
            .as_deref(),
            Some("简体中文")
        );
    }

    #[test]
    fn cantonese_uses_hong_kong_tables() {
        // Hong Kong writes 裏 where Taiwan writes 裡.
        assert_eq!(convert("里面", Cantonese, Script::Traditional), "裏面");
        assert_eq!(convert("裏面", Cantonese, Script::Simplified), "里面");
    }

    #[test]
    fn the_live_preview_waits_for_enough_text_and_then_sticks() {
        let mut live = LiveScript::new(ScriptRule::IfChinese(Mandarin, Script::Traditional));
        // Too little to tell yet: shown as written.
        assert_eq!(live.apply("简体", ""), ("简体".into(), String::new()));
        // Decided on the whole preview, committed and tentative alike.
        assert_eq!(
            live.apply("简体中文", "很好"),
            ("簡體中文".into(), "很好".into())
        );
        // Once decided it holds, even over text that alone would not decide.
        assert_eq!(live.apply("简体中文 ok", "").0, "簡體中文 ok");

        let mut japanese = LiveScript::new(ScriptRule::IfChinese(Mandarin, Script::Traditional));
        assert_eq!(japanese.apply("今日はいい天気", "").0, "今日はいい天気");
        assert_eq!(
            japanese.apply("今日はいい天気 简体", "").0,
            "今日はいい天気 简体"
        );

        let mut keep = LiveScript::new(ScriptRule::Keep);
        assert_eq!(keep.apply("简体中文", "").0, "简体中文");
    }
}
