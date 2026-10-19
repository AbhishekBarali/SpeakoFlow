use anyhow::Result;

use crate::audio_toolkit::constants;

/// Samples in one recorder frame: 30 ms of 16 kHz audio, the size the recorder
/// resamples to and the Silero detector judges.
pub const FRAME_SAMPLES: usize = (constants::WHISPER_SAMPLE_RATE * 30 / 1000) as usize;

/// Trailing silence kept after speech for a model that transcribes the whole
/// recording when it ends. Long enough to keep a soft last syllable, short
/// enough that pauses do not pile up in the audio the model is given.
pub const VAD_OFFLINE_HANGOVER_MS: u64 = 450;

/// Trailing silence kept after speech for a model that streams. A streaming
/// model decides where an utterance ends from the pauses it hears, and with
/// 450 ms every pause between sentences was cut to the same short gap, so it
/// ran sentences together. 1650 ms is upstream Handy's value
/// (`VAD_STREAMING_HANGOVER_MS`).
pub const VAD_STREAMING_HANGOVER_MS: u64 = 1650;

/// [`VAD_OFFLINE_HANGOVER_MS`] in recorder frames: the 15 frames the recorder
/// has always kept.
pub const VAD_OFFLINE_TAIL_FRAMES: usize =
    frames_for_duration_ms(VAD_OFFLINE_HANGOVER_MS, FRAME_SAMPLES);

/// Whole detector frames covering `duration_ms`, rounded up so a timing is
/// never shortened by the frame size.
pub const fn frames_for_duration_ms(duration_ms: u64, frame_samples: usize) -> usize {
    assert!(frame_samples > 0, "VAD frame size must be non-zero");
    let numerator = duration_ms * constants::WHISPER_SAMPLE_RATE as u64;
    let denominator = frame_samples as u64 * 1000;
    numerator.div_ceil(denominator) as usize
}

/// How much trailing silence a recording keeps after each stretch of speech,
/// chosen per recording from the model that will transcribe it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VadTail {
    /// A model that transcribes the recording once it ends.
    Offline,
    /// A model that streams.
    Streaming,
}

impl VadTail {
    /// The tail for a model, from whether it streams.
    pub fn for_model(streams: bool) -> Self {
        if streams {
            Self::Streaming
        } else {
            Self::Offline
        }
    }

    pub fn hangover_ms(self) -> u64 {
        match self {
            Self::Offline => VAD_OFFLINE_HANGOVER_MS,
            Self::Streaming => VAD_STREAMING_HANGOVER_MS,
        }
    }

    /// The tail in recorder frames.
    pub fn hangover_frames(self) -> usize {
        frames_for_duration_ms(self.hangover_ms(), FRAME_SAMPLES)
    }
}

pub enum VadFrame<'a> {
    /// Speech – may aggregate several frames (prefill + current + hangover)
    Speech(&'a [f32]),
    /// Non-speech (silence, noise). Down-stream code can ignore it.
    Noise,
}

impl<'a> VadFrame<'a> {
    #[inline]
    pub fn is_speech(&self) -> bool {
        matches!(self, VadFrame::Speech(_))
    }
}

pub trait VoiceActivityDetector: Send + Sync {
    /// Primary streaming API: feed one 30-ms frame, get keep/drop decision.
    fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> Result<VadFrame<'a>>;

    fn is_voice(&mut self, frame: &[f32]) -> Result<bool> {
        Ok(self.push_frame(frame)?.is_speech())
    }

    /// Set the trailing-silence tail, in frames, applied from the next frame
    /// on. Detectors without one ignore it.
    fn set_hangover_frames(&mut self, _frames: usize) {}

    /// How many frames in a row the detector has kept since it last heard
    /// voice: how far into its trailing-silence tail the last frame was. 0
    /// while voice is heard, and for a detector without a tail.
    fn tail_frames(&self) -> usize {
        0
    }

    fn reset(&mut self) {}
}

/// A detector's own speech probabilities, before any threshold or smoothing:
/// what long-form chunking cuts at.
pub trait SpeechScorer: Send {
    /// Samples in one frame; [`score`](Self::score) takes whole frames.
    fn frame_samples(&self) -> usize;

    /// One probability per frame of `frames`, which must be whole frames.
    /// Scoring carries on from the frames scored before, so a clip scored in
    /// pieces gets the same probabilities as scored in one go.
    fn score(&mut self, frames: &[f32]) -> Result<Vec<f32>>;
}

mod silero;
mod smoothed;

pub use silero::SileroVad;
pub use smoothed::SmoothedVad;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tails_in_recorder_frames() {
        assert_eq!(FRAME_SAMPLES, 480);
        // The recorder's long-standing 15-frame tail is exactly the offline one.
        assert_eq!(VadTail::Offline.hangover_frames(), 15);
        assert_eq!(VadTail::Streaming.hangover_frames(), 55);
        // Rounded up, never down.
        assert_eq!(frames_for_duration_ms(31, FRAME_SAMPLES), 2);
        assert_eq!(frames_for_duration_ms(1650, 512), 52);
    }

    #[test]
    fn a_streaming_model_keeps_the_longer_tail() {
        assert_eq!(VadTail::for_model(true), VadTail::Streaming);
        assert_eq!(VadTail::for_model(false), VadTail::Offline);
        assert_eq!(VadTail::Streaming.hangover_ms(), 1650);
        assert_eq!(VadTail::Offline.hangover_ms(), 450);
    }
}
