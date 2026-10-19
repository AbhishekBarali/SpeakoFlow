//! Long-form chunked decoding, for a whole clip and for a recording still in
//! progress. Both cut the same audio at the same pauses
//! ([`crate::audio_toolkit::chunking`]) and decode each chunk with the same
//! run, so a live result and a batch run over the recording agree.

use std::time::Instant;

use log::{debug, info, warn};

use super::chinese_script::{self, LiveScript};
use super::policy::{live_decode, LiveDecode, ModelTraits};
use super::{
    correct_live_display, recognition_words, sanitize_pcm, silero_scorer_factory,
    transcribe_cpp_run_plan, LiveResult, OpenedStream, RunFacts, RunPlan, StreamCmd,
    StreamTextPayload, TranscriptionManager,
};
use crate::audio_toolkit::chunking::{join_transcripts, Chunker, MakeScorer};
use crate::audio_toolkit::normalize_peak;
use crate::settings::get_settings;
use tauri::Emitter;

const RATE: f64 = crate::audio_toolkit::constants::WHISPER_SAMPLE_RATE as f64;

/// A clip decoded in chunks.
pub(super) struct ChunkedText {
    pub text: String,
    /// How many chunks it was cut into; the live tests report it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub chunks: usize,
}

/// Decode a whole clip in chunks cut at pauses, each through `decode` (a run
/// in the transcription worker, which queues behind any other decode). Every
/// chunk must decode: a failure fails the clip, as a single run failing would.
/// Cutting runs here, in the app: the VAD that scores the pauses is ONNX, not
/// transcribe.cpp.
pub(super) fn decode_clip(
    decode: &mut dyn FnMut(Vec<f32>) -> anyhow::Result<String>,
    audio: Vec<f32>,
    make_scorer: MakeScorer,
) -> anyhow::Result<ChunkedText> {
    let started = Instant::now();
    let total_secs = audio.len() as f64 / RATE;
    let mut chunker = Chunker::new(make_scorer);
    let mut chunks = chunker.push(audio);
    chunks.extend(chunker.finish());
    let cut_in = started.elapsed();

    let count = chunks.len();
    let mut texts = Vec::with_capacity(count);
    for (index, chunk) in chunks.into_iter().enumerate() {
        let chunk_started = Instant::now();
        let chunk_secs = chunk.len() as f64 / RATE;
        texts.push(decode(chunk)?);
        debug!(
            "Decoded long-form chunk {}/{} ({:.1}s of audio) in {:.2}s",
            index + 1,
            count,
            chunk_secs,
            chunk_started.elapsed().as_secs_f64()
        );
    }
    info!(
        "Decoded {:.1}s of audio in {} chunks in {:.2}s (cut in {:.2}s)",
        total_secs,
        count,
        started.elapsed().as_secs_f64(),
        cut_in.as_secs_f64()
    );
    Ok(ChunkedText {
        text: join_transcripts(&texts),
        chunks: count,
    })
}

/// A recording's long-form chunks as the recording goes: the audio is cut as
/// it arrives and each chunk handed to `decode` once its cut is certain.
///
/// The decision of whether the result can stand in for a batch run over the
/// whole recording lives here, apart from the engine, so it can be tested
/// with a decoder that is not a model.
pub(super) struct ChunkSession {
    chunker: Chunker,
    texts: Vec<String>,
    /// A chunk failed or was cancelled; nothing more is decoded and the
    /// recording goes to the batch path.
    failed: bool,
}

impl ChunkSession {
    pub(super) fn new(make_scorer: MakeScorer) -> Self {
        Self {
            chunker: Chunker::new(make_scorer),
            texts: Vec::new(),
            failed: false,
        }
    }

    /// Chunks decoded so far.
    #[cfg(test)]
    pub(super) fn decoded(&self) -> usize {
        self.texts.len()
    }

    /// The text decoded so far, joined.
    pub(super) fn text(&self) -> String {
        join_transcripts(&self.texts)
    }

