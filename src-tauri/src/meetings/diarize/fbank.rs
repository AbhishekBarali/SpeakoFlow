//! Kaldi-compatible 80-bin log-mel filterbank features.
//!
//! The speaker-embedding model does not take audio; it takes features, and it
//! takes them in one specific dialect. The contract is WeSpeaker's own, from its
//! `infer_onnx.py`:
//!
//! ```text
//! waveform * 32768                      # trained on int16-scaled audio
//! kaldi.fbank(num_mel_bins=80, frame_length=25, frame_shift=10,
//!             dither=0.0, window_type='hamming', use_energy=False)
//! mat -= mat.mean(dim=0)                # CMN, and no CVN
//! ```
//!
//! Two of those are silent failures rather than loud ones, which is why this
//! module exists in-tree instead of borrowing a generic mel implementation. Get
//! the ×32768 scaling wrong and every feature is shifted by a constant; use
//! `povey` or `hann` instead of `hamming` and every filter is slightly off. In
//! neither case does anything error — the embeddings still come back, still
//! cluster, and are quietly worse. There is no symptom to chase, just a
//! diarization that is mediocre for no visible reason.
//!
//! It is written here rather than taken from a crate for two more reasons: it adds
//! no dependency at all (`rustfft` is already a direct one), and the alternatives
//! each cost something real — `knf-rs` is C++ plus bindgen/libclang, and
//! `mel_spec` would drag in a second `image` and a second `ndarray` for ~180 lines
//! of arithmetic.
//!
//! `transcribe_rs::features::mel::compute_mel` is public and very close, and is
//! the right thing to prototype against. It is the wrong thing to ship: it
//! interpolates its triangles in FFT-bin space (the librosa convention) rather
//! than in the mel domain (Kaldi's), and it does not remove the per-frame DC
//! offset. Depending on it would also mean a `transcribe-rs` bump could change
//! feature values underneath us with no test failing.

use std::sync::Arc;

use rustfft::{num_complex::Complex32, Fft, FftPlanner};

/// Frame geometry and mel warp. Defaults are the WeSpeaker contract.
#[derive(Clone, Debug)]
pub struct FbankConfig {
    pub sample_rate: u32,
    pub num_mel_bins: usize,
    pub frame_length_ms: f32,
    pub frame_shift_ms: f32,
    pub low_freq: f32,
    /// `None` means Nyquist.
    pub high_freq: Option<f32>,
    pub pre_emphasis: f32,
    /// Subtract the per-frame mean before pre-emphasis, as Kaldi does.
    pub remove_dc_offset: bool,
    /// Multiply the waveform by 32768 on the way in.
    ///
    /// Required, not optional: WeSpeaker was trained on int16-scaled audio and the
    /// app's samples are normalised `f32`. Skipping it shifts every log-mel value
    /// by `ln(32768)` — which the model has never seen, and which produces
    /// plausible-looking embeddings that discriminate worse.
    pub scale_to_int16: bool,
}

impl Default for FbankConfig {
    fn default() -> Self {
        Self {
            sample_rate: crate::audio_toolkit::constants::WHISPER_SAMPLE_RATE,
            num_mel_bins: 80,
            frame_length_ms: 25.0,
            frame_shift_ms: 10.0,
            low_freq: 20.0,
            high_freq: None,
            pre_emphasis: 0.97,
            remove_dc_offset: true,
            scale_to_int16: true,
        }
    }
}

impl FbankConfig {
    /// Samples per analysis frame.
    pub fn frame_length(&self) -> usize {
        ((self.sample_rate as f32 * self.frame_length_ms) / 1_000.0).round() as usize
    }

    /// Samples between consecutive frames.
    pub fn frame_shift(&self) -> usize {
        ((self.sample_rate as f32 * self.frame_shift_ms) / 1_000.0).round() as usize
    }

    /// FFT size: the frame length rounded up to a power of two, as Kaldi does.
    pub fn fft_size(&self) -> usize {
        self.frame_length().next_power_of_two()
    }

    /// How many frames `len` samples yields. Kaldi's `snip_edges=true` rule: only
    /// whole frames count, so a tail shorter than one frame is dropped rather than
    /// padded with silence the model would read as speech.
    pub fn num_frames(&self, len: usize) -> usize {
        let frame_length = self.frame_length();
        if len < frame_length {
            return 0;
        }
        (len - frame_length) / self.frame_shift() + 1
    }
}

