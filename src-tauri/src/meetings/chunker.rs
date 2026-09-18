//! Cutting a continuous audio stream into transcribable chunks.
//!
//! A dictation is one utterance: record, stop, transcribe once. A meeting never
//! stops, so something has to decide where one transcript segment ends and the
//! next begins. That decision is this module, and it is deliberately pure —
//! frames in, chunks out, no audio devices, no models, no clock — because every
//! interesting failure here (words cut in half, silence transcribed into
//! hallucinated text, timestamps drifting apart between the two streams) is a
//! bug in the decision rather than in the plumbing, and a pure function can be
//! tested for all of them.
//!
//! One accumulator is used **per stream**. The microphone and system audio are
//! chunked independently, which is what allows the two sides of a call to be
//! transcribed without one waiting on the other.

use crate::audio_toolkit::constants::WHISPER_SAMPLE_RATE;

/// Frames arriving from the capture layer are 30 ms.
const FRAME_MS: i64 = 30;

/// RMS below this counts as silence.
///
/// Deliberately low. Getting this wrong in the quiet direction merely delays a
/// chunk boundary; getting it wrong in the loud direction clips speech, and
/// room noise, fan noise and line hiss all sit above a naively "silent" level.
const DEFAULT_SILENCE_RMS: f32 = 0.005;

/// How much continuous silence closes a chunk.
///
/// 1.5 s rather than something snappier like 400 ms. A shorter window cuts on
/// the natural pauses *inside* a sentence — after a comma, mid-list, while
/// someone thinks — and hands the transcriber fragments with no grammatical
/// context, which measurably degrades the text it produces. Meetily converged on
/// the same figure after shipping something shorter.
const DEFAULT_SILENCE_HOLD_MS: i64 = 1_500;

/// Chunks shorter than this are discarded rather than transcribed.
///
/// A quarter second cannot contain a word worth keeping, but it can absolutely
/// contain a keyboard click or a door, and Whisper answers short noise with
/// confident nonsense ("Thank you.", "Yeah.", subtitle credits).
const DEFAULT_MIN_CHUNK_MS: i64 = 250;

/// Force a chunk boundary after this long regardless of silence.
///
/// Without a cap, one person talking uninterrupted for ten minutes produces no
/// transcript at all until they pause — the live view would appear frozen
/// through the most important part of a meeting. 30 s also bounds the buffer:
/// 30 s of 16 kHz mono `f32` is under 2 MB, so a stuck-open chunk cannot grow
/// into a memory problem over a three-hour call.
const DEFAULT_MAX_CHUNK_MS: i64 = 30_000;

/// A slice of audio ready to transcribe.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioChunk {
    /// Milliseconds from the start of the meeting.
    pub start_ms: i64,
    pub end_ms: i64,
    pub samples: Vec<f32>,
}

impl AudioChunk {
    pub fn duration_ms(&self) -> i64 {
        self.end_ms - self.start_ms
    }
}

/// Tunables, extracted so tests can use tiny values instead of real durations.
#[derive(Clone, Copy, Debug)]
pub struct ChunkConfig {
    pub silence_rms: f32,
    pub silence_hold_ms: i64,
    pub min_chunk_ms: i64,
    pub max_chunk_ms: i64,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            silence_rms: DEFAULT_SILENCE_RMS,
            silence_hold_ms: DEFAULT_SILENCE_HOLD_MS,
            min_chunk_ms: DEFAULT_MIN_CHUNK_MS,
            max_chunk_ms: DEFAULT_MAX_CHUNK_MS,
        }
    }
}

/// Accumulates frames and emits chunks at speech boundaries.
pub struct ChunkAccumulator {
    config: ChunkConfig,
    /// Audio held for the chunk currently being built.
    buffer: Vec<f32>,
    /// Total samples ever pushed. **This is the clock.**
    ///
    /// Position is derived from sample count rather than read from
    /// `Instant::now()` in the capture callback. Two reasons, and the second is
    /// the one that matters: a callback timestamp includes scheduling jitter of
    /// tens of milliseconds, and the microphone and system streams are serviced
    /// by different threads, so wall-clock stamps would slide the two halves of
    /// a conversation against each other. Counting samples anchors both streams
    /// to the same `t = 0` and keeps them mutually consistent.
    total_samples: u64,
    /// Samples that were already emitted or dropped, so the next chunk knows
    /// where it starts.
    consumed_samples: u64,
    /// Length of the current run of silent frames, in milliseconds.
    silence_run_ms: i64,
    /// Whether the current buffer contains anything above the silence floor.
    /// A buffer of pure silence is dropped rather than transcribed.
    has_voice: bool,
}

impl Default for ChunkAccumulator {
    fn default() -> Self {
        Self::new(ChunkConfig::default())
    }
}

impl ChunkAccumulator {
    pub fn new(config: ChunkConfig) -> Self {
        Self {
            config,
            buffer: Vec::new(),
            total_samples: 0,
            consumed_samples: 0,
            silence_run_ms: 0,
            has_voice: false,
        }
    }