    /// Add recorded audio, decoding every chunk it makes certain. Returns
    /// whether new text arrived.
    pub(super) fn push(
        &mut self,
        pcm: Vec<f32>,
        decode: &mut dyn FnMut(Vec<f32>) -> anyhow::Result<String>,
    ) -> bool {
        if self.failed {
            return false;
        }
        let before = self.texts.len();
        for chunk in self.chunker.push(pcm) {
            if !self.decode(chunk, decode) {
                break;
            }
        }
        self.texts.len() > before
    }

    /// The whole recording's text, once it has stopped: the rest decoded and
    /// every chunk joined. `None` sends the recording to the batch path,
    /// which has the complete audio: when nothing was decoded while
    /// recording (a short dictation, which then gets exactly the batch run it
    /// always had), when a chunk failed, or when the audio here does not
    /// match the `recorded_samples` of the recording.
    pub(super) fn finish(
        mut self,
        recorded_samples: usize,
        decode: &mut dyn FnMut(Vec<f32>) -> anyhow::Result<String>,
    ) -> Option<String> {
        if self.failed || self.texts.is_empty() {
            return None;
        }
        if self.chunker.samples() != recorded_samples {
            info!(
                "Live long-form chunks saw {} of the recording's {} samples; \
                 transcribing in batch",
                self.chunker.samples(),
                recorded_samples
            );
            return None;
        }
        let while_recording = self.texts.len();
        for chunk in self.chunker.finish() {
            if !self.decode(chunk, decode) {
                return None;
            }
        }
        info!(
            "{} of {} long-form chunks were decoded while recording",
            while_recording,
            self.texts.len()
        );
        Some(self.text())
    }

    fn decode(
        &mut self,
        chunk: Vec<f32>,
        decode: &mut dyn FnMut(Vec<f32>) -> anyhow::Result<String>,
    ) -> bool {
        match decode(chunk) {
            Ok(text) => {
                self.texts.push(text);
                true
            }
            Err(e) => {
                info!(
                    "Live long-form chunking stopped ({}); the recording will be \
                     transcribed in batch",
                    e
                );
                self.failed = true;
                false
            }
        }
    }
}

/// Everything a recording's chunk worker keeps besides the chunks.
struct LiveChunks {
    session: ChunkSession,
    plan: RunPlan,
    model_id: String,
    live_script: LiveScript,
    live_hints: Vec<String>,
    live_threshold: f64,
    live_phrase: Option<String>,
}

impl TranscriptionManager {
    /// Worker body for [`LiveDecode::Chunked`]: the recorder's kept speech is
    /// cut at pauses as it arrives, and each chunk is decoded once its cut is
    /// certain, so a long dictation has only its last chunk left when it
    /// stops, and the overlay's live card shows the text so far.
    ///
    /// Unlike a stream, this never holds the engine between chunks: each one is
    /// its own run in the transcription worker, queued like any other, so a
    /// meeting chunk or a History retry can run in between, and two decodes
    /// never run at once. Cancel stops the decode under way and nothing more
    /// is decoded.
    ///
    /// Any doubt goes to the batch path (see [`ChunkSession::finish`]), and so
    /// does a model switched mid-way.
    pub(super) fn run_chunked_worker(&self, opened: OpenedStream) {
        let OpenedStream {
            rx,
            generation,
            cancel,
            ..
        } = opened;

        let Some(mut live) = self.begin_live_chunks() else {
            self.drain_stream_no_model(rx);
            self.stream_router.clear(generation);
            return;
        };
        info!(
            "Live long-form chunking started (model '{}')",
            live.model_id
        );

        let mut finalize = None;
        while let Ok(cmd) = rx.recv() {
            match cmd {
                StreamCmd::Feed(pcm) => {
                    self.stream_router.note_feed_consumed();
                    let LiveChunks {
                        session,
                        plan,
                        model_id,
                        ..
                    } = &mut live;
                    let mut decode = |chunk| self.decode_live_chunk(chunk, plan, model_id, &cancel);
                    if session.push(pcm, &mut decode) {
                        self.emit_live_chunks(&mut live, generation);
                    }
                }
                StreamCmd::Finalize {
                    recorded_samples,
                    reply,
                } => {
                    finalize = Some((recorded_samples, reply));
                    break;
                }
                StreamCmd::Cancel => break,
            }
        }

        let Some((recorded_samples, reply)) = finalize else {
            // Cancelled: no result, and nothing more is decoded.
            self.stream_router.clear(generation);
            return;
        };
        let LiveChunks {
            session,
            plan,
            model_id,
            ..
        } = live;
        let mut decode = |chunk| self.decode_live_chunk(chunk, &plan, &model_id, &cancel);
        let result = session
            .finish(recorded_samples, &mut decode)
            .map(|text| LiveResult {
                text,
                facts: RunFacts::local(&plan),
            });
        self.stream_router.clear(generation);
        let _ = reply.send(result);
    }