/// Reusable feature extractor.
///
/// The FFT plan and the filterbank are built once and reused across every window
/// of a meeting — a three-hour recording is thousands of windows, and rebuilding a
/// 512-point plan and an 80×257 filterbank per window would dominate the cost of
/// the whole pass.
pub struct FbankComputer {
    config: FbankConfig,
    fft: Arc<dyn Fft<f32>>,
    /// `[num_mel_bins][fft_bins]` triangular weights.
    filters: Vec<Vec<f32>>,
    window: Vec<f32>,
}

impl FbankComputer {
    pub fn new(config: FbankConfig) -> Self {
        let fft_size = config.fft_size();
        let fft = FftPlanner::new().plan_fft_forward(fft_size);
        let filters = mel_filterbank(&config, fft_size);
        let window = hamming(config.frame_length());
        Self {
            config,
            fft,
            filters,
            window,
        }
    }

    pub fn num_mel_bins(&self) -> usize {
        self.config.num_mel_bins
    }

    pub fn config(&self) -> &FbankConfig {
        &self.config
    }

    /// Log-mel energies as `[num_frames][num_mel_bins]`, with CMN applied.
    ///
    /// Returns an empty vector when `samples` is shorter than one frame, rather
    /// than erroring: a caller that already filters short segments should not have
    /// to handle an error for the same condition twice.
    pub fn compute(&self, samples: &[f32]) -> Vec<Vec<f32>> {
        let frame_length = self.config.frame_length();
        let frame_shift = self.config.frame_shift();
        let frames = self.config.num_frames(samples.len());
        if frames == 0 {
            return Vec::new();
        }

        let scale = if self.config.scale_to_int16 {
            32_768.0
        } else {
            1.0
        };

        let fft_size = self.config.fft_size();
        let mut features = Vec::with_capacity(frames);
        let mut buffer = vec![Complex32::new(0.0, 0.0); fft_size];
        let mut frame = vec![0.0f32; frame_length];

        for index in 0..frames {
            let start = index * frame_shift;
            for (slot, sample) in frame.iter_mut().zip(&samples[start..start + frame_length]) {
                *slot = *sample * scale;
            }

            if self.config.remove_dc_offset {
                let mean = frame.iter().sum::<f32>() / frame_length as f32;
                for sample in frame.iter_mut() {
                    *sample -= mean;
                }
            }

            if self.config.pre_emphasis > 0.0 {
                // Backwards, so each sample still sees its unfiltered predecessor.
                // Kaldi replicates the first sample rather than assuming a zero
                // before it, which avoids a spurious step at the frame boundary.
                let coefficient = self.config.pre_emphasis;
                for i in (1..frame_length).rev() {
                    frame[i] -= coefficient * frame[i - 1];
                }
                frame[0] -= coefficient * frame[0];
            }

            for (slot, weight) in frame.iter_mut().zip(&self.window) {
                *slot *= weight;
            }

            buffer
                .iter_mut()
                .for_each(|value| *value = Complex32::new(0.0, 0.0));
            for (slot, sample) in buffer.iter_mut().zip(frame.iter()) {
                slot.re = *sample;
            }
            self.fft.process(&mut buffer);

            // Power spectrum over the non-redundant half. Kaldi's `use_power`
            // default is true, so this is magnitude *squared*.
            let bins = fft_size / 2 + 1;
            let power: Vec<f32> = buffer[..bins]
                .iter()
                .map(|value| value.re * value.re + value.im * value.im)
                .collect();

            let mut row = Vec::with_capacity(self.config.num_mel_bins);
            for filter in &self.filters {
                let energy: f32 = filter
                    .iter()
                    .zip(&power)
                    .map(|(weight, value)| weight * value)
                    .sum();
                // Floored at epsilon, as Kaldi does: a mel bin can legitimately
                // collect zero energy in silence, and `ln(0)` is -inf, which
                // poisons the mean subtraction below and then every embedding.
                row.push(energy.max(f32::EPSILON).ln());
            }
            features.push(row);
        }

        apply_cmn(&mut features);
        features
    }
}

