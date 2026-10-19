//! Spoken-language codes: how the user's choice meets a model's list.
//!
//! Three vocabularies meet here. The app's own (`selected_language`, the
//! language picker) uses Whisper's codes plus `zh-Hans` / `zh-Hant`, because
//! the Simplified/Traditional conversion runs on the transcript afterwards.
//! A model lists whatever its authors wrote: `nb` or `no` for Norwegian, `fil`
//! or `tl` for Filipino, `zh` for Chinese, sometimes a full locale (`en-US`).
//! And the engine wants one of the model's own strings back.
//!
//! So there is one canonical form for *comparing* codes
//! ([`canonical_language_code`]), and two directions out of it:
//! [`effective_language`] answers in the app's vocabulary (what the picker
//! shows, what the transcript post-processing keys on), and
//! [`engine_language_code`] answers in the model's (what is handed to the
//! engine). The frontend mirrors the same rule in `src/lib/languageCodes.ts`;
//! the two are tested against the same cases so they cannot drift.
//!
//! None of this ever writes back to settings: `selected_language` is the
//! user's intent, so switching to a model that cannot honour it and back again
//! returns to exactly what they chose.

/// The value of `selected_language` that asks the model to detect the language.
pub const AUTO: &str = "auto";

/// The primary language subtag, lowercased: `en-US` → `en`, `zh-Hant` → `zh`,
/// `pt_BR` → `pt`.
pub fn base_language(code: &str) -> String {
    code.trim()
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The form two codes are compared in: the base language, with the two
/// aliases model authors disagree on folded together. Norwegian Bokmål (`nb`)
/// is Norwegian (`no`) while Nynorsk (`nn`) stays its own language; Filipino
/// (`fil`) is Tagalog (`tl`). Cantonese (`yue`) is a language of its own, not a
/// kind of Chinese, so it never matches `zh`.
pub fn canonical_language_code(code: &str) -> String {
    match base_language(code).as_str() {
        "nb" => "no".to_string(),
        "fil" => "tl".to_string(),
        base => base.to_string(),
    }
}

/// Whether two codes name the same spoken language.
pub fn languages_match(a: &str, b: &str) -> bool {
    let a = canonical_language_code(a);
    !a.is_empty() && a == canonical_language_code(b)
}

/// A model's code written the way the app writes it. Chinese becomes
/// Simplified, the script nearly every Chinese model writes by default; the
/// two script variants the app already uses are kept as they are.
pub fn app_language_code(model_code: &str) -> String {
    let code = model_code.trim();
    if code == "zh-Hans" || code == "zh-Hant" {
        return code.to_string();
    }
    match canonical_language_code(code).as_str() {
        "zh" => "zh-Hans".to_string(),
        other => other.to_string(),
    }
}

/// The language a model will actually use for the user's `intent`, in the
/// app's vocabulary.
///
/// In order: the intent itself when the model lists it (exactly, by base
/// language, or through an alias, so `zh-Hans` stays `zh-Hans` on a model that
/// lists `zh` and `no` stays `no` on one that lists `nb`); otherwise `auto`
/// when the model can detect the language; otherwise English when it has it;
/// otherwise the model's first language. A model with no list at all is
/// language-agnostic, so the intent passes through untouched.
pub fn effective_language(intent: &str, supported: &[String], supports_detection: bool) -> String {
    if supported.is_empty() {
        return intent.to_string();
    }
    if intent != AUTO && supported.iter().any(|code| languages_match(code, intent)) {
        return intent.to_string();
    }
    if supports_detection {
        return AUTO.to_string();
    }
    if supported.iter().any(|code| base_language(code) == "en") {
        return "en".to_string();
    }
    app_language_code(&supported[0])
}

/// The model's own code for an [`effective_language`] result, which is what
/// the engine is given. `None` for `auto` (the engine detects) and for a
/// language the model does not list.
///
/// Script variants are dropped first (`zh-Hans` → `zh`), because the engine
/// recognises Chinese and the script is converted afterwards. Then the
/// closest listed code wins: the same string, then the same base language
/// (`en` → `en-US`), then an alias (`no` → `nb`). A model that lists both `no`
/// and `nb` therefore gets back exactly what was asked for. A model without a
/// list takes the code as it is.
pub fn engine_language_code(effective: &str, supported: &[String]) -> Option<String> {
    if effective == AUTO || effective.trim().is_empty() {
        return None;
    }
    let wanted = match effective {
        "zh-Hans" | "zh-Hant" => "zh",
        other => other,
    };
    if supported.is_empty() {
        return Some(wanted.to_string());
    }
    let wanted_base = base_language(wanted);
    supported
        .iter()
        .find(|code| code.as_str() == wanted)
        .or_else(|| {
            supported
                .iter()
                .find(|code| base_language(code) == wanted_base)
        })
        .or_else(|| supported.iter().find(|code| languages_match(code, wanted)))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn langs(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn canonical_codes() {
        assert_eq!(canonical_language_code("en"), "en");
        assert_eq!(canonical_language_code("en-US"), "en");
        assert_eq!(canonical_language_code("EN_gb"), "en");
        assert_eq!(canonical_language_code("zh-Hans"), "zh");
        assert_eq!(canonical_language_code("zh-Hant"), "zh");
        assert_eq!(canonical_language_code("nb"), "no");
        assert_eq!(canonical_language_code("no"), "no");
        assert_eq!(canonical_language_code("nn"), "nn");
        assert_eq!(canonical_language_code("fil"), "tl");
        assert_eq!(canonical_language_code("tl"), "tl");
        assert_eq!(canonical_language_code("yue"), "yue");
        assert_eq!(canonical_language_code(""), "");
    }

    #[test]
    fn matching_folds_aliases_but_keeps_cantonese_apart() {
        assert!(languages_match("nb", "no"));
        assert!(languages_match("fil", "tl"));
        assert!(languages_match("zh-Hans", "zh"));
        assert!(languages_match("zh-Hant", "zh"));
        assert!(languages_match("zh-Hans", "zh-Hant"));
        assert!(!languages_match("yue", "zh"));
        assert!(!languages_match("nn", "no"));
        assert!(!languages_match("", ""));
    }

    #[test]
    fn app_codes() {
        assert_eq!(app_language_code("zh"), "zh-Hans");
        assert_eq!(app_language_code("zh-Hant"), "zh-Hant");
        assert_eq!(app_language_code("zh-Hans"), "zh-Hans");
        assert_eq!(app_language_code("nb"), "no");
        assert_eq!(app_language_code("fil"), "tl");
        assert_eq!(app_language_code("en-US"), "en");
        assert_eq!(app_language_code("yue"), "yue");
    }

    #[test]
    fn effective_language_matrix() {
        let whisper = langs(&["en", "zh", "zh-Hans", "zh-Hant", "de", "no", "tl", "yue"]);
        let nemotron = langs(&["en", "de", "nb", "zh"]);
        let english = langs(&["en"]);
        let breeze = langs(&["zh", "en"]);
        let chinese_only = langs(&["zh"]);
        let cantonese_only = langs(&["yue"]);
        let locales = langs(&["en-US", "fil-PH"]);

        // Exact.
        assert_eq!(effective_language("de", &whisper, true), "de");
        // Script variant on a model that lists only `zh`: kept as chosen.
        assert_eq!(effective_language("zh-Hans", &nemotron, false), "zh-Hans");
        assert_eq!(effective_language("zh-Hant", &breeze, false), "zh-Hant");
        // Alias.
        assert_eq!(effective_language("no", &nemotron, false), "no");
        assert_eq!(effective_language("tl", &locales, false), "tl");
        // Base language against a full locale.
        assert_eq!(effective_language("en", &locales, false), "en");
        // Cantonese is not Chinese.
        assert_eq!(effective_language("yue", &nemotron, true), "auto");
        assert_eq!(effective_language("zh-Hans", &cantonese_only, false), "yue");
        // Not supported, model detects.
        assert_eq!(effective_language("ja", &whisper, true), "auto");
        assert_eq!(effective_language("auto", &whisper, true), "auto");
        // Not supported, no detection: English when it has it.
        assert_eq!(effective_language("ja", &nemotron, false), "en");
        assert_eq!(effective_language("auto", &english, false), "en");
        assert_eq!(effective_language("auto", &locales, false), "en");
        // ...otherwise the first language, in the app's vocabulary.
        assert_eq!(effective_language("auto", &chinese_only, false), "zh-Hans");
        assert_eq!(effective_language("de", &langs(&["nb", "sv"]), false), "no");
        // No list: language-agnostic, the intent passes through.
        assert_eq!(effective_language("ja", &[], false), "ja");
        assert_eq!(effective_language("auto", &[], false), "auto");
    }

    #[test]
    fn engine_language_code_matrix() {
        let whisper = langs(&["en", "zh", "zh-Hans", "zh-Hant", "no", "yue"]);
        let nemotron = langs(&["en", "nb", "zh"]);
        let locales = langs(&["en-US", "fil-PH"]);
        let both = langs(&["no", "nb"]);

        assert_eq!(engine_language_code("auto", &whisper), None);
        assert_eq!(engine_language_code("", &whisper), None);
        // Script variants reach the engine as plain Chinese.
        assert_eq!(
            engine_language_code("zh-Hans", &whisper).as_deref(),
            Some("zh")
        );
        assert_eq!(
            engine_language_code("zh-Hant", &nemotron).as_deref(),
            Some("zh")
        );
        // Alias to the model's own spelling.
        assert_eq!(engine_language_code("no", &nemotron).as_deref(), Some("nb"));
        assert_eq!(
            engine_language_code("tl", &locales).as_deref(),
            Some("fil-PH")
        );
        // Base language to a full locale.
        assert_eq!(
            engine_language_code("en", &locales).as_deref(),
            Some("en-US")
        );
        // A model listing both spellings gets exactly what was asked for.
        assert_eq!(engine_language_code("no", &both).as_deref(), Some("no"));
        assert_eq!(engine_language_code("nb", &both).as_deref(), Some("nb"));
        // Cantonese stays Cantonese.
        assert_eq!(
            engine_language_code("yue", &whisper).as_deref(),
            Some("yue")
        );
        assert_eq!(engine_language_code("yue", &nemotron), None);
        // Not listed.
        assert_eq!(engine_language_code("ja", &nemotron), None);
        // No list: as it is, minus the script.
        assert_eq!(engine_language_code("ja", &[]).as_deref(), Some("ja"));
        assert_eq!(engine_language_code("zh-Hant", &[]).as_deref(), Some("zh"));
    }

    #[test]
    fn effective_then_engine_round_trip() {
        // What the transcription path will do: resolve the intent, then hand
        // the model its own code.
        let nemotron = langs(&["en", "nb", "zh"]);
        for (intent, detect, engine) in [
            ("no", false, Some("nb")),
            ("zh-Hant", false, Some("zh")),
            ("ja", false, Some("en")),
            ("ja", true, None),
            ("auto", true, None),
        ] {
            let effective = effective_language(intent, &nemotron, detect);
            assert_eq!(
                engine_language_code(&effective, &nemotron).as_deref(),
                engine,
                "intent {intent}, detect {detect}"
            );
        }
    }
}
