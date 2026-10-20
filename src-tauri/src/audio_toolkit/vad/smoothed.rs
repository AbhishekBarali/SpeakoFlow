use super::{VadFrame, VoiceActivityDetector};
use anyhow::Result;
use std::collections::VecDeque;

/// One pre-roll buffer slot.
///
/// `emitted` is what keeps a frame from reaching the output twice. Hangover
/// emits the silent frames after speech one at a time, and those frames stay
/// in the pre-roll buffer; when speech resumes within `prefill_frames` of
/// them, the onset used to replay the whole buffer as prefill and repeat up
/// to ~0.45 s of audio the recording already had, which the model then
/// transcribed as repeated words.
struct BufferedFrame {
    samples: Vec<f32>,
    emitted: bool,
}

pub struct SmoothedVad {
    inner_vad: Box<dyn VoiceActivityDetector>,
    prefill_frames: usize,
    hangover_frames: usize,
    onset_frames: usize,

    frame_buffer: VecDeque<BufferedFrame>,
    hangover_counter: usize,
    onset_counter: usize,
    in_speech: bool,
    /// Hangover frames emitted since voice was last heard (see
    /// [`VoiceActivityDetector::tail_frames`]).
    tail: usize,

    temp_out: Vec<f32>,
}

impl SmoothedVad {
    pub fn new(
        inner_vad: Box<dyn VoiceActivityDetector>,
        prefill_frames: usize,
        hangover_frames: usize,
        onset_frames: usize,
    ) -> Self {
        Self {
            inner_vad,
            prefill_frames,
            hangover_frames,
            onset_frames,
            frame_buffer: VecDeque::new(),
            hangover_counter: 0,
            onset_counter: 0,
            in_speech: false,
            tail: 0,
            temp_out: Vec::new(),
        }
    }

    /// Records that the newest buffered frame (the one just pushed) went out
    /// on its own, so a later onset does not replay it as prefill.
    fn mark_last_emitted(&mut self) {
        if let Some(frame) = self.frame_buffer.back_mut() {
            frame.emitted = true;
        }
    }
}

impl VoiceActivityDetector for SmoothedVad {
    fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> Result<VadFrame<'a>> {
        // 1. Buffer every incoming frame for possible pre-roll
        self.frame_buffer.push_back(BufferedFrame {
            samples: frame.to_vec(),
            emitted: false,
        });
        while self.frame_buffer.len() > self.prefill_frames + 1 {
            self.frame_buffer.pop_front();
        }

        // 2. Delegate to the wrapped boolean VAD
        let is_voice = self.inner_vad.is_voice(frame)?;
        // Only a hangover frame (below) extends the tail.
        let tail = std::mem::take(&mut self.tail);

