use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::sync::Arc;

// The bars track how far the signal rises *above the room*, not its absolute
// level — that is what lets quiet speech move them on a low-gain mic while
// steady ambient hiss stays at rest. Each bucket maps the band level from its
// adaptive noise floor (plus a small margin) across DYN_RANGE dB onto 0..1.
const DYN_RANGE: f32 = 36.0; // dB above the floor that spans the full bar height
const FLOOR_MARGIN: f32 = 5.0; // dB above the floor before a bar starts to lift
const FLOOR_TRACK_BAND: f32 = 10.0; // creep the floor up to ambient within this band; above it = speech
const FLOOR_MIN: f32 = -70.0; // hard limit so the floor never chases digital silence
const FLOOR_FALL: f32 = 0.15; // fast downward tracking toward quieter ambient (<1s)
const FLOOR_RISE: f32 = 0.02; // slow upward creep while only room tone is present
const GAIN: f32 = 1.2; // gentle lift of the mid-range
const CURVE_POWER: f32 = 0.6; // sub-1 curve raises soft speech without clipping loud

pub struct AudioVisualiser {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    bucket_ranges: Vec<(usize, usize)>,
    fft_input: Vec<Complex32>,
    noise_floor: Vec<f32>,
    buffer: Vec<f32>,
    window_size: usize,
    buckets: usize,
}

impl AudioVisualiser {
    pub fn new(
        sample_rate: u32,
        window_size: usize,
        buckets: usize,
        freq_min: f32,
        freq_max: f32,
    ) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(window_size);

        // Pre-compute Hann window
        let window: Vec<f32> = (0..window_size)
            .map(|i| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / window_size as f32).cos())
            })
            .collect();

        // Pre-compute bucket frequency ranges
        let nyquist = sample_rate as f32 / 2.0;
        let freq_min = freq_min.min(nyquist);
        let freq_max = freq_max.min(nyquist);

        let mut bucket_ranges = Vec::with_capacity(buckets);

        for b in 0..buckets {
            // Use logarithmic spacing for better perceptual representation
            let log_start = (b as f32 / buckets as f32).powi(2);
            let log_end = ((b + 1) as f32 / buckets as f32).powi(2);

            let start_hz = freq_min + (freq_max - freq_min) * log_start;
            let end_hz = freq_min + (freq_max - freq_min) * log_end;

            let start_bin = ((start_hz * window_size as f32) / sample_rate as f32) as usize;
            let mut end_bin = ((end_hz * window_size as f32) / sample_rate as f32) as usize;

            // Ensure each bucket has at least one bin
            if end_bin <= start_bin {
                end_bin = start_bin + 1;
            }

            // Clamp to valid range
            let start_bin = start_bin.min(window_size / 2);
            let end_bin = end_bin.min(window_size / 2);

            bucket_ranges.push((start_bin, end_bin));
        }

        Self {
            fft,
            window,
            bucket_ranges,
            fft_input: vec![Complex32::new(0.0, 0.0); window_size],
            noise_floor: vec![-45.0; buckets], // start just above typical ambient; fast-fall converges in <1s
            buffer: Vec::with_capacity(window_size * 2),
            window_size,
            buckets,
        }
    }

    pub fn feed(&mut self, samples: &[f32]) -> Option<Vec<f32>> {
        // Add new samples to buffer
        self.buffer.extend_from_slice(samples);

        // Only process if we have enough samples
        if self.buffer.len() < self.window_size {
            return None;
        }

        // Take the required window of samples
        let window_samples = &self.buffer[..self.window_size];

        // Remove DC component
        let mean = window_samples.iter().sum::<f32>() / self.window_size as f32;

        // Apply window function and prepare FFT input
        for (i, &sample) in window_samples.iter().enumerate() {
            let windowed_sample = (sample - mean) * self.window[i];
            self.fft_input[i] = Complex32::new(windowed_sample, 0.0);
        }

        // Perform FFT
        self.fft.process(&mut self.fft_input);

        // Compute power spectrum and bucket levels
        let mut buckets = vec![0.0; self.buckets];

        for (bucket_idx, &(start_bin, end_bin)) in self.bucket_ranges.iter().enumerate() {
            if start_bin >= end_bin || end_bin > self.fft_input.len() / 2 {
                continue;
            }

            // Calculate average power in this frequency range
            let mut power_sum = 0.0;
            for bin_idx in start_bin..end_bin {
                let magnitude = self.fft_input[bin_idx].norm();
                power_sum += magnitude * magnitude;
            }

            let avg_power = power_sum / (end_bin - start_bin) as f32;

            // Convert to dB with proper scaling
            let db = if avg_power > 1e-12 {
                20.0 * (avg_power.sqrt() / self.window_size as f32).log10()
            } else {
                -90.0 // effectively silent
            };

            // Adaptive per-bucket noise floor (a minimum-follower): fall quickly
            // toward quieter ambient, creep up only slowly while no speech is
            // present, and never descend past FLOOR_MIN. Anchoring to this floor
            // — instead of a fixed DB_MIN — is what makes quiet speech register:
            // we react to how far the band rises above the room, so a soft voice
            // on a low-gain mic still lifts the bars while steady hiss does not.
            let nf = self.noise_floor[bucket_idx];
            self.noise_floor[bucket_idx] = if db < nf {
                (FLOOR_FALL * db + (1.0 - FLOOR_FALL) * nf).max(FLOOR_MIN)
            } else if db < nf + FLOOR_TRACK_BAND {
                FLOOR_RISE * db + (1.0 - FLOOR_RISE) * nf
            } else {
                nf // clearly speech/transient — hold the floor steady
            };

            // Map [floor+margin .. floor+margin+DYN_RANGE] dB onto 0..1 so the
            // gesture reads consistently regardless of mic gain, then lift the
            // low end with gain + a sub-1 curve so soft speech stays visible.
            let floor = self.noise_floor[bucket_idx] + FLOOR_MARGIN;
            let normalized = ((db - floor) / DYN_RANGE).clamp(0.0, 1.0);
            buckets[bucket_idx] = (normalized * GAIN).powf(CURVE_POWER).clamp(0.0, 1.0);
        }

        // Apply light smoothing to reduce jitter
        for i in 1..buckets.len() - 1 {
            buckets[i] = buckets[i] * 0.7 + buckets[i - 1] * 0.15 + buckets[i + 1] * 0.15;
        }

        // Clear processed samples from buffer
        self.buffer.clear();

        Some(buckets)
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        // Reset noise floor to initial values
        self.noise_floor.fill(-45.0);
    }
}
