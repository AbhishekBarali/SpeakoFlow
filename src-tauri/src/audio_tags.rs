//! ElevenLabs audio tags: bracketed performance cues such as `[laughs]`,
//! `[whispers]` or `[applause]` that Eleven v3 and v4 act out (or play as a
//! sound effect) instead of reading aloud.
//!
//! The feature has three parts, and they are all here so they cannot disagree:
//!
//! - [`active`] decides whether *this* reply may carry tags. That needs the
//!   ElevenLabs engine, a model that understands them, and the user's switch.
//! - [`prompt_section`] is the instruction added to the system prompt when
//!   tags are active, at the intensity the user chose.
//! - [`strip`] removes tags from text bound for any engine that would read them
//!   aloud as words. Tags can reach such an engine even with the feature off:
//!   a conversation held with tags on keeps them in its history, and a model
//!   imitates its own earlier replies after the user switches engine.
//!
//! The panel shows the reply as the model wrote it, tags included, so the
//! transcript reads like a script of what was performed.

use crate::settings::{AppSettings, AudioTagIntensity};
use once_cell::sync::Lazy;
use regex::Regex;

/// The model the ElevenLabs engine will actually request, with the same
/// fallback [`crate::tts`] applies when the field is empty.
pub fn elevenlabs_model(settings: &AppSettings) -> String {
    let model = settings.assistant_tts_model.trim();
    if model.is_empty() || model == "gpt-4o-mini-tts" {
        crate::tts::ELEVENLABS_DEFAULT_MODEL.to_string()
    } else {
        model.to_string()
    }
}

/// Whether an ElevenLabs model id performs audio tags.
///
/// Model ids name their generation in a `v<N>` segment: `eleven_v3`,
/// `eleven_v3_conversational`, `eleven_v4`, `eleven_v4_turbo`, against
/// `eleven_multilingual_v2` and `eleven_flash_v2_5`. Tags arrived with v3, so
/// any generation from 3 up counts, which keeps a future v5 working without a
/// release. Matching on the segment rather than a substring means a version
/// number elsewhere in the id cannot pass for one.
pub fn model_supports_tags(model: &str) -> bool {
    model
        .trim()
        .to_ascii_lowercase()
        .split(['_', '-'])
        .filter_map(|segment| segment.strip_prefix('v'))
        .filter_map(|digits| digits.parse::<u32>().ok())
        .any(|generation| generation >= 3)
}

/// Whether the reply being generated may carry audio tags: the engine is
/// ElevenLabs, its model performs them, and the user has switched them on.
pub fn active(settings: &AppSettings) -> bool {
    settings.assistant_tts_elevenlabs_audio_tags
        && settings.assistant_tts_engine == "elevenlabs"
        && model_supports_tags(&elevenlabs_model(settings))
}