        match (self.in_speech, is_voice) {
            // Potential start of speech - need to accumulate onset frames
            (false, true) => {
                self.onset_counter += 1;
                if self.onset_counter >= self.onset_frames {
                    // We have enough consecutive voice frames to trigger speech
                    self.in_speech = true;
                    self.hangover_counter = self.hangover_frames;
                    self.onset_counter = 0; // Reset for next time

                    // Collect the prefill that has not gone out yet, plus the
                    // current frame. Frames emitted during an earlier hangover
                    // are skipped: they are already in the recording.
                    self.temp_out.clear();
                    for buffered in self.frame_buffer.iter_mut().filter(|f| !f.emitted) {
                        self.temp_out.extend(buffered.samples.iter());
                        buffered.emitted = true;
                    }
                    Ok(VadFrame::Speech(&self.temp_out))
                } else {
                    // Not enough frames yet, still silence
                    Ok(VadFrame::Noise)
                }
            }

            // Ongoing Speech
            (true, true) => {
                self.hangover_counter = self.hangover_frames;
                self.mark_last_emitted();
                Ok(VadFrame::Speech(frame))
            }

            // End of Speech or interruption during onset phase
            (true, false) => {
                if self.hangover_counter > 0 {
                    self.hangover_counter -= 1;
                    self.tail = tail + 1;
                    self.mark_last_emitted();
                    Ok(VadFrame::Speech(frame))
                } else {
                    self.in_speech = false;
                    Ok(VadFrame::Noise)
                }
            }

            // Silence or broken onset sequence
            (false, false) => {
                self.onset_counter = 0; // Reset onset counter on silence
                Ok(VadFrame::Noise)
            }
        }
    }

    fn set_hangover_frames(&mut self, frames: usize) {
        self.hangover_frames = frames;
        // A tail already running never outlasts the new length.
        self.hangover_counter = self.hangover_counter.min(frames);
    }

    fn tail_frames(&self) -> usize {
        self.tail
    }

    fn reset(&mut self) {
        self.frame_buffer.clear();
        self.inner_vad.reset();
        self.hangover_counter = 0;
        self.onset_counter = 0;
        self.in_speech = false;
        self.tail = 0;
        self.temp_out.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inner VAD that replays a scripted voice/no-voice sequence.
    struct ScriptedVad {
        script: VecDeque<bool>,
    }

    impl VoiceActivityDetector for ScriptedVad {
        fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> Result<VadFrame<'a>> {
            if self.script.pop_front().unwrap_or(false) {
                Ok(VadFrame::Speech(frame))
            } else {
                Ok(VadFrame::Noise)
            }
        }
    }

    /// Runs `script` through a smoothed VAD shaped like the recorder's
    /// (`prefill`, `hangover`, `onset`), where frame `i` is filled with the
    /// value `i`, and returns the index of every frame that came out, in order.
    fn emitted_frame_indices(
        script: &[bool],
        prefill: usize,
        hangover: usize,
        onset: usize,
    ) -> Vec<usize> {
        let inner = ScriptedVad {
            script: script.iter().copied().collect(),
        };
        let mut vad = SmoothedVad::new(Box::new(inner), prefill, hangover, onset);
        let mut out = Vec::new();
        for i in 0..script.len() {
            let frame = vec![i as f32; 4];
            if let VadFrame::Speech(samples) = vad.push_frame(&frame).unwrap() {
                // Every frame is four copies of its own index.
                out.extend(samples.chunks(4).map(|chunk| chunk[0] as usize));
            }
        }
        out
    }

    fn assert_strictly_increasing(indices: &[usize]) {
        for pair in indices.windows(2) {
            assert!(
                pair[0] < pair[1],
                "frame {} emitted out of order or twice: {indices:?}",
                pair[1]
            );
        }
    }

    #[test]
    fn reset_reaches_the_detector() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        struct CountsResets(Arc<AtomicUsize>);
        impl VoiceActivityDetector for CountsResets {
            fn push_frame<'a>(&'a mut self, _frame: &'a [f32]) -> Result<VadFrame<'a>> {
                Ok(VadFrame::Noise)
            }
            fn reset(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        // The recorder resets the smoother at every start; Silero's state must
        // not carry over from the previous recording.
        let resets = Arc::new(AtomicUsize::new(0));
        let mut vad = SmoothedVad::new(Box::new(CountsResets(resets.clone())), 15, 15, 2);
        vad.reset();
        assert_eq!(resets.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn speech_resuming_within_prefill_does_not_replay_hangover_frames() {
        // Speech, a pause shorter than the hangover plus the prefill, then
        // speech again: the shape that used to repeat words.
        let mut script = vec![true; 5];
        script.extend([false; 4]);
        script.extend([true; 5]);
        let emitted = emitted_frame_indices(&script, 15, 2, 2);
        assert_strictly_increasing(&emitted);
    }

    #[test]
    fn no_frame_is_emitted_twice_under_any_pause_length() {
        // The recorder's own shape (15 prefill, 15 hangover, 2 onset), with
        // every pause length from one frame to well past the pre-roll.
        for pause in 1..40 {
            let mut script = vec![true; 6];
            script.extend(vec![false; pause]);
            script.extend([true; 6]);
            script.extend(vec![false; pause]);
            script.extend([true, false, true, true]);
            let emitted = emitted_frame_indices(&script, 15, 15, 2);
            assert_strictly_increasing(&emitted);
        }
    }

    #[test]
    fn onset_still_carries_unemitted_prefill() {
        // Silence, then speech: the pre-roll before the onset is still
        // emitted, so the first syllable is not clipped.
        let script = [false, false, false, true, true, true];
        let emitted = emitted_frame_indices(&script, 3, 2, 2);
        assert_eq!(emitted, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn resumed_speech_adds_only_the_frames_hangover_left_out() {
        // Onset at frame 1 emits 0 and 1; hangover emits 2 and 3; 4 and 5 are
        // withheld. The onset at 7 has 2..=7 in its pre-roll, and must add
        // only 4..=7, not 2 and 3 a second time.
        let script = [true, true, false, false, false, false, true, true];
        let emitted = emitted_frame_indices(&script, 5, 2, 2);
        assert_eq!(emitted, (0..8).collect::<Vec<_>>());
    }

    /// The frames a recorder-shaped VAD keeps after five frames of speech and
    /// then `silence` frames of silence, with the tail set to `hangover`.
    fn kept_after_speech(hangover: usize, silence: usize) -> (usize, Vec<usize>) {
        let mut script = vec![true; 5];
        script.extend(vec![false; silence]);
        let inner = ScriptedVad {
            script: script.iter().copied().collect(),
        };
        let mut vad = SmoothedVad::new(Box::new(inner), 15, 15, 2);
        vad.set_hangover_frames(hangover);
        let mut kept = 0;
        let mut tails = Vec::new();
        for _ in 0..script.len() {
            if vad.push_frame(&[0.0; 4]).unwrap().is_speech() {
                kept += 1;
                tails.push(vad.tail_frames());
            }
        }
        (kept, tails)
    }

    #[test]
    fn the_tail_length_follows_the_setting() {
        // Five speech frames (the onset releases the first two together),
        // then the tail, then nothing.
        let (offline, _) = kept_after_speech(15, 80);
        let (streaming, _) = kept_after_speech(55, 80);
        assert_eq!(offline, 4 + 15);
        assert_eq!(streaming, 4 + 55);
    }

    #[test]
    fn the_tail_counts_frames_since_voice() {
        let (_, tails) = kept_after_speech(3, 10);
        // Speech frames read 0; each tail frame its place in the tail.
        assert_eq!(tails, vec![0, 0, 0, 0, 1, 2, 3]);
    }

    #[test]
    fn shortening_the_tail_mid_way_ends_it_sooner() {
        let inner = ScriptedVad {
            script: [true, true, false, false, false, false].into(),
        };
        let mut vad = SmoothedVad::new(Box::new(inner), 4, 10, 2);
        let mut kept = 0;
        for i in 0..6 {
            if i == 3 {
                vad.set_hangover_frames(1);
            }
            if vad.push_frame(&[0.0; 4]).unwrap().is_speech() {
                kept += 1;
            }
        }
        // Onset, one tail frame at the old length, one at the new, then off.
        assert_eq!(kept, 3);
    }
}
