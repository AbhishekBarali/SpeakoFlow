//! How audio is decoded: the one place a model's traits and the settings
//! choose between streaming, long-form chunking and a single run.
//!
//! Ported from upstream Handy's `managers/transcription/policy.rs`. The
//! overlay style is deliberately not an input: it decides what the user is
//! shown while recording, never how the audio is decoded. Reading it here is
//! how the Minimal pill used to switch streaming off for a model built for it.

use crate::audio_toolkit::vad::VadTail;

/// What a model can do that decides how its audio is decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelTraits {
    /// Decodes audio incrementally as it is recorded.
    pub streaming: bool,
    /// Keeps its accuracy on audio longer than its native window in one run
    /// (Whisper today).
    pub long_form: bool,
}

impl ModelTraits {
    /// The loaded model's own traits: the authority once a model is loaded,
    /// as the transcription worker read them at load.
    pub fn of_loaded(info: &crate::engine_supervisor::LoadedInfo) -> Self {
        Self {
            streaming: info.capabilities.supports_streaming,
            long_form: info.supports_long_form,
        }
    }

    /// The same, from a model loaded in this process (the in-process live
    /// tests).
    #[cfg(test)]
    pub fn of(model: &transcribe_cpp::Model, caps: &transcribe_cpp::Capabilities) -> Self {
        Self {
            streaming: caps.supports_streaming,
            long_form: model.supports(transcribe_cpp::Feature::LongForm),
        }
    }

    /// The best guess before the model is loaded, from the model list. Whether
    /// it keeps its accuracy on long audio is not known yet, so it is taken
    /// not to: the worker decides again with the loaded model, and a wrong
    /// guess costs one idle worker.
    pub fn expected(streaming: bool) -> Self {
        Self {
            streaming,
            long_form: false,
        }
    }
}

/// How a recording is decoded while it is still going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveDecode {
    /// The model decodes the audio as it arrives.
    Stream,
    /// Long audio is cut at pauses and each chunk decoded once its cut is
    /// certain.
    Chunked,
    /// Nothing until the recording stops; then it is decoded in batch.
    None,
}

/// How a whole clip is decoded: a recording without a live result, a
/// re-transcribed History entry, a meeting chunk, or a cloud fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchDecode {
    /// Cut at pauses into chunks of about 30 s, each decoded in turn; a clip
    /// of 40 s or less is one chunk.
    Chunked,
    /// One run.
    Whole,
}

/// Streaming wins: a streaming model handles long audio itself. Otherwise
/// long audio is chunked unless the model keeps its accuracy on it.
pub fn live_decode(model: ModelTraits, long_form_chunking: bool) -> LiveDecode {
    if model.streaming {
        LiveDecode::Stream
    } else if long_form_chunking && !model.long_form {
        LiveDecode::Chunked
    } else {
        LiveDecode::None
    }
}

/// As live, except that a streaming model takes a whole clip in one run, as
/// it did before long-form chunking existed: its batch path is the same
/// streaming decoder, run over the whole clip.
pub fn batch_decode(model: ModelTraits, long_form_chunking: bool) -> BatchDecode {
    if long_form_chunking && !model.long_form && !model.streaming {
        BatchDecode::Chunked
    } else {
        BatchDecode::Whole
    }
}

/// How much silence the recorder keeps after speech, for the model that will
/// transcribe the recording.
pub fn vad_tail(model: ModelTraits) -> VadTail {
    VadTail::for_model(model.streaming)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STREAMING: ModelTraits = ModelTraits {
        streaming: true,
        long_form: false,
    };
    const LONG_FORM: ModelTraits = ModelTraits {
        streaming: false,
        long_form: true,
    };
    const SHORT_FORM: ModelTraits = ModelTraits {
        streaming: false,
        long_form: false,
    };

    #[test]
    fn a_streaming_model_always_streams() {
        assert_eq!(live_decode(STREAMING, true), LiveDecode::Stream);
        assert_eq!(live_decode(STREAMING, false), LiveDecode::Stream);
        // And takes a whole clip in one run.
        assert_eq!(batch_decode(STREAMING, true), BatchDecode::Whole);
    }

    #[test]
    fn only_a_short_form_model_is_chunked_and_only_when_on() {
        assert_eq!(live_decode(SHORT_FORM, true), LiveDecode::Chunked);
        assert_eq!(batch_decode(SHORT_FORM, true), BatchDecode::Chunked);
        assert_eq!(live_decode(SHORT_FORM, false), LiveDecode::None);
        assert_eq!(batch_decode(SHORT_FORM, false), BatchDecode::Whole);
        // Whisper handles long audio itself.
        assert_eq!(live_decode(LONG_FORM, true), LiveDecode::None);
        assert_eq!(batch_decode(LONG_FORM, true), BatchDecode::Whole);
    }

    #[test]
    fn before_loading_a_model_is_taken_to_need_chunks() {
        assert_eq!(ModelTraits::expected(false), SHORT_FORM);
        assert_eq!(
            live_decode(ModelTraits::expected(false), true),
            LiveDecode::Chunked
        );
        assert_eq!(
            live_decode(ModelTraits::expected(true), true),
            LiveDecode::Stream
        );
    }

    #[test]
    fn the_vad_tail_follows_streaming() {
        assert_eq!(vad_tail(STREAMING), VadTail::Streaming);
        assert_eq!(vad_tail(SHORT_FORM), VadTail::Offline);
        assert_eq!(vad_tail(LONG_FORM), VadTail::Offline);
    }
}