    /// The run setup for a recording's chunks, from the loaded model. `None`
    /// when there is no engine to look at, or the loaded model is not one to
    /// chunk (a streaming model, Whisper, or chunking turned off).
    fn begin_live_chunks(&self) -> Option<LiveChunks> {
        {
            let mut is_loading = self.is_loading.lock().unwrap_or_else(|e| e.into_inner());
            while *is_loading {
                is_loading = self
                    .loading_condvar
                    .wait(is_loading)
                    .unwrap_or_else(|e| e.into_inner());
            }
        }
        let settings = get_settings(&self.app_handle);
        let plan = {
            let Some(info) = self.engine.loaded() else {
                debug!("Live long-form chunking: no model loaded; transcribing in batch");
                return None;
            };
            let traits = ModelTraits::of_loaded(&info);
            if live_decode(traits, settings.long_form_chunking) != LiveDecode::Chunked {
                debug!(
                    "Live long-form chunking: the loaded model is not one to chunk ({:?})",
                    traits
                );
                return None;
            }
            transcribe_cpp_run_plan(
                &settings,
                &info.capabilities,
                &info.arch,
                info.supports_vocabulary,
            )
        };
        let model_id = self.get_current_model()?;
        let vad_path = crate::managers::audio::silero_vad_path(&self.app_handle).ok();
        let live_script = LiveScript::new(chinese_script::script_rule(
            &settings.selected_language,
            &plan.effective_language,
            settings.chinese_script,
            &plan.languages,
        ));
        let live_hints = if plan.words_biased {
            Vec::new()
        } else {
            recognition_words(&settings)
        };
        Some(LiveChunks {
            session: ChunkSession::new(silero_scorer_factory(vad_path)),
            plan,
            model_id,
            live_script,
            live_hints,
            live_threshold: settings.word_correction_threshold,
            live_phrase: settings
                .flow_enabled
                .then(|| settings.flow_phrase.trim().to_string())
                .filter(|phrase| !phrase.is_empty()),
        })
    }

    /// Decode one chunk on the engine.
    fn decode_live_chunk(
        &self,
        mut chunk: Vec<f32>,
        plan: &RunPlan,
        model_id: &str,
        cancel: &transcribe_cpp::CancelToken,
    ) -> anyhow::Result<String> {
        if cancel.is_cancelled() {
            anyhow::bail!("the recording was cancelled");
        }
        if self.get_current_model().as_deref() != Some(model_id) {
            anyhow::bail!("the model changed");
        }
        // The batch path gives the whole recording these two passes; a chunk
        // gets them on its own.
        sanitize_pcm(&mut chunk);
        normalize_peak(&mut chunk, 0.95, 8.0);

        let started = Instant::now();
        let audio_secs = chunk.len() as f64 / RATE;
        let decoded = self.run_in_worker(chunk, &plan.options, Some(cancel));
        match &decoded {
            Ok(_) => info!(
                "Decoded a long-form chunk ({:.1}s of audio) in {:.2}s while recording",
                audio_secs,
                started.elapsed().as_secs_f64()
            ),
            Err(_) if cancel.is_cancelled() => debug!("Live long-form chunk cancelled"),
            Err(e) => warn!("Live long-form chunk failed: {}", e),
        }
        decoded
    }