    /// Feed one frame. Returns a chunk when a boundary is reached.
    ///
    /// Silence is buffered along with speech rather than discarded, so a chunk
    /// carries the natural pauses inside it — the transcriber uses them, and
    /// dropping them would splice unrelated words together.
    pub fn push(&mut self, frame: &[f32]) -> Option<AudioChunk> {
        if frame.is_empty() {
            return None;
        }

        self.total_samples += frame.len() as u64;
        self.buffer.extend_from_slice(frame);

        if rms(frame) >= self.config.silence_rms {
            self.has_voice = true;
            self.silence_run_ms = 0;
        } else {
            self.silence_run_ms += FRAME_MS;
        }

        let buffered_ms = samples_to_ms(self.buffer.len() as u64);

        // A long-enough silence closes the chunk, but only if there is speech in
        // it. Silence alone is not a chunk boundary, it is just silence — and
        // emitting it would ask the transcriber to invent words for nothing.
        let closed_by_silence =
            self.has_voice && self.silence_run_ms >= self.config.silence_hold_ms;

        // The cap fires regardless of speech content so the buffer stays bounded.
        let closed_by_length = buffered_ms >= self.config.max_chunk_ms;

        if closed_by_silence || closed_by_length {
            return self.take_chunk();
        }

        // Nothing but silence for longer than the hold: throw it away so a
        // muted participant or an idle call does not accumulate.
        if !self.has_voice && buffered_ms >= self.config.silence_hold_ms {
            self.discard_buffer();
        }

        None
    }

    /// Emit whatever remains. Call once when capture stops.
    ///
    /// Without this the final utterance of every meeting is lost — the speaker
    /// stops talking and the recording ends before the silence hold elapses,
    /// which is exactly what "and one more thing before we go" sounds like.
    pub fn flush(&mut self) -> Option<AudioChunk> {
        if !self.has_voice {
            self.discard_buffer();
            return None;
        }
        self.take_chunk()
    }

    /// Position of the end of the stream so far, in milliseconds.
    pub fn position_ms(&self) -> i64 {
        samples_to_ms(self.total_samples)
    }

    fn take_chunk(&mut self) -> Option<AudioChunk> {
        let start_ms = samples_to_ms(self.consumed_samples);
        let len = self.buffer.len() as u64;
        let end_ms = samples_to_ms(self.consumed_samples + len);

        let samples = std::mem::take(&mut self.buffer);
        self.consumed_samples += len;
        self.silence_run_ms = 0;
        let had_voice = std::mem::replace(&mut self.has_voice, false);

        if !had_voice || end_ms - start_ms < self.config.min_chunk_ms {
            return None;
        }

        Some(AudioChunk {
            start_ms,
            end_ms,
            samples,
        })
    }

    fn discard_buffer(&mut self) {
        self.consumed_samples += self.buffer.len() as u64;
        self.buffer.clear();
        self.silence_run_ms = 0;
        self.has_voice = false;
    }
}

/// Root-mean-square level of a frame.
fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f32 = frame.iter().map(|s| s * s).sum();
    (sum / frame.len() as f32).sqrt()
}

