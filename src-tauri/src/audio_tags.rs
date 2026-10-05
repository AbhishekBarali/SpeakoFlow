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
    // `replace_all` does not revisit the character a match consumed, so two
    // touching tags (`[laughs][applause]`) need a second pass for the second.
    let once = TAG.replace_all(text, "$1 ");
    TAG.replace_all(&once, "$1 ").into_owned()
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
