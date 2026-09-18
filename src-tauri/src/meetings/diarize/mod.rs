//! Post-meeting speaker diarization for the **system** audio stream.
//!
//! Turns "Others" into "Speaker 1", "Speaker 2" — the thing a meeting transcript is
//! unreadable without once more than one person is on the call.
//!
//! # Three properties that shape everything here
//!
//! * **It is advisory, and it never touches the microphone side.**
//!   [`SpeakerSource`](super::SpeakerSource) is a fact about the hardware:
//!   microphone samples are the user, loopback samples are everyone else.
//!   Clustering is a statistical guess. So the mic stream is left exactly as it is,
//!   and any system segment this pass declines to label keeps `"them"`. Every
//!   failure below degrades to the transcript the user already has — never to a
//!   failed meeting.
//! * **It labels existing rows rather than re-segmenting.** `chunker.rs` already cut
//!   the stream at 1.5 s of silence, which in a conference call is a turn boundary,
//!   and [`store::apply_diarization`](super::store::MeetingStore::apply_diarization)
//!   already matches spans to rows by midpoint. Frame-level diarization would be a
//!   great deal of work to rediscover boundaries we are handed for free.
//! * **It never loads the whole recording.** Three hours of 16 kHz mono `f32` is
//!   ~691 MB. Every read seeks to one segment.
//!
//! # The pipeline
//!
//! ```text
//!   system .wav ──seek per segment──▶ Silero VAD ──▶ fixed 1.5s windows
//!                                                        │
//!                                          fbank ──▶ ONNX ──▶ embeddings
//!                                                        │
//!                            average-linkage clustering + four guards
//!                                                        │
//!                                    spans ──▶ one transaction ──▶ meetings.db
//! ```
//!
//! The clustering step and its guards live in [`cluster`], pure and unit-tested,
//! because that is where every interesting bug is. This module is the I/O around
//! them.

pub mod cluster;
pub mod embed;
pub mod fbank;
pub mod model;

use std::sync::Arc;

use anyhow::{anyhow, Result};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter};

use super::store::{DiarizedSpan, MeetingStore};
use super::MeetingSegment;
use crate::audio_toolkit::constants::WHISPER_SAMPLE_RATE;

/// Progress of a diarization pass, for the UI.
pub const PROGRESS_EVENT: &str = "meeting-diarization-progress";

/// Tunables. Extracted so tests can use small values instead of real durations,
/// exactly as `ChunkConfig` does.
#[derive(Clone, Debug)]
pub struct DiarizeConfig {
    /// Embedding window length, in ms. 1.5 s is the length these models were
    /// trained to score, and short enough that a segment usually yields several.
    pub window_ms: i64,
    /// Hop between windows, in ms.
    pub hop_ms: i64,
    /// Most windows taken from one segment.
    ///
    /// Caps how much one long turn can weigh in the linkage. Without it, a
    /// five-minute monologue contributes two hundred windows and drags every
    /// cluster boundary toward itself.
    pub max_windows_per_segment: usize,
    /// Below this much voiced audio a segment is embedded but held out of the
    /// linkage, then assigned by nearest centroid. A noisy embedding must not be
    /// able to invent a speaker.
    pub min_voiced_for_linkage_ms: i64,
    /// Below this, no span is emitted at all and the row keeps `"them"`. Better
    /// unlabelled than wrong.
    pub min_voiced_for_label_ms: i64,
    /// Clusters holding less speech than this are dissolved into their neighbour.
    pub min_cluster_speech_ms: i64,
    pub max_speakers: usize,
    /// Cap on segments entering the linkage.
    ///
    /// The condensed distance matrix is O(n²): 4,000 segments is ~32 MB, 20,000
    /// would be ~800 MB. Past the cap a uniform sample is clustered and the rest
    /// assigned by nearest centroid, which costs a little accuracy and bounds the
    /// memory.
    pub max_linkage_segments: usize,
}

impl Default for DiarizeConfig {
    fn default() -> Self {
        Self {
            window_ms: 1_500,
            hop_ms: 750,
            max_windows_per_segment: 3,
            min_voiced_for_linkage_ms: 800,
            min_voiced_for_label_ms: 400,
            min_cluster_speech_ms: cluster::MIN_CLUSTER_SPEECH_MS,
            max_speakers: cluster::MAX_SPEAKERS,
            max_linkage_segments: 4_000,
        }
    }
}