fn samples_to_ms(samples: u64) -> i64 {
    (samples as i64 * 1_000) / WHISPER_SAMPLE_RATE as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 30 ms at 16 kHz.
    const FRAME_LEN: usize = (WHISPER_SAMPLE_RATE as usize * FRAME_MS as usize) / 1_000;

    fn loud() -> Vec<f32> {
        vec![0.3; FRAME_LEN]
    }

    fn quiet() -> Vec<f32> {
        vec![0.0; FRAME_LEN]
    }

    /// Short holds so tests describe behaviour rather than wait for real time.
    fn test_config() -> ChunkConfig {
        ChunkConfig {
            silence_rms: DEFAULT_SILENCE_RMS,
            silence_hold_ms: 90, // three frames
            min_chunk_ms: 60,
            max_chunk_ms: 600,
        }
    }

    #[test]
    fn rms_distinguishes_speech_from_silence() {
        assert!(rms(&loud()) > DEFAULT_SILENCE_RMS);
        assert!(rms(&quiet()) < DEFAULT_SILENCE_RMS);
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn speech_followed_by_silence_emits_one_chunk() {
        let mut acc = ChunkAccumulator::new(test_config());

        for _ in 0..5 {
            assert!(acc.push(&loud()).is_none());
        }
        // Two frames of silence is not yet enough.
        assert!(acc.push(&quiet()).is_none());
        assert!(acc.push(&quiet()).is_none());
        // The third crosses the hold.
        let chunk = acc.push(&quiet()).expect("chunk closes after the hold");

        assert_eq!(chunk.start_ms, 0);
        assert_eq!(chunk.duration_ms(), 8 * FRAME_MS);
        assert_eq!(chunk.samples.len(), 8 * FRAME_LEN);
    }

    /// Pure silence must never reach the transcriber: Whisper answers noise with
    /// confident invented text.
    #[test]
    fn silence_alone_never_produces_a_chunk() {
        let mut acc = ChunkAccumulator::new(test_config());
        for _ in 0..50 {
            assert!(acc.push(&quiet()).is_none());
        }
    }

    /// An idle call must not accumulate memory for its whole duration.
    #[test]
    fn sustained_silence_is_discarded_rather_than_buffered() {
        let mut acc = ChunkAccumulator::new(test_config());
        for _ in 0..100 {
            acc.push(&quiet());
        }
        assert!(
            acc.buffer.len() < 10 * FRAME_LEN,
            "silence should not pile up"
        );
        // The timeline still advanced, so later speech is stamped correctly.
        assert_eq!(acc.position_ms(), 100 * FRAME_MS);
    }

    /// Silence before speech must not be charged to the speech that follows, but
    /// the timeline must still account for it.
    #[test]
    fn timestamps_survive_discarded_silence() {
        let mut acc = ChunkAccumulator::new(test_config());
        for _ in 0..10 {
            acc.push(&quiet());
        }
        for _ in 0..3 {
            acc.push(&loud());
        }
        for _ in 0..2 {
            acc.push(&quiet());
        }
        let chunk = acc.push(&quiet()).expect("speech chunk");

        // The discarded silence is behind us, so this chunk starts late.
        assert!(
            chunk.start_ms >= 9 * FRAME_MS,
            "chunk started at {}ms, expected it to follow the discarded silence",
            chunk.start_ms
        );
    }

    /// An uninterrupted monologue must still produce transcript during the
    /// meeting rather than nothing until the speaker finally pauses.
    #[test]
    fn a_long_monologue_is_cut_by_the_length_cap() {
        let cfg = test_config();
        let mut acc = ChunkAccumulator::new(cfg);

        let frames_to_cap = (cfg.max_chunk_ms / FRAME_MS) as usize;
        let mut emitted = None;
        for _ in 0..frames_to_cap {
            if let Some(chunk) = acc.push(&loud()) {
                emitted = Some(chunk);
                break;
            }
        }

        let chunk = emitted.expect("the cap must force a boundary");
        assert!(chunk.duration_ms() >= cfg.max_chunk_ms);
    }

    /// Consecutive chunks must tile the timeline without gaps or overlap,
    /// otherwise the two streams cannot be interleaved coherently.
    #[test]
    fn consecutive_chunks_are_contiguous() {
        let mut acc = ChunkAccumulator::new(test_config());
        let mut chunks = Vec::new();

        for _ in 0..3 {
            for _ in 0..4 {
                if let Some(c) = acc.push(&loud()) {
                    chunks.push(c);
                }
            }
            for _ in 0..3 {
                if let Some(c) = acc.push(&quiet()) {
                    chunks.push(c);
                }
            }
        }

        assert!(
            chunks.len() >= 2,
            "expected several chunks, got {}",
            chunks.len()
        );
        for pair in chunks.windows(2) {
            assert_eq!(
                pair[0].end_ms, pair[1].start_ms,
                "chunks must tile: {:?} then {:?}",
                pair[0], pair[1]
            );
        }
    }

    /// The last thing said before the recording stops is often the most
    /// important, and it never gets its trailing silence.
    #[test]
    fn flush_emits_the_final_utterance() {
        let mut acc = ChunkAccumulator::new(test_config());
        for _ in 0..4 {
            acc.push(&loud());
        }
        let chunk = acc.flush().expect("flush must not drop trailing speech");
        assert_eq!(chunk.samples.len(), 4 * FRAME_LEN);
    }

    #[test]
    fn flush_on_silence_yields_nothing() {
        let mut acc = ChunkAccumulator::new(test_config());
        acc.push(&quiet());
        assert!(acc.flush().is_none());
    }

    #[test]
    fn flush_on_an_empty_stream_yields_nothing() {
        let mut acc = ChunkAccumulator::new(test_config());
        assert!(acc.flush().is_none());
    }

    /// A click or a door is not speech, and transcribing it produces invented
    /// words rather than nothing.
    #[test]
    fn chunks_below_the_minimum_are_dropped() {
        let cfg = ChunkConfig {
            min_chunk_ms: 10_000,
            ..test_config()
        };
        let mut acc = ChunkAccumulator::new(cfg);
        acc.push(&loud());
        assert!(acc.flush().is_none(), "a 30ms blip must not be transcribed");
    }

    #[test]
    fn empty_frames_are_ignored() {
        let mut acc = ChunkAccumulator::new(test_config());
        assert!(acc.push(&[]).is_none());
        assert_eq!(acc.position_ms(), 0);
    }

    /// Sample-derived positions must be exact, since both streams rely on them
    /// to stay aligned with each other.
    #[test]
    fn position_tracks_sample_count_exactly() {
        let mut acc = ChunkAccumulator::new(test_config());
        for i in 1..=10 {
            acc.push(&loud());
            assert_eq!(acc.position_ms(), i * FRAME_MS);
        }
    }

    #[test]
    fn samples_to_ms_is_exact_at_the_frame_size() {
        assert_eq!(samples_to_ms(0), 0);
        assert_eq!(samples_to_ms(WHISPER_SAMPLE_RATE as u64), 1_000);
        assert_eq!(samples_to_ms(FRAME_LEN as u64), FRAME_MS);
    }
}
