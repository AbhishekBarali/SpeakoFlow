use anyhow::Result;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use vad_rs::Vad;

use super::{SpeechScorer, VadFrame, VoiceActivityDetector, FRAME_SAMPLES};
use crate::audio_toolkit::constants;

pub struct SileroVad {
    engine: Vad,
    threshold: f32,
}

impl SileroVad {
    pub fn new<P: AsRef<Path>>(model_path: P, threshold: f32) -> Result<Self> {
        if !(0.0..=1.0).contains(&threshold) {
            anyhow::bail!("threshold must be between 0.0 and 1.0");
        }

        Ok(Self {
            engine: Vad::new(&model_path, constants::WHISPER_SAMPLE_RATE as usize)
                .map_err(|e| anyhow::anyhow!("Failed to create VAD: {e}"))?,
            threshold,
        })
    }

    /// Silero's speech probability for one frame.
    fn probability(&mut self, frame: &[f32]) -> Result<f32> {
        if frame.len() != FRAME_SAMPLES {
            anyhow::bail!("expected {FRAME_SAMPLES} samples, got {}", frame.len());
        }
        let result = self
            .engine
            .compute(frame)
            .map_err(|e| anyhow::anyhow!("Silero VAD error: {e}"))?;
        Ok(result.prob)
    }
}

impl VoiceActivityDetector for SileroVad {
    fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> Result<VadFrame<'a>> {
        if self.probability(frame)? > self.threshold {
            Ok(VadFrame::Speech(frame))
        } else {
            Ok(VadFrame::Noise)
        }
    }
}

impl SpeechScorer for SileroVad {
    fn frame_samples(&self) -> usize {
        FRAME_SAMPLES
    }

    fn score(&mut self, frames: &[f32]) -> Result<Vec<f32>> {
        if !frames.len().is_multiple_of(FRAME_SAMPLES) {
            anyhow::bail!(
                "expected whole {FRAME_SAMPLES}-sample frames, got {} samples",
                frames.len()
            );
        }
        // vad-rs unwraps the model's outputs. Scoring only places cuts, so a
        // panic there must become an error the chunker can fall back from,
        // not unwind through the transcription that asked for it.
        catch_unwind(AssertUnwindSafe(|| {
            frames
                .chunks(FRAME_SAMPLES)
                .map(|frame| self.probability(frame))
                .collect::<Result<Vec<f32>>>()
        }))
        .unwrap_or_else(|_| Err(anyhow::anyhow!("Silero VAD panicked while scoring")))
    }
}