/// Outcome of a pass, so callers can tell the three "nothing was written" cases
/// apart — they need different follow-up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum DiarizationOutcome {
    /// Several voices were found and rows were relabelled.
    Labelled { speakers: usize, segments: usize },
    /// Exactly one voice on the system side.
    ///
    /// No span is written — renaming a lone remote voice to "Speaker 1" implies
    /// there are others — but the meeting **is** marked diarized, so this does not
    /// re-run on every open and spend a minute of CPU reaching the same answer.
    OneSpeaker,
    /// Nothing was attempted. Carries the reason, so the UI can offer the right
    /// next step rather than a generic retry.
    Skipped { reason: SkipReason },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The 27 MB model is not on disk. The one reason worth an affordance.
    ModelNotInstalled,
    NoSystemAudio,
    AudioUnreadable,
    /// One segment cannot contain two speakers.
    TooFewSegments,
    AlreadyDiarized,
    /// The model is installed but would not load.
    ModelUnusable,
}

/// Live progress, so a long pass is visible rather than looking hung.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct DiarizationProgress {
    pub meeting_id: i64,
    /// Segments embedded so far.
    pub done: u32,
    pub total: u32,
}

/// One system segment plus what the VAD found inside it.
#[derive(Clone, Debug)]
struct SegmentAudio {
    row_id: i64,
    start_ms: i64,
    end_ms: i64,
    voiced_ms: i64,
    /// Sample offsets, relative to the segment, of the windows chosen to embed.
    windows: Vec<(usize, usize)>,
}

/* ─────────────────────────────── the pass ─────────────────────────────── */