/// Remove audio tags so an engine that cannot perform them does not read them
/// aloud ("laughs", "applause").
///
/// Deliberately narrow, because square brackets have other uses:
///
/// - A tag starts with a lowercase letter, which is how the prompt asks for
///   them and how ElevenLabs documents them, so `[Ctrl]`, `[1]` and `[TODO]`
///   are left alone.
/// - It holds only letters, spaces and light punctuation, and is at least two
///   characters long, so a checkbox `[x]` or an index `[i]` survives.
/// - It must not be glued to a word, so `items[idx]` is code, not a tag.
/// - Markdown links are consumed by the speech sanitizer before this runs, so
///   `[docs](https://…)` keeps its text.
pub fn strip(text: &str) -> String {
    static TAG: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(^|[^\p{L}\p{N}_\]])\[[a-z][\p{L} ,.'’\-]{1,58}\]").unwrap());
    // A tag must not follow a `]` (that is what keeps `map[key][value]` intact),
    // so in a run of touching tags (`[gasps][laughs][applause]`) a pass removes
    // only the first. The old fixed two passes therefore left the third of a run
    // to be read aloud. Repeat until nothing changes instead: every pass that
    // changes anything removes a tag, so this ends, and text without tags takes
    // one pass.
    let mut text = text.to_string();
    loop {
        let next = TAG.replace_all(&text, "$1 ").into_owned();
        if next == text {
            return text;
        }
        text = next;
    }
}

/// What the model is told about audio tags, at the user's intensity.
///
/// Fixed text per level, so the prompt prefix stays cache-stable for a whole
/// call. Like every prompt the assistant sees, it uses no dashes as clause
/// breaks: a model mirrors the punctuation of its instructions, and the app's
/// style rules forbid them in replies.
pub fn prompt_section(intensity: AudioTagIntensity) -> String {
    let level = match intensity {
        AudioTagIntensity::Subtle => SUBTLE,
        AudioTagIntensity::Balanced => BALANCED,
        AudioTagIntensity::Theatrical => THEATRICAL,
    };
    format!("{AUDIO_TAGS_GUIDE}\n{level}")
}

/// The rules every intensity shares. The tag vocabulary follows ElevenLabs' own
/// v4 prompting guide and tag list; tags not in it still work, since the model
/// reads them as natural-language direction.
const AUDIO_TAGS_GUIDE: &str = "## Voice performance with audio tags\n\
Your reply is spoken by an expressive ElevenLabs voice that performs audio tags: short cues in square brackets, like [laughs] or [whispers]. The voice acts a tag out or plays it as a sound, and never says the bracketed words aloud. Use them to make the conversation feel alive.\n\
- Put a tag right before the words it should color (\"[excited] You actually did it!\") or right after a line for a reaction (\"That was the plan all along. [chuckles]\"). A delivery tag carries on until the next one, so add a new tag only when the delivery changes, and give each clause at most one.\n\
- Emotion and delivery: [excited], [warmly], [curious], [thoughtful], [sarcastic], [mischievously], [nervous], [relieved], [whispers], [shouts], [softly], [slowly], [rushed], [dramatically].\n\
- Human sounds: [laughs], [chuckles], [giggles], [laughs harder], [sighs], [exhales], [gasps], [clears throat], [gulps], [sniffs].\n\
- Sound effects and atmosphere: [applause], [clapping], [crowd cheering], [drumroll], [footsteps], [door creaks], [thunder rumbling], [explosion], [gunshot].\n\
- Character: [strong British accent], [pirate voice], [sings], [like a sports commentator], [movie trailer voice].\n\
- You can combine cues in one tag, like [whispering, playful], or describe a delivery in plain words, like [low, gravelly voice]. When you mean the voice, word the tag as a voice quality, and when you mean a sound effect, name the sound, so the two are not confused.\n\
- When the user asks for a scene or an effect, such as being cheered on by a crowd, a drumroll, or a spooky story, build it with sound effect tags around your words: \"[crowd cheering] [applause] Ladies and gentlemen... [drumroll] here is the champion!\"\n\
- An ellipsis (...) adds a pause or hesitation, and a word in CAPITALS gets extra emphasis.\n\
- Write tags in English and in lowercase, even when you reply in another language, and only in square brackets. Never use parentheses, asterisks, or emoji for actions, and never put words that should be spoken inside brackets.\n\
- Only use cues that make a sound. Visual ones such as [smiles], [nods], [winks], or [grinning] do nothing.\n\
- Never mention, explain, or list the tags you use, and never put them inside code, links, or text the user asked for so they can paste it somewhere. Tags do not count toward the length of your reply.";

const SUBTLE: &str = "How much to perform: subtly. Most replies need no tags at all. Add one, or at most two, only where a reaction is obvious, like a laugh at something funny or a sigh at bad news. Use sound effects only when the user asks for them.";

const BALANCED: &str = "How much to perform: like an expressive person in conversation. Use a few tags per reply, roughly one for every two or three sentences, where they fit the moment. Bring in sound effects when the user asks for them or a moment clearly invites one, such as a celebration.";

const THEATRICAL: &str = "How much to perform: theatrically, like a voice actor. Give most sentences a delivery or reaction tag, let the emotion shift freely, and layer sound effects and atmosphere to set the scene whenever it adds fun or drama. The answer itself stays as useful and on topic as ever; the performance is how you say it, not a replacement for it.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;

    fn elevenlabs(model: &str, switch_on: bool) -> AppSettings {
        let mut settings = get_default_settings();
        settings.assistant_tts_engine = "elevenlabs".to_string();
        settings.assistant_tts_model = model.to_string();
        settings.assistant_tts_elevenlabs_audio_tags = switch_on;
        settings
    }

    #[test]
    fn tags_need_a_v3_or_later_model() {
        for model in [
            "eleven_v3",
            "eleven_v3_conversational",
            "eleven_v4",
            "eleven_v4_turbo",
            "ELEVEN_V4",
            " eleven_v5 ",
        ] {
            assert!(model_supports_tags(model), "{model}");
        }
        for model in [
            "eleven_flash_v2_5",
            "eleven_multilingual_v2",
            "eleven_turbo_v2",
            "eleven_monolingual_v1",
            "",
            // A version number that is not a `v<N>` segment of its own.
            "eleven_v2_3",
            "eleven3",
        ] {
            assert!(!model_supports_tags(model), "{model}");
        }
    }

    /// The gate asks about the model that is actually requested, so an empty
    /// field (and the OpenAI default left over from switching engines) reads as
    /// ElevenLabs' own default, which does not perform tags.
    #[test]
    fn the_gate_uses_the_requested_model() {
        assert_eq!(
            elevenlabs_model(&elevenlabs("", true)),
            crate::tts::ELEVENLABS_DEFAULT_MODEL
        );
        assert_eq!(
            elevenlabs_model(&elevenlabs("gpt-4o-mini-tts", true)),
            crate::tts::ELEVENLABS_DEFAULT_MODEL
        );
        assert!(!active(&elevenlabs("", true)));
        assert!(!active(&elevenlabs("gpt-4o-mini-tts", true)));
        assert!(active(&elevenlabs("eleven_v4", true)));
    }

    #[test]
    fn the_gate_needs_the_switch_and_the_engine() {
        assert!(!active(&elevenlabs("eleven_v4", false)));
        let mut other_engine = elevenlabs("eleven_v4", true);
        other_engine.assistant_tts_engine = "kokoro".to_string();
        assert!(!active(&other_engine));
        other_engine.assistant_tts_engine = "openai".to_string();
        assert!(!active(&other_engine));
    }

    #[test]
    fn the_feature_is_off_on_a_fresh_install() {
        let settings = get_default_settings();
        assert!(!settings.assistant_tts_elevenlabs_audio_tags);
        assert_eq!(
            settings.assistant_tts_elevenlabs_audio_tag_intensity,
            AudioTagIntensity::Balanced
        );
        assert!(!active(&settings));
    }

    fn spoken(text: &str) -> String {
        strip(text).split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn strip_removes_tags_wherever_they_sit() {
        assert_eq!(spoken("[laughs] That is great."), "That is great.");
        assert_eq!(spoken("That is great. [chuckles]"), "That is great.");
        assert_eq!(
            spoken("Well [sighs] it happens. [whispering, playful] Or does it?"),
            "Well it happens. Or does it?"
        );
        assert_eq!(spoken("(fine) [laughs] okay"), "(fine) okay");
        assert_eq!(spoken("[strong British accent] Cheers."), "Cheers.");
    }

    /// A run of touching tags is removed whole. A pass only removes the first
    /// tag of a run (the rest sit against a `]`), so the old fixed two passes
    /// left the third of `[gasps][laughs][applause]` to be read aloud.
    #[test]
    fn strip_removes_a_run_of_touching_tags() {
        assert_eq!(spoken("[laughs][applause] Thanks!"), "Thanks!");
        assert_eq!(spoken("[gasps][laughs][applause] Thanks!"), "Thanks!");
        assert_eq!(
            spoken("Wow [gasps][laughs][applause][crowd cheering] yes"),
            "Wow yes"
        );
    }

    #[test]
    fn strip_leaves_other_brackets_alone() {
        for text in [
            "Press [Ctrl] and [Shift].",
            "See note [1] and [TODO] later.",
            "- [x] done",
            "- [ ] open",
            "Read items[idx] and map[key][value] in a loop.",
            "Use arr[i] here.",
            "Nothing to strip here.",
        ] {
            assert_eq!(strip(text), text, "{text}");
        }
    }

    /// The speech sanitizer turns a Markdown link into its label before tags
    /// are stripped, so a link survives as words rather than vanishing.
    #[test]
    fn a_link_label_is_spoken_and_a_tag_is_not() {
        let text = "[laughs] Read [the docs](https://example.com) first.";
        let spoken = crate::tts::sanitize_for_speech(text);
        assert_eq!(spoken, "Read the docs first.");
        assert_eq!(
            crate::tts::sanitize_for_speech_for(&elevenlabs("eleven_v4", true), text),
            "[laughs] Read the docs first."
        );
    }

    /// A reply that is nothing but tags stays silent on an engine that cannot
    /// perform them, instead of falling back to reading the raw tags.
    #[test]
    fn a_tags_only_reply_is_silent_without_the_feature() {
        assert_eq!(crate::tts::sanitize_for_speech("[laughs] [applause]"), "");
        assert_eq!(
            crate::tts::sanitize_for_speech_for(
                &elevenlabs("eleven_v4", true),
                "[laughs] [applause]"
            ),
            "[laughs] [applause]"
        );
    }

    #[test]
    fn each_intensity_has_its_own_fixed_paragraph() {
        let subtle = prompt_section(AudioTagIntensity::Subtle);
        let balanced = prompt_section(AudioTagIntensity::Balanced);
        let theatrical = prompt_section(AudioTagIntensity::Theatrical);
        for section in [&subtle, &balanced, &theatrical] {
            assert!(section.starts_with(AUDIO_TAGS_GUIDE));
        }
        assert!(subtle.ends_with(SUBTLE));
        assert!(balanced.ends_with(BALANCED));
        assert!(theatrical.ends_with(THEATRICAL));
        // Cache-stable: the same level always produces the same text.
        assert_eq!(balanced, prompt_section(AudioTagIntensity::Balanced));
    }

    /// The assistant's replies may not use dashes as clause breaks, and a model
    /// mirrors the punctuation of its instructions.
    #[test]
    fn the_prompt_uses_no_dashes() {
        for intensity in [
            AudioTagIntensity::Subtle,
            AudioTagIntensity::Balanced,
            AudioTagIntensity::Theatrical,
        ] {
            let section = prompt_section(intensity);
            assert!(!section.contains('—'), "em dash in {intensity:?}");
            assert!(!section.contains('–'), "en dash in {intensity:?}");
            assert!(!section.contains(" - "), "spaced hyphen in {intensity:?}");
        }
    }
}