    /// Show the text decoded so far on the overlay's live card.
    fn emit_live_chunks(&self, live: &mut LiveChunks, generation: u64) {
        if !self.stream_router.is_generation_current(generation) {
            return;
        }
        let (committed, _) = live.live_script.apply(&live.session.text(), "");
        let display = correct_live_display(
            &committed,
            &live.live_hints,
            live.live_threshold,
            live.live_phrase.as_deref(),
        );
        crate::flow::note_live_transcript(&self.app_handle, &display);
        let _ = self.app_handle.emit(
            "stream-text",
            StreamTextPayload {
                committed: display,
                tentative: String::new(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_toolkit::vad::SpeechScorer;

    const FRAME: usize = 480;
    const RATE: usize = 16_000;

    /// Scores each frame with its first sample, as the chunker's own tests do.
    struct FirstSample;
    impl SpeechScorer for FirstSample {
        fn frame_samples(&self) -> usize {
            FRAME
        }
        fn score(&mut self, frames: &[f32]) -> anyhow::Result<Vec<f32>> {
            Ok(frames.chunks(FRAME).map(|frame| frame[0]).collect())
        }
    }

    fn session() -> ChunkSession {
        ChunkSession::new(Box::new(|| {
            Ok(Box::new(FirstSample) as Box<dyn SpeechScorer>)
        }))
    }

    /// `seconds` of speech with a one-second pause every 25 s.
    fn speech(seconds: usize) -> Vec<f32> {
        (0..seconds * RATE)
            .map(|i| if (i / RATE) % 25 == 24 { 0.0 } else { 1.0 })
            .collect()
    }

    /// A decoder that names each chunk by its length in seconds.
    fn lengths(calls: &mut usize) -> impl FnMut(Vec<f32>) -> anyhow::Result<String> + '_ {
        move |chunk| {
            *calls += 1;
            Ok(format!("{}s", chunk.len() / RATE))
        }
    }

    #[test]
    fn chunks_are_decoded_while_recording_and_the_tail_at_the_end() {
        let pcm = speech(130);
        let mut calls = 0;
        let mut decode = lengths(&mut calls);
        let mut live = session();
        let mut arrived = 0;
        for piece in pcm.chunks(FRAME * 10) {
            if live.push(piece.to_vec(), &mut decode) {
                arrived += 1;
            }
        }
        assert!(
            live.decoded() >= 2,
            "{} decoded while recording",
            live.decoded()
        );
        assert_eq!(arrived, live.decoded());
        let text = live.finish(pcm.len(), &mut decode).expect("a live result");
        let decoded: usize = text
            .split(' ')
            .map(|piece| piece.trim_end_matches('s').parse::<usize>().unwrap())
            .sum();
        // Every second of the recording, in some chunk, rounded down per chunk.
        assert!((126..=130).contains(&decoded), "{text}");
    }

    #[test]
    fn a_short_recording_is_left_to_the_batch_path() {
        let pcm = speech(20);
        let mut calls = 0;
        let mut decode = lengths(&mut calls);
        let mut live = session();
        live.push(pcm.clone(), &mut decode);
        assert_eq!(live.finish(pcm.len(), &mut decode), None);
        drop(decode);
        assert_eq!(calls, 0, "nothing decoded twice");
    }

    #[test]
    fn audio_that_does_not_match_the_recording_is_left_to_the_batch_path() {
        let pcm = speech(110);
        let mut calls = 0;
        let mut decode = lengths(&mut calls);
        let mut live = session();
        live.push(pcm.clone(), &mut decode);
        assert!(live.decoded() > 0);
        assert_eq!(live.finish(pcm.len() + 1, &mut decode), None);
    }

    #[test]
    fn a_failed_chunk_stops_decoding_and_leaves_it_to_the_batch_path() {
        let pcm = speech(130);
        let calls = std::cell::Cell::new(0);
        let mut decode = |_chunk: Vec<f32>| -> anyhow::Result<String> {
            calls.set(calls.get() + 1);
            anyhow::bail!("cancelled")
        };
        let mut live = session();
        for piece in pcm.chunks(FRAME * 10) {
            live.push(piece.to_vec(), &mut decode);
        }
        assert_eq!(calls.get(), 1, "nothing is decoded after a failure");
        assert_eq!(live.finish(pcm.len(), &mut decode), None);
        assert_eq!(calls.get(), 1);
    }
}