/// Diarize one meeting and persist the result.
///
/// Blocking and CPU-heavy — a minute or two for an hour-long meeting — so call it
/// from `spawn_blocking`, never on the async runtime.
///
/// Idempotent: nothing is written until one final transaction, so an interrupted run
/// costs only the work, and a re-run overwrites cleanly.
pub fn diarize_meeting(
    app: &AppHandle,
    store: &MeetingStore,
    meeting_id: i64,
    config: &DiarizeConfig,
) -> Result<DiarizationOutcome> {
    let meeting = store
        .get_meeting(meeting_id)?
        .ok_or_else(|| anyhow!("Meeting {meeting_id} no longer exists"))?;

    if meeting.diarized {
        return Ok(skipped(SkipReason::AlreadyDiarized));
    }
    let Some(system_file) = meeting.system_file.clone() else {
        return Ok(skipped(SkipReason::NoSystemAudio));
    };
    if !model::is_installed(app) {
        return Ok(skipped(SkipReason::ModelNotInstalled));
    }

    let segments = store.system_segments(meeting_id)?;
    if segments.len() < 2 {
        // One segment cannot have two speakers in it as far as this pass is
        // concerned, and there is nothing to cluster against.
        return Ok(skipped(SkipReason::TooFewSegments));
    }

    let audio_path = store.audio_path(&system_file);
    let fbank = fbank::FbankComputer::new(fbank::FbankConfig::default());
    let model_path = model::model_path(app)?;
    let mut embedder = match embed::SpeakerEmbedder::new(&model_path, fbank) {
        Ok(embedder) => embedder,
        Err(error) => {
            // A truncated or corrupt download. Removing it means the next attempt
            // re-fetches rather than failing identically forever.
            warn!("Speaker model at {model_path:?} is unusable ({error}); removing it");
            let _ = std::fs::remove_file(&model_path);
            return Ok(skipped(SkipReason::ModelUnusable));
        }
    };

    // Voiced windows for every segment, then embeddings. Two passes rather than one
    // so the whole set of window offsets is known before any inference runs, which
    // is what lets windows batch.
    let mut vad = match crate::audio_toolkit::SileroVad::new(silero_path(app)?, 0.5) {
        Ok(vad) => vad,
        Err(error) => {
            warn!("Diarization could not load the VAD: {error}");
            return Ok(skipped(SkipReason::ModelUnusable));
        }
    };

    let mut reader = match hound::WavReader::open(&audio_path) {
        Ok(reader) => reader,
        Err(error) => {
            // An interrupted meeting's WAV may never have had its RIFF header
            // finalised, which makes it undecodable. Expected, not exceptional.
            warn!("Diarization could not read {audio_path:?}: {error}");
            return Ok(skipped(SkipReason::AudioUnreadable));
        }
    };

    let total = segments.len() as u32;
    let mut prepared: Vec<SegmentAudio> = Vec::with_capacity(segments.len());
    let mut embeddings: Vec<Vec<f32>> = Vec::with_capacity(segments.len());

    for (index, segment) in segments.iter().enumerate() {
        let samples = match read_segment(&mut reader, segment.start_ms, segment.end_ms) {
            Ok(samples) => samples,
            Err(error) => {
                debug!(
                    "Diarization skipped segment {} of meeting {meeting_id}: {error}",
                    segment.id
                );
                continue;
            }
        };

        let (windows, voiced_ms) = voiced_windows(&samples, &mut vad, config);
        if windows.is_empty() || voiced_ms < config.min_voiced_for_label_ms {
            continue;
        }

        let clips: Vec<Vec<f32>> = windows
            .iter()
            .map(|(from, to)| samples[*from..*to].to_vec())
            .collect();
        let embedded = match embedder.embed_batch(&clips) {
            Ok(embedded) if !embedded.is_empty() => embedded,
            Ok(_) => continue,
            Err(error) => {
                debug!(
                    "Diarization could not embed segment {}: {error}",
                    segment.id
                );
                continue;
            }
        };

        embeddings.push(mean_embedding(&embedded));
        prepared.push(SegmentAudio {
            row_id: segment.id,
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            voiced_ms,
            windows,
        });

        // Every 16 segments: often enough to look alive, rarely enough not to
        // re-trigger the wry per-emit leak over a thousand segments.
        if index % 16 == 0 {
            let _ = app.emit(
                PROGRESS_EVENT,
                DiarizationProgress {
                    meeting_id,
                    done: index as u32,
                    total,
                },
            );
        }
    }

    if prepared.len() < 2 {
        return Ok(skipped(SkipReason::TooFewSegments));
    }

    let duration_ms = meeting
        .duration_secs()
        .map(|seconds| seconds * 1_000)
        .unwrap_or_else(|| prepared.last().map_or(0, |last| last.end_ms));

    match assign_speakers(&prepared, &embeddings, duration_ms, config) {
        SpeakerAssignment::OneSpeaker => {
            // Nothing to relabel, but the flag must still be set or this pass
            // re-runs on every open to reach the same conclusion.
            store.mark_diarized(meeting_id)?;
            info!("Meeting {meeting_id}: one voice on the system side; left as \"Others\"");
            Ok(DiarizationOutcome::OneSpeaker)
        }
        SpeakerAssignment::Speakers { spans, count } => {
            // One transaction, at the end. Nothing before this point has written
            // anything, so an interrupted run costs only the work.
            let updated = store.apply_diarization(meeting_id, &spans)?;
            info!(
                "Meeting {meeting_id}: diarization found {count} speaker(s) and relabelled \
                 {updated} segment(s)"
            );
            let _ = app.emit(
                PROGRESS_EVENT,
                DiarizationProgress {
                    meeting_id,
                    done: total,
                    total,
                },
            );
            Ok(DiarizationOutcome::Labelled {
                speakers: count,
                segments: updated,
            })
        }
    }
}

fn skipped(reason: SkipReason) -> DiarizationOutcome {
    DiarizationOutcome::Skipped { reason }
}

/// Where the bundled Silero VAD lives.
fn silero_path(app: &AppHandle) -> Result<std::path::PathBuf> {
    use tauri::Manager;
    app.path()
        .resolve(
            "resources/models/silero_vad_v4.onnx",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| anyhow!("Could not resolve the VAD model path: {e}"))
}

