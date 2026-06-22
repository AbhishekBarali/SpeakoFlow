//! Lightweight audio DSP helpers applied to a captured utterance just before
//! transcription. These run once per recording, off the hot capture path, and
//! are O(n) over the buffer — a couple of linear passes over a few hundred
//! thousand `f32` samples, i.e. well under a millisecond. They are negligible
//! next to model inference and never touch live capture latency.

/// Peak-normalise quiet speech up toward a target level so the transcription
/// engine sees input closer to what it was trained on. This addresses
/// low-volume captures (e.g. speaking away from the mic, or outdoors) where
/// Whisper-style models degrade or hallucinate on faint signals.
///
/// Design choices that keep this safe:
/// - **Boost only.** If the audio is already at or above `target_peak`, it is
///   left untouched, so close-mic / already-hot recordings are unchanged.
/// - **Gain is capped** at `max_gain` so a near-silent buffer (pure background
///   noise that slipped past the VAD) isn't amplified to full scale.
/// - Output is clamped to `[-1.0, 1.0]` to avoid any chance of clipping.
///
/// `target_peak` and `max_gain` are validated/clamped to sane ranges. Returns
/// the gain that was actually applied (`1.0` means "left unchanged").
pub fn normalize_peak(samples: &mut [f32], target_peak: f32, max_gain: f32) -> f32 {
    if samples.is_empty() {
        return 1.0;
    }

    let target_peak = target_peak.clamp(0.1, 1.0);
    let max_gain = max_gain.max(1.0);

    // Find the current peak amplitude in a single pass.
    let peak = samples.iter().fold(0.0f32, |acc, &s| acc.max(s.abs()));

    // Treat effectively-silent buffers as nothing to do — boosting would only
    // amplify the noise floor.
    if peak <= 1e-4 {
        return 1.0;
    }

    let gain = target_peak / peak;

    // Only ever boost quiet audio; never attenuate already-loud audio.
    if gain <= 1.0 {
        return 1.0;
    }

    let gain = gain.min(max_gain);

    for s in samples.iter_mut() {
        *s = (*s * gain).clamp(-1.0, 1.0);
    }

    gain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_noop() {
        let mut samples: Vec<f32> = Vec::new();
        assert_eq!(normalize_peak(&mut samples, 0.9, 8.0), 1.0);
    }

    #[test]
    fn boosts_quiet_audio_toward_target() {
        // Peak 0.1 -> with target 0.9 the ideal gain is 9x (under the cap).
        let mut samples = vec![0.1, -0.05, 0.08, -0.1];
        let gain = normalize_peak(&mut samples, 0.9, 16.0);
        assert!((gain - 9.0).abs() < 1e-3, "unexpected gain: {gain}");
        let new_peak = samples.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!((new_peak - 0.9).abs() < 1e-3, "unexpected peak: {new_peak}");
    }

    #[test]
    fn does_not_attenuate_loud_audio() {
        let mut samples = vec![0.95, -0.99, 0.8];
        let before = samples.clone();
        let gain = normalize_peak(&mut samples, 0.9, 8.0);
        assert_eq!(gain, 1.0);
        assert_eq!(samples, before);
    }

    #[test]
    fn caps_gain_for_near_silent_noise() {
        // Peak 0.01 would want 90x to reach 0.9; the cap must hold it to 8x.
        let mut samples = vec![0.01, -0.008, 0.005];
        let gain = normalize_peak(&mut samples, 0.9, 8.0);
        assert_eq!(gain, 8.0);
        let new_peak = samples.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(new_peak <= 0.9, "peak should stay below target: {new_peak}");
    }

    #[test]
    fn silence_is_noop() {
        let mut samples = vec![0.0, 0.0, 0.00001, -0.00002];
        let gain = normalize_peak(&mut samples, 0.9, 8.0);
        assert_eq!(gain, 1.0);
    }

    #[test]
    fn output_is_always_in_range() {
        let mut samples = vec![0.2, -0.3, 0.25, -0.15];
        normalize_peak(&mut samples, 1.0, 100.0);
        for &s in &samples {
            assert!((-1.0..=1.0).contains(&s), "sample out of range: {s}");
        }
    }
}
