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
                    self.emit_frames(&out[0], &mut emit);
                }
                self.in_buf.clear();
            }
        }
    }

    pub fn finish(&mut self, mut emit: impl FnMut(&[f32])) {
        // Process any remaining input samples
        if let Some(ref mut resampler) = self.resampler {
            if !self.in_buf.is_empty() {
                // Pad with zeros to reach chunk size
                self.in_buf.resize(self.chunk_in, 0.0);
                if let Ok(out) = resampler.process(&[&self.in_buf[..]], None) {
                    self.emit_frames(&out[0], &mut emit);
                }
            }
            // Clear the (now zero-padded) input tail so it can't leak into the
            // next recording. `finish()` previously left `in_buf` full at
            // `chunk_in`, which the next `push()` would flush as stale audio.
            // Backport of the Handy PR #1582 follow-up commit.
            self.in_buf.clear();
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
    use super::FrameResampler;
    use std::time::Duration;

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