/// Mean of several window embeddings, re-normalised.
fn mean_embedding(embeddings: &[Vec<f32>]) -> Vec<f32> {
    let dimensions = embeddings.first().map_or(0, Vec::len);
    let mut mean = vec![0.0f32; dimensions];
    for embedding in embeddings {
        for (slot, value) in mean.iter_mut().zip(embedding) {
            *slot += value;
        }
    }
    if !embeddings.is_empty() {
        for value in mean.iter_mut() {
            *value /= embeddings.len() as f32;
        }
    }
    cluster::normalize(&mut mean);
    mean
}

/// Read one segment's samples from the system WAV by seeking.
///
/// The meeting WAV is 16 kHz mono **32-bit float** (see `session::open_wav`).
/// `audio_toolkit::audio::utils::read_wav_samples` reads `i16` and would return
/// silent garbage from this file, which is why this exists separately. Seeking
/// rather than slurping is not an optimisation: three hours is ~691 MB.
fn read_segment(
    reader: &mut hound::WavReader<std::io::BufReader<std::fs::File>>,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<f32>> {
    let rate = WHISPER_SAMPLE_RATE as i64;
    let from = (start_ms.max(0) * rate / 1_000) as u32;
    let to = (end_ms.max(0) * rate / 1_000) as u32;
    if to <= from {
        return Err(anyhow!("empty segment"));
    }

    reader
        .seek(from)
        .map_err(|e| anyhow!("could not seek to {from}: {e}"))?;

    let wanted = (to - from) as usize;
    let samples: Vec<f32> = reader
        .samples::<f32>()
        .take(wanted)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| anyhow!("could not read samples: {e}"))?;

    if samples.is_empty() {
        return Err(anyhow!("no samples at {start_ms}ms"));
    }
    Ok(samples)
}

/// Find voiced audio inside one segment and cut it into fixed-length windows.
///
/// Returns the window offsets and how much voiced audio was found.
///
/// A segment deliberately contains its internal pauses — `chunker.rs` keeps them
/// because the transcriber uses them — and embedding silence dilutes the voice. Fixed
/// length is what lets several windows share one tensor.
///
/// When the segment is voiced but too short for even one full window, one window is
/// still taken from its start: a 900 ms turn is a real turn, and the model tolerates
/// a short input better than the pass tolerates dropping the row.
fn voiced_windows(
    samples: &[f32],
    vad: &mut crate::audio_toolkit::SileroVad,
    config: &DiarizeConfig,
) -> (Vec<(usize, usize)>, i64) {
    use crate::audio_toolkit::vad::VoiceActivityDetector;

    let rate = WHISPER_SAMPLE_RATE as usize;
    let frame = rate * 30 / 1_000;
    let window = (config.window_ms as usize * rate) / 1_000;
    let hop = (config.hop_ms as usize * rate) / 1_000;

    // Voiced frames, as a run-length map over the segment.
    let mut voiced = vec![false; samples.len() / frame.max(1)];
    let mut voiced_frames = 0usize;
    for (index, chunk) in samples.chunks_exact(frame).enumerate() {
        if index >= voiced.len() {
            break;
        }
        let speech = vad.is_voice(chunk).unwrap_or(false);
        voiced[index] = speech;
        if speech {
            voiced_frames += 1;
        }
    }
    let voiced_ms = (voiced_frames * 30) as i64;
    if voiced_frames == 0 {
        return (Vec::new(), 0);
    }

    // Candidate windows are those whose midpoint frame is voiced — cheap, and it
    // avoids centring a window on a pause between two words.
    let mut candidates: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    while start + window <= samples.len() {
        let middle_frame = (start + window / 2) / frame;
        if voiced.get(middle_frame).copied().unwrap_or(false) {
            candidates.push((start, start + window));
        }
        start += hop.max(1);
    }

    if candidates.is_empty() {
        // Voiced but shorter than one window, or all the voiced audio sits at the
        // edges. Take what there is rather than dropping a real turn.
        let end = samples.len().min(window.max(frame * 4));
        return (vec![(0, end)], voiced_ms);
    }

    // Spread the chosen windows across the segment rather than taking the first N:
    // consecutive windows overlap by half and describe almost the same audio, so
    // the first three of a long turn would be nearly one sample.
    (
        pick_spread(&candidates, config.max_windows_per_segment),
        voiced_ms,
    )
}