/// Subtract the per-coefficient mean over time, in place. CMN without CVN.
///
/// Separate from [`FbankComputer::compute`] so it can be tested alone, and because
/// it is the step most likely to differ for another model — sherpa's 3D-Speaker
/// metadata, for instance, calls for a global mean rather than a per-utterance one.
pub fn apply_cmn(features: &mut [Vec<f32>]) {
    let Some(bins) = features.first().map(Vec::len) else {
        return;
    };
    let frames = features.len() as f32;
    for bin in 0..bins {
        let mean: f32 = features.iter().map(|row| row[bin]).sum::<f32>() / frames;
        for row in features.iter_mut() {
            row[bin] -= mean;
        }
    }
}

/// Kaldi's Hamming window: `0.54 - 0.46 cos(2πi/(N-1))`.
fn hamming(length: usize) -> Vec<f32> {
    if length <= 1 {
        return vec![1.0; length];
    }
    let denominator = (length - 1) as f32;
    (0..length)
        .map(|i| 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / denominator).cos())
        .collect()
}

/// Hertz to mel, Kaldi's warp: `1127 ln(1 + f/700)`.
fn hz_to_mel(hz: f32) -> f32 {
    1127.0 * (1.0 + hz / 700.0).ln()
}

/// Triangular mel filterbank, `[num_mel_bins][fft_bins]`.
///
/// Triangles are interpolated in the **mel** domain, which is what Kaldi and
/// `torchaudio.compliance.kaldi` do. Interpolating in FFT-bin space is the librosa
/// convention and produces measurably different weights — the single most likely
/// place for this module to be subtly and invisibly wrong.
fn mel_filterbank(config: &FbankConfig, fft_size: usize) -> Vec<Vec<f32>> {
    let bins = fft_size / 2 + 1;
    let nyquist = config.sample_rate as f32 / 2.0;
    let high_freq = config.high_freq.unwrap_or(nyquist).min(nyquist);
    let low_freq = config.low_freq.max(0.0).min(high_freq);

    let mel_low = hz_to_mel(low_freq);
    let mel_high = hz_to_mel(high_freq);
    let delta = (mel_high - mel_low) / (config.num_mel_bins + 1) as f32;
    let hz_per_bin = config.sample_rate as f32 / fft_size as f32;

    (0..config.num_mel_bins)
        .map(|bin| {
            let left = mel_low + bin as f32 * delta;
            let center = left + delta;
            let right = center + delta;

            (0..bins)
                .map(|k| {
                    let mel = hz_to_mel(k as f32 * hz_per_bin);
                    if mel <= left || mel >= right {
                        0.0
                    } else if mel <= center {
                        (mel - left) / (center - left)
                    } else {
                        (right - mel) / (right - center)
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn computer() -> FbankComputer {
        FbankComputer::new(FbankConfig::default())
    }

    /// A tone at 16 kHz: enough frames to exercise the whole path.
    fn tone(samples: usize, hz: f32) -> Vec<f32> {
        (0..samples)
            .map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / 16_000.0).sin() * 0.3)
            .collect()
    }

    /* ─────────────────────────── frame geometry ─────────────────────────── */

    /// The WeSpeaker contract in numbers: 25 ms / 10 ms at 16 kHz is 400 / 160,
    /// and Kaldi rounds the FFT up to 512.
    #[test]
    fn the_geometry_matches_the_model_contract() {
        let config = FbankConfig::default();
        assert_eq!(config.frame_length(), 400);
        assert_eq!(config.frame_shift(), 160);
        assert_eq!(config.fft_size(), 512);
        assert_eq!(config.num_mel_bins, 80);
    }

    /// `snip_edges=true`: only whole frames, so a partial tail is dropped rather
    /// than zero-padded into something the model reads as a quiet sound.
    #[test]
    fn only_whole_frames_are_counted() {
        let config = FbankConfig::default();
        assert_eq!(config.num_frames(0), 0);
        assert_eq!(config.num_frames(399), 0);
        assert_eq!(config.num_frames(400), 1);
        assert_eq!(config.num_frames(559), 1);
        assert_eq!(config.num_frames(560), 2);
        // One second is 98 frames at 25/10 ms with snipped edges.
        assert_eq!(config.num_frames(16_000), 98);
    }

    /* ─────────────────────────── output shape ─────────────────────────── */

    #[test]
    fn the_output_is_frames_by_mel_bins() {
        let features = computer().compute(&tone(16_000, 440.0));
        assert_eq!(features.len(), 98);
        for row in &features {
            assert_eq!(row.len(), 80);
        }
    }

    /// A caller that already drops short segments should not also have to handle
    /// an error for the same condition.
    #[test]
    fn audio_shorter_than_one_frame_yields_no_frames() {
        assert!(computer().compute(&[]).is_empty());
        assert!(computer().compute(&tone(100, 440.0)).is_empty());
    }

    /// Every value must be finite. An `-inf` from `ln(0)` in a silent bin would
    /// poison the mean subtraction and then every embedding computed from it — and
    /// it would do so without any error.
    #[test]
    fn silence_produces_finite_features() {
        let features = computer().compute(&vec![0.0f32; 16_000]);
        assert_eq!(features.len(), 98);
        for row in &features {
            for value in row {
                assert!(value.is_finite(), "silence produced {value}");
            }
        }
    }

    #[test]
    fn loud_audio_produces_finite_features() {
        let features = computer().compute(&vec![0.99f32; 8_000]);
        for row in &features {
            for value in row {
                assert!(value.is_finite(), "loud audio produced {value}");
            }
        }
    }

    /* ─────────────────────────────── CMN ─────────────────────────────── */

    /// After CMN each coefficient averages zero over time. That is the property
    /// the model was trained to expect, and it is cheap to assert exactly.
    #[test]
    fn cmn_zeroes_the_mean_of_every_coefficient() {
        let features = computer().compute(&tone(16_000, 300.0));
        let frames = features.len() as f32;
        for bin in 0..80 {
            let mean: f32 = features.iter().map(|row| row[bin]).sum::<f32>() / frames;
            assert!(mean.abs() < 1e-3, "bin {bin} has mean {mean} after CMN");
        }
    }

    #[test]
    fn cmn_on_an_empty_matrix_is_a_no_op() {
        let mut empty: Vec<Vec<f32>> = Vec::new();
        apply_cmn(&mut empty);
        assert!(empty.is_empty());
    }

    #[test]
    fn cmn_subtracts_the_column_mean() {
        let mut features = vec![vec![1.0, 10.0], vec![3.0, 20.0]];
        apply_cmn(&mut features);
        assert_eq!(features, vec![vec![-1.0, -5.0], vec![1.0, 5.0]]);
    }

    /* ───────────────────────── the mel filterbank ───────────────────────── */

    #[test]
    fn the_filterbank_has_one_triangle_per_mel_bin() {
        let config = FbankConfig::default();
        let filters = mel_filterbank(&config, config.fft_size());
        assert_eq!(filters.len(), 80);
        for filter in &filters {
            assert_eq!(filter.len(), config.fft_size() / 2 + 1);
        }
    }

    /// Every triangle must actually cover some FFT bins. A degenerate empty filter
    /// yields a constant log-epsilon channel, which contributes nothing and is
    /// invisible in the output.
    #[test]
    fn every_triangle_covers_at_least_one_bin() {
        let config = FbankConfig::default();
        for (index, filter) in mel_filterbank(&config, config.fft_size())
            .iter()
            .enumerate()
        {
            assert!(
                filter.iter().any(|weight| *weight > 0.0),
                "mel bin {index} collects no energy"
            );
        }
    }

    /// Triangles peak at 1.0 and never exceed it — a weight above one would
    /// amplify a band the model expects unamplified.
    #[test]
    fn triangle_weights_stay_within_zero_and_one() {
        let config = FbankConfig::default();
        for filter in mel_filterbank(&config, config.fft_size()) {
            for weight in filter {
                assert!(
                    (0.0..=1.0 + 1e-6).contains(&weight),
                    "weight {weight} out of range"
                );
            }
        }
    }

    /// Adjacent triangles overlap: that is what makes the filterbank continuous
    /// rather than a set of disjoint bandpasses with gaps between them.
    #[test]
    fn adjacent_triangles_overlap() {
        let config = FbankConfig::default();
        let filters = mel_filterbank(&config, config.fft_size());
        let mut overlaps = 0;
        for pair in filters.windows(2) {
            if pair[0]
                .iter()
                .zip(&pair[1])
                .any(|(a, b)| *a > 0.0 && *b > 0.0)
            {
                overlaps += 1;
            }
        }
        assert!(
            overlaps > 60,
            "only {overlaps} of 79 adjacent pairs overlap; the bank has gaps"
        );
    }

    /// Bins below `low_freq` must be excluded, or DC and rumble land in the first
    /// mel channel and dominate it.
    #[test]
    fn energy_below_the_low_cutoff_is_excluded() {
        let config = FbankConfig::default();
        let filters = mel_filterbank(&config, config.fft_size());
        let hz_per_bin = config.sample_rate as f32 / config.fft_size() as f32;
        let dc_bin = 0;
        let low_bin = (config.low_freq / hz_per_bin) as usize;
        for filter in &filters {
            assert_eq!(filter[dc_bin], 0.0, "DC must not reach any mel bin");
        }
        assert!(
            filters[0][..=low_bin].iter().all(|weight| *weight == 0.0),
            "the first triangle must start above the low cutoff"
        );
    }

    /* ───────────────────────────── the window ───────────────────────────── */

    /// Hamming, not Hann and not Povey. It ends at 0.08 rather than 0.0, and
    /// getting this wrong changes every feature without producing any error.
    #[test]
    fn the_window_is_hamming() {
        let window = hamming(400);
        assert_eq!(window.len(), 400);
        assert!(
            (window[0] - 0.08).abs() < 1e-4,
            "Hamming starts at 0.08, got {}",
            window[0]
        );
        assert!(
            (window[399] - 0.08).abs() < 1e-4,
            "Hamming ends at 0.08, got {}",
            window[399]
        );
        // Symmetric, and peaking at 1.0 in the middle.
        assert!((window[200] - 1.0).abs() < 0.01);
        for i in 0..200 {
            assert!((window[i] - window[399 - i]).abs() < 1e-5);
        }
    }

    #[test]
    fn a_degenerate_window_length_does_not_panic() {
        assert_eq!(hamming(0).len(), 0);
        assert_eq!(hamming(1), vec![1.0]);
    }

    /* ─────────────────────────── the mel warp ─────────────────────────── */

    #[test]
    fn the_mel_warp_is_kaldis() {
        assert_eq!(hz_to_mel(0.0), 0.0);
        // 1127 * ln(1 + 1000/700) = 1000.0 to within rounding.
        assert!((hz_to_mel(1_000.0) - 999.99).abs() < 1.0);
        // Monotonic, and compressive at the top.
        assert!(hz_to_mel(8_000.0) > hz_to_mel(4_000.0));
        assert!(hz_to_mel(8_000.0) - hz_to_mel(4_000.0) < hz_to_mel(4_000.0) - hz_to_mel(0.0));
    }

    /* ─────────────────────── discrimination sanity ─────────────────────── */

    /// The property the whole pass depends on: two different sounds must produce
    /// different features. A bug that collapses everything to a constant would
    /// still cluster — into one speaker, always — so this is the cheapest guard
    /// against the module silently doing nothing.
    #[test]
    fn different_audio_produces_different_features() {
        let computer = computer();
        let low = computer.compute(&tone(8_000, 200.0));
        let high = computer.compute(&tone(8_000, 3_000.0));
        assert_eq!(low.len(), high.len());

        let difference: f32 = low
            .iter()
            .zip(&high)
            .map(|(a, b)| a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>())
            .sum();
        assert!(
            difference > 1.0,
            "a 200 Hz and a 3 kHz tone produced near-identical features ({difference})"
        );
    }

    /// The scaling is required by the model, so it must actually be applied — and
    /// because CMN removes any constant offset, the only way to observe it is that
    /// the *unscaled* path clips into the epsilon floor where the scaled one does
    /// not.
    #[test]
    fn the_int16_scaling_is_applied() {
        let quiet = tone(8_000, 440.0)
            .iter()
            .map(|sample| sample * 1e-4)
            .collect::<Vec<_>>();

        let scaled = FbankComputer::new(FbankConfig::default()).compute(&quiet);
        let unscaled = FbankComputer::new(FbankConfig {
            scale_to_int16: false,
            ..Default::default()
        })
        .compute(&quiet);

        let spread = |features: &[Vec<f32>]| -> f32 {
            let flat: Vec<f32> = features.iter().flatten().copied().collect();
            let max = flat.iter().cloned().fold(f32::MIN, f32::max);
            let min = flat.iter().cloned().fold(f32::MAX, f32::min);
            max - min
        };
        assert!(
            spread(&scaled) > spread(&unscaled),
            "scaling should lift quiet audio off the epsilon floor: {} vs {}",
            spread(&scaled),
            spread(&unscaled)
        );
    }
}
