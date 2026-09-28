use rubato::{FftFixedIn, Resampler};
use std::time::Duration;

// Make this a constant you can tweak
const RESAMPLER_CHUNK_SIZE: usize = 1024;

pub struct FrameResampler {
    resampler: Option<FftFixedIn<f32>>,
    chunk_in: usize,
    in_buf: Vec<f32>,
    frame_samples: usize,
    pending: Vec<f32>,
    in_hz: usize,
    out_hz: usize,
    /// Samples fed into / produced by the inner resampler this session.
    /// `finish()` uses the pair to know how much real audio (~10-30 ms) is
    /// still held in the resampler's delay line when a recording ends.
    in_count: usize,
    out_count: usize,
}

impl FrameResampler {
    pub fn new(in_hz: usize, out_hz: usize, frame_dur: Duration) -> Self {
        let frame_samples = ((out_hz as f64 * frame_dur.as_secs_f64()).round()) as usize;
        assert!(frame_samples > 0, "frame duration too short");

        // Use fixed chunk size instead of GCD-based
        let chunk_in = RESAMPLER_CHUNK_SIZE;

        let resampler = (in_hz != out_hz).then(|| {
            FftFixedIn::<f32>::new(in_hz, out_hz, chunk_in, 1, 1)
                .expect("Failed to create resampler")
        });

        Self {
            resampler,
            chunk_in,
            in_buf: Vec::with_capacity(chunk_in),
            frame_samples,
            pending: Vec::with_capacity(frame_samples),
            in_hz,
            out_hz,
            in_count: 0,
            out_count: 0,
        }
    }

    /// Clear all buffered state so a fresh recording can't inherit samples
    /// from a previous one.
    ///
    /// `FrameResampler` is created once per audio session and reused across
    /// every recording in that session (the worker thread in `run_consumer`
    /// lives for as long as the microphone stream is open). Without clearing
    /// its state on each new recording, three buffers leak the tail of the
    /// previous recording into the start of the next one:
    ///   * `in_buf`    — a partially-filled input chunk not yet resampled,
    ///   * `pending`   — a partially-filled 16 kHz output frame not yet emitted,
    ///   * the rubato `FftFixedIn` FFT **overlap** buffers (internal), which
    ///     retain ~one chunk of prior audio by design of overlap-add.
    ///
    /// The overlap in particular corrupted the first ~30 ms of each new
    /// recording — lost/garbled first words, and occasional stale text
    /// fragments bleeding across sessions.
    ///
    /// Backport of Handy PR #1344 ("reset resampler state between recordings").
    /// Call at the start of every recording (see `run_consumer`'s `Cmd::Start`).
    pub fn reset(&mut self) {
        self.in_buf.clear();
        self.pending.clear();
        self.in_count = 0;
        self.out_count = 0;
        if let Some(ref mut resampler) = self.resampler {
            // Zero the FFT overlap buffers so no prior-recording audio remains.
            resampler.reset();
        }
    }

    pub fn push(&mut self, mut src: &[f32], mut emit: impl FnMut(&[f32])) {
        if self.resampler.is_none() {
            self.emit_frames(src, &mut emit);
            return;
        }
        self.in_count += src.len();

        while !src.is_empty() {
            let space = self.chunk_in - self.in_buf.len();
            let take = space.min(src.len());
            self.in_buf.extend_from_slice(&src[..take]);
            src = &src[take..];

            if self.in_buf.len() == self.chunk_in {
                // let start = std::time::Instant::now();
                if let Ok(out) = self
                    .resampler
                    .as_mut()
                    .unwrap()
                    .process(&[&self.in_buf[..]], None)
                {
                    // let duration = start.elapsed();
                    // log::debug!("Resampler took: {:?}", duration);
                    self.out_count += out[0].len();
                    self.emit_frames(&out[0], &mut emit);
                }
                self.in_buf.clear();
            }
        }
    }

    pub fn finish(&mut self, mut emit: impl FnMut(&[f32])) {
        if self.resampler.is_some() {
            // Process any remaining input samples (rubato pads internally).
            if !self.in_buf.is_empty() {
                let result = self
                    .resampler
                    .as_mut()
                    .unwrap()
                    .process_partial(Some(&[&self.in_buf[..]]), None);
                if let Ok(out) = result {
                    self.out_count += out[0].len();
                    self.emit_frames(&out[0], &mut emit);
                }
            }
            // Drop the consumed input tail so it can't leak into the next
            // recording: a full `in_buf` would be flushed by the next `push()`
            // as stale audio. Backport of the Handy PR #1582 follow-up commit.
            self.in_buf.clear();

            // Output lags input by `output_delay()` samples, so the last real
            // audio has only emerged once `in * ratio + delay` samples are out.
            // Without this drain the final ~10-30 ms of every recording stayed
            // inside the delay line — a clipped last syllable, most noticeable
            // on a push-to-talk release. Feed empty chunks until then and trim
            // the synthetic remainder. Backport of Handy PR #1958.
            if self.in_count > 0 {
                let delay = self.resampler.as_ref().unwrap().output_delay();
                let expected = self.in_count * self.out_hz / self.in_hz + delay;
                let mut rounds = 0;
                while self.out_count < expected && rounds < 8 {
                    rounds += 1;
                    let result = self
                        .resampler
                        .as_mut()
                        .unwrap()
                        .process_partial::<&[f32]>(None, None);
                    match result {
                        Ok(out) => {
                            let take = (expected - self.out_count).min(out[0].len());
                            self.out_count += take;
                            self.emit_frames(&out[0][..take], &mut emit);
                        }
                        Err(_) => break,
                    }
                }
            }
        }

        // Emit any remaining pending frame (padded with zeros)
        if !self.pending.is_empty() {
            self.pending.resize(self.frame_samples, 0.0);
            emit(&self.pending);
            self.pending.clear();
        }
    }