/// Take at most `count` items, spread evenly across the input.
fn pick_spread<T: Clone>(items: &[T], count: usize) -> Vec<T> {
    if items.is_empty() || count == 0 {
        return Vec::new();
    }
    if items.len() <= count {
        return items.to_vec();
    }
    (0..count)
        .map(|i| items[i * (items.len() - 1) / (count - 1).max(1)].clone())
        .collect()
}

/* ────────────────────────────── pure: the decision ────────────────────── */

/// What the clustering decided, before anything is written.
///
/// Its own type rather than [`DiarizationOutcome`] so the pure decision does not
/// have to know about row counts, which only the store can report.
#[derive(Clone, Debug, PartialEq)]
enum SpeakerAssignment {
    OneSpeaker,
    Speakers {
        spans: Vec<DiarizedSpan>,
        count: usize,
    },
}

/// Cluster the embeddings and turn them into spans.
///
/// Separated from [`diarize_meeting`] and pure, because every interesting failure
/// here is a decision — a threshold, a dissolved cluster, a confidence — and a pure
/// function can be tested for all of them without a WAV, a model or a clock.
fn assign_speakers(
    segments: &[SegmentAudio],
    embeddings: &[Vec<f32>],
    duration_ms: i64,
    config: &DiarizeConfig,
) -> SpeakerAssignment {
    debug_assert_eq!(segments.len(), embeddings.len());

    // Segments with too little voiced audio are held out: a noisy embedding must not
    // be able to invent a speaker. They are assigned afterwards by nearest centroid.
    let confident: Vec<usize> = (0..segments.len())
        .filter(|i| segments[*i].voiced_ms >= config.min_voiced_for_linkage_ms)
        .collect();
    // If holding them out leaves too few, cluster everything instead — a meeting of
    // short exchanges is still a meeting with two people in it.
    let linkage_indices: Vec<usize> = if confident.len() < 2 {
        (0..segments.len()).collect()
    } else {
        confident
    };

    // O(n²) memory in the linkage, so a very long meeting clusters a uniform sample
    // and assigns the rest by nearest centroid.
    let sampled = pick_spread(&linkage_indices, config.max_linkage_segments);
    let linkage_embeddings: Vec<Vec<f32>> =
        sampled.iter().map(|i| embeddings[*i].clone()).collect();

    let threshold = cluster::threshold_for_duration(duration_ms);
    let Some(mut clustering) = cluster::cluster_cosine(&linkage_embeddings, threshold) else {
        return SpeakerAssignment::OneSpeaker;
    };

    let speech_ms: Vec<i64> = sampled.iter().map(|i| segments[*i].voiced_ms).collect();
    cluster::dissolve_small_clusters(&mut clustering, &speech_ms, config.min_cluster_speech_ms);
    cluster::cap_speakers(&mut clustering, config.max_speakers);

    if clustering.num_clusters <= 1 {
        // The guards collapsed it to one voice, which is the same answer as the
        // linkage never separating them in the first place.
        return SpeakerAssignment::OneSpeaker;
    }

    // Every segment is assigned against the final centroids, including the ones held
    // out of the linkage — that is the whole point of holding them out rather than
    // dropping them.
    let spans = spans_from_clustering(segments, embeddings, &clustering);
    SpeakerAssignment::Speakers {
        spans,
        count: clustering.num_clusters,
    }
}

/// Spans for every segment, from a finished clustering.
///
/// Separate from [`assign_speakers`] so the mapping from a cluster index to a stable
/// `spk_N` key and a `Speaker N` label lives in exactly one place. Numbering is
/// 1-based to match the label the user reads.
fn spans_from_clustering(
    segments: &[SegmentAudio],
    embeddings: &[Vec<f32>],
    clustering: &cluster::Clustering,
) -> Vec<DiarizedSpan> {
    segments
        .iter()
        .zip(embeddings)
        .filter_map(|(segment, embedding)| {
            let (index, confidence) = cluster::assign_nearest(embedding, &clustering.centroids)?;
            Some(DiarizedSpan {
                // Copied verbatim from the row, so `apply_diarization`'s midpoint
                // containment matches exactly one span with no ambiguity.
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                speaker_key: format!("spk_{}", index + 1),
                default_label: format!("Speaker {}", index + 1),
                confidence: Some(confidence),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(row_id: i64, start_ms: i64, voiced_ms: i64) -> SegmentAudio {
        SegmentAudio {
            row_id,
            start_ms,
            end_ms: start_ms + 3_000,
            voiced_ms,
            windows: vec![(0, 24_000)],
        }
    }

    fn embedding(axis: usize) -> Vec<f32> {
        let mut vector = vec![0.0f32; 8];
        vector[axis % 8] = 1.0;
        cluster::normalize(&mut vector);
        vector
    }

    #[test]
    fn spreading_takes_the_ends_and_the_middle() {
        let items: Vec<usize> = (0..10).collect();
        let picked = pick_spread(&items, 3);
        assert_eq!(picked.len(), 3);
        assert_eq!(picked[0], 0);
        assert_eq!(picked[2], 9);
    }

    #[test]
    fn spreading_fewer_items_than_asked_for_returns_them_all() {
        let items = vec![1, 2];
        assert_eq!(pick_spread(&items, 5), vec![1, 2]);
    }

    #[test]
    fn spreading_nothing_yields_nothing() {
        let empty: Vec<usize> = Vec::new();
        assert!(pick_spread(&empty, 3).is_empty());
        assert!(pick_spread(&[1, 2, 3], 0).is_empty());
    }

    #[test]
    fn the_mean_embedding_is_normalized() {
        let mean = mean_embedding(&[embedding(0), embedding(1)]);
        let norm = mean.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_mean_of_one_embedding_is_itself() {
        let single = embedding(3);
        assert_eq!(mean_embedding(&[single.clone()]), single);
    }

    /* ─────────────────────── the decision, end to end ─────────────────── */

    fn config() -> DiarizeConfig {
        DiarizeConfig {
            // Small enough that the fixtures below are not all held out of the
            // linkage for being too short.
            min_voiced_for_linkage_ms: 500,
            min_cluster_speech_ms: 500,
            ..Default::default()
        }
    }

    /// The output the whole feature exists for: two voices become `spk_1` and
    /// `spk_2`, and every segment gets a span.
    #[test]
    fn two_voices_produce_numbered_speakers() {
        let segments: Vec<SegmentAudio> = (0..6)
            .map(|i| segment(i as i64 + 1, i as i64 * 4_000, 3_000))
            .collect();
        let embeddings: Vec<Vec<f32>> = (0..6)
            .map(|i| embedding(if i % 2 == 0 { 0 } else { 4 }))
            .collect();

        let SpeakerAssignment::Speakers { spans, count } =
            assign_speakers(&segments, &embeddings, 10 * 60 * 1_000, &config())
        else {
            panic!("two distinct voices must not resolve to one speaker");
        };

        assert_eq!(count, 2);
        assert_eq!(spans.len(), 6, "every segment must get a span");

        let keys: std::collections::BTreeSet<&str> =
            spans.iter().map(|span| span.speaker_key.as_str()).collect();
        assert_eq!(
            keys,
            ["spk_1", "spk_2"].into_iter().collect(),
            "keys must be 1-based and match the labels"
        );
        for span in &spans {
            assert!(span.default_label.starts_with("Speaker "));
            assert!(span.confidence.is_some());
        }
    }

    /// Spans must carry the row's own timings verbatim, because
    /// `apply_diarization` matches them by midpoint containment — an approximated
    /// span would match the wrong row or none.
    #[test]
    fn spans_copy_the_segment_timings_exactly() {
        let segments = vec![
            segment(1, 0, 3_000),
            segment(2, 7_000, 3_000),
            segment(3, 14_000, 3_000),
            segment(4, 21_000, 3_000),
        ];
        let embeddings = vec![embedding(0), embedding(4), embedding(0), embedding(4)];

        let SpeakerAssignment::Speakers { spans, .. } =
            assign_speakers(&segments, &embeddings, 60_000, &config())
        else {
            panic!("expected two speakers");
        };

        for (span, segment) in spans.iter().zip(&segments) {
            assert_eq!(span.start_ms, segment.start_ms);
            assert_eq!(span.end_ms, segment.end_ms);
        }
    }

    /// One remote voice must answer `OneSpeaker`, not one cluster: renaming it to
    /// "Speaker 1" would tell the user the app had told several voices apart.
    #[test]
    fn one_voice_answers_one_speaker() {
        let segments: Vec<SegmentAudio> = (0..5)
            .map(|i| segment(i as i64 + 1, i as i64 * 4_000, 3_000))
            .collect();
        let embeddings: Vec<Vec<f32>> = (0..5).map(|_| embedding(0)).collect();

        assert_eq!(
            assign_speakers(&segments, &embeddings, 60_000, &config()),
            SpeakerAssignment::OneSpeaker
        );
    }

    /// The guard against "46 speakers on a 73-minute recording": the same set of
    /// embeddings must not fragment further as the recording gets longer.
    #[test]
    fn a_longer_meeting_never_finds_more_speakers() {
        let segments: Vec<SegmentAudio> = (0..8)
            .map(|i| segment(i as i64 + 1, i as i64 * 4_000, 3_000))
            .collect();
        // Eight mutually distant voices: the pathological input.
        let embeddings: Vec<Vec<f32>> = (0..8).map(embedding).collect();

        let count_at = |duration_ms: i64| match assign_speakers(
            &segments,
            &embeddings,
            duration_ms,
            &config(),
        ) {
            SpeakerAssignment::OneSpeaker => 1,
            SpeakerAssignment::Speakers { count, .. } => count,
        };

        let short = count_at(5 * 60 * 1_000);
        let long = count_at(90 * 60 * 1_000);
        assert!(
            long <= short,
            "a 90-minute meeting found {long} speakers where a 5-minute one found {short}"
        );
    }

    /// Eleven detected voices is a clustering failure, not an eleven-person call.
    #[test]
    fn the_speaker_count_is_capped() {
        let segments: Vec<SegmentAudio> = (0..8)
            .map(|i| segment(i as i64 + 1, i as i64 * 4_000, 3_000))
            .collect();
        let embeddings: Vec<Vec<f32>> = (0..8).map(embedding).collect();

        let capped = DiarizeConfig {
            max_speakers: 3,
            ..config()
        };
        match assign_speakers(&segments, &embeddings, 60_000, &capped) {
            SpeakerAssignment::Speakers { count, spans } => {
                assert!(count <= 3, "found {count} speakers with a cap of 3");
                assert_eq!(spans.len(), 8);
            }
            SpeakerAssignment::OneSpeaker => {}
        }
    }

    /// A segment too short to trust is held out of the linkage but must still be
    /// labelled afterwards — dropping it would leave a gap in the transcript.
    #[test]
    fn short_segments_are_still_labelled() {
        let segments = vec![
            segment(1, 0, 3_000),
            segment(2, 4_000, 3_000),
            segment(3, 8_000, 3_000),
            // Well under `min_voiced_for_linkage_ms`.
            segment(4, 12_000, 300),
        ];
        let embeddings = vec![embedding(0), embedding(4), embedding(0), embedding(4)];

        let SpeakerAssignment::Speakers { spans, .. } =
            assign_speakers(&segments, &embeddings, 60_000, &config())
        else {
            panic!("expected two speakers");
        };
        assert_eq!(
            spans.len(),
            4,
            "the short segment must be assigned, not dropped"
        );
    }

    /// A cluster built from almost no speech is a cough, not a participant.
    #[test]
    fn a_negligible_cluster_does_not_become_a_speaker() {
        let segments = vec![
            segment(1, 0, 8_000),
            segment(2, 10_000, 8_000),
            segment(3, 20_000, 8_000),
            // A 100 ms blip with its own distinct embedding.
            segment(4, 30_000, 100),
        ];
        let embeddings = vec![embedding(0), embedding(0), embedding(4), embedding(2)];

        let strict = DiarizeConfig {
            min_voiced_for_linkage_ms: 50,
            min_cluster_speech_ms: 1_000,
            ..Default::default()
        };
        match assign_speakers(&segments, &embeddings, 60_000, &strict) {
            SpeakerAssignment::Speakers { count, .. } => {
                assert!(count <= 2, "the 100 ms blip became speaker {count}")
            }
            SpeakerAssignment::OneSpeaker => {}
        }
    }
}