    fn emit_frames(&mut self, mut data: &[f32], emit: &mut impl FnMut(&[f32])) {
        while !data.is_empty() {
            let space = self.frame_samples - self.pending.len();
            let take = space.min(data.len());
            self.pending.extend_from_slice(&data[..take]);
            data = &data[take..];

            if self.pending.len() == self.frame_samples {
                emit(&self.pending);
                self.pending.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameResampler, RESAMPLER_CHUNK_SIZE};
    use std::time::Duration;

    /// Push silence ending in a 200-sample 0.5 burst, then assert `finish()`
    /// recovers the burst and emits floor(input*ratio) + output_delay samples,
    /// padded to whole 480-sample frames.
    fn assert_tail_burst_flushed(in_hz: usize, input_len: usize, expected_out: usize) {
        let mut rs = FrameResampler::new(in_hz, 16000, Duration::from_millis(30));
        let mut input = vec![0.0f32; input_len];
        input[input_len - 200..].fill(0.5);

        let mut out = Vec::new();
        rs.push(&input, |frame| out.extend_from_slice(frame));
        rs.finish(|frame| out.extend_from_slice(frame));

        let max_abs = out.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(
            max_abs > 0.3,
            "tail burst was lost in the resampler, max_abs={max_abs}"
        );
        assert_eq!(out.len(), expected_out);
    }

    #[test]
    fn finish_flushes_resampler_delay() {
        // Exact chunks keep in_buf empty, so the burst survives only via the
        // delay-line drain. 4096 in -> 1365 real out + 171 delay -> 1920 framed.
        assert_tail_burst_flushed(48000, 4 * RESAMPLER_CHUNK_SIZE, 1920);
    }

    #[test]
    fn finish_flushes_resampler_delay_44100() {
        // fft_size_in (1323) exceeds the 1024 chunk, so the drain must survive
        // a zero-output round. 4096 in -> 1486 real out + 240 delay -> 1920.
        assert_tail_burst_flushed(44100, 4 * RESAMPLER_CHUNK_SIZE, 1920);
    }

    #[test]
    fn finish_flushes_unaligned_tail() {
        // Ends mid-chunk: partial-chunk path plus delay drain together.
        // 4396 in -> 1465 real out + 171 delay -> 1920 framed.
        assert_tail_burst_flushed(48000, 4 * RESAMPLER_CHUNK_SIZE + 300, 1920);
    }

    // 48 kHz -> 16 kHz, 30 ms frames (=> 480 output samples per frame).
    // A non-1:1 ratio ensures the rubato `FftFixedIn` resampler is actually
    // created, so the overlap-buffer behaviour is exercised.
    fn make() -> FrameResampler {
        FrameResampler::new(48_000, 16_000, Duration::from_millis(30))
    }

    #[test]
    fn reset_clears_in_buf_and_pending() {
        let mut rs = make();

        // A partial chunk (fewer than `chunk_in` samples) stays buffered in
        // `in_buf` without triggering a resample.
        rs.push(&vec![0.5_f32; 100], |_| {});
        assert!(
            !rs.in_buf.is_empty(),
            "precondition: in_buf should hold the partial input chunk"
        );

        // Force a partially-filled output frame so we can prove it's cleared.
        rs.pending.push(0.25);
        assert!(!rs.pending.is_empty(), "precondition: pending is non-empty");

        rs.reset();

        assert!(rs.in_buf.is_empty(), "reset() must clear in_buf");
        assert!(rs.pending.is_empty(), "reset() must clear pending");
    }

    #[test]
    fn reset_zeroes_rubato_overlap() {
        let mut rs = make();

        // Push a loud sine wave so the FFT overlap buffers fill with energy.
        let sine: Vec<f32> = (0..8_192).map(|i| (i as f32 * 0.15).sin()).collect();
        rs.push(&sine, |_| {});

        // Reset should zero the rubato overlap buffers (and our own buffers).
        rs.reset();

        // Now feed pure silence. If the overlap leaked, the first output frame
        // would still carry a decaying tail of the previous sine wave.
        let mut max_abs = 0.0_f32;
        rs.push(&vec![0.0_f32; 8_192], |frame| {
            for &s in frame {
                max_abs = max_abs.max(s.abs());
            }
        });

        assert!(
            max_abs < 1e-6,
            "resampler overlap leaked into the next recording (max abs = {max_abs})"
        );
    }

    #[test]
    fn back_to_back_recordings_do_not_bleed() {
        // End-to-end shape of the crosstalk fix: recording A is loud and is
        // finished (draining the tail), then reset() begins recording B, which
        // is pure silence and must produce silent output.
        let mut rs = make();

        let sine: Vec<f32> = (0..16_000).map(|i| (i as f32 * 0.2).sin()).collect();
        rs.push(&sine, |_| {});
        rs.finish(|_| {});

        // Start of recording B — the reset run_consumer performs on Cmd::Start.
        rs.reset();

        let mut max_abs = 0.0_f32;
        rs.push(&vec![0.0_f32; 16_000], |frame| {
            for &s in frame {
                max_abs = max_abs.max(s.abs());
            }
        });
        rs.finish(|frame| {
            for &s in frame {
                max_abs = max_abs.max(s.abs());
            }
        });

        assert!(
            max_abs < 1e-6,
            "recording A bled into recording B (max abs = {max_abs})"
        );
    }
}
