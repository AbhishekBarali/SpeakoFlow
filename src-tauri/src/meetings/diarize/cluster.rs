//! Grouping speaker embeddings into speakers.
//!
//! Pure: embeddings and a threshold in, cluster assignments out. No models, no
//! I/O, no clock — the same discipline as `chunker.rs` and
//! `history::resolve_retention_plan`, for the same reason. Every bug this will
//! ever have is in a decision, and a decision that reads the world cannot be
//! tested for.
//!
//! `kodama` supplies the linkage. The flat cut and all four guards are written
//! here, because kodama has no `fcluster` equivalent and because the guards are
//! where the product judgement lives rather than the mathematics.
//!
//! # The threshold, and why it moves with duration
//!
//! pyannote 3.1 ships **0.7045** for exactly these weights
//! (`wespeaker-voxceleb-resnet34-LM`), which is the main reason that model was
//! chosen — the tuned number transfers rather than having to be rediscovered.
//!
//! But a fixed threshold fails at long durations, and it fails in the direction
//! that looks broken. A voice drifts over an hour: someone leans back, changes
//! headset, gets tired, and their later embeddings sit further from their earlier
//! ones than two different people's do in a five-minute call. OpenWhispr shipped a
//! fixed threshold and a 73-minute recording came back with **46 speakers**. So the
//! threshold ramps with recorded length ([`threshold_for_duration`]) — looser for a
//! long meeting, tighter for a short one.
//!
//! The asymmetry matters too. Merging two people reads as a mildly confusing
//! transcript; splitting one person into "Speaker 2" and "Speaker 4" reads as
//! broken software. So every choice here errs toward **fewer** speakers.

use kodama::{linkage, Method};

/// Cluster threshold for a short recording.
///
/// Below [`RAMP_START_MS`] a voice has not had time to drift, so the tight
/// threshold is safe and buys precision.
pub const THRESHOLD_SHORT: f32 = 0.55;

/// Cluster threshold for a long recording.
///
/// Above [`RAMP_END_MS`], looser: an hour of the same voice spans more embedding
/// distance than two voices do in five minutes.
pub const THRESHOLD_LONG: f32 = 0.80;

/// Recording length at which the ramp begins.
pub const RAMP_START_MS: i64 = 15 * 60 * 1_000;

/// Recording length at which the ramp reaches [`THRESHOLD_LONG`].
pub const RAMP_END_MS: i64 = 60 * 60 * 1_000;

/// Clusters holding less speech than this are dissolved into their nearest
/// neighbour.
///
/// The analogue of pyannote's `min_cluster_size`, and what stops a cough, a door,
/// or a burst of hold music from becoming a participant. A one-sentence
/// `Speaker 4` who appears once and never again is the most obviously wrong output
/// this pass can produce.
pub const MIN_CLUSTER_SPEECH_MS: i64 = 1_000;

/// Hard cap on speakers.
///
/// Eleven detected voices on a call is a clustering failure, not an eleven-person
/// call — and eleven labels would be useless to read even if it were true.
pub const MAX_SPEAKERS: usize = 10;

/// Flat clustering result, numbered by first appearance.
#[derive(Clone, Debug, PartialEq)]
pub struct Clustering {
    /// One 0-based cluster index per input embedding, in input order.
    pub labels: Vec<usize>,
    pub num_clusters: usize,
    /// Cluster centroids, L2-normalised. Kept so short segments held out of the
    /// linkage can be assigned afterwards, and so confidence can be scored.
    pub centroids: Vec<Vec<f32>>,
}

/// The threshold to use for a recording of `duration_ms`.
///
/// Linear between [`RAMP_START_MS`] and [`RAMP_END_MS`]. Linear rather than
/// anything cleverer because there is no principled curve here — it is an
/// empirical correction for embedding drift, and two anchor points plus a straight
/// line between them is all the evidence supports.
pub fn threshold_for_duration(duration_ms: i64) -> f32 {
    if duration_ms <= RAMP_START_MS {
        return THRESHOLD_SHORT;
    }
    if duration_ms >= RAMP_END_MS {
        return THRESHOLD_LONG;
    }
    let span = (RAMP_END_MS - RAMP_START_MS) as f32;
    let progress = (duration_ms - RAMP_START_MS) as f32 / span;
    THRESHOLD_SHORT + (THRESHOLD_LONG - THRESHOLD_SHORT) * progress
}

/// Cosine distance in `[0, 2]` between two L2-normalised embeddings.
///
/// `1 - dot` rather than the full cosine formula: the caller normalises, which
/// makes the denominators one. That also gives the identity
/// `‖a-b‖² = 2(1-cos)`, which is what makes a squared-Euclidean threshold and a
/// cosine threshold interchangeable at a factor of two.
pub fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    (1.0 - dot).clamp(0.0, 2.0)
}

/// L2-normalise in place. A zero vector is left alone rather than producing NaNs.
pub fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for value in vector.iter_mut() {
            *value /= norm;
        }
    }
}

/// Cluster with average linkage on cosine distance, cut at `threshold`.
///
/// Average linkage rather than kodama's `Centroid`: centroid, median and Ward are
/// Euclidean-geometry methods, only correct on squared-Euclidean input. Average
/// linkage on cosine distance is unambiguously valid and behaves very similarly at
/// the same threshold.
///
/// Returns `None` when the tallest merge in the dendrogram is still below
/// `threshold` — every embedding is one voice. That is a real answer, and the
/// caller must handle it differently from a one-element cluster list: see
/// `DiarizationOutcome::OneSpeaker`.
pub fn cluster_cosine(embeddings: &[Vec<f32>], threshold: f32) -> Option<Clustering> {
    if embeddings.len() < 2 {
        return None;
    }

    // Condensed upper-triangular distance matrix, which is the layout kodama takes.
    let n = embeddings.len();
    let mut condensed = Vec::with_capacity(n * (n - 1) / 2);
    for i in 0..n {
        for j in (i + 1)..n {
            condensed.push(cosine_distance(&embeddings[i], &embeddings[j]));
        }
    }

    let tallest = condensed.iter().cloned().fold(0.0f32, f32::max);
    if tallest < threshold {
        // Everything is within the threshold of everything else: one voice.
        return None;
    }

    let dendrogram = linkage(&mut condensed, n, Method::Average);
    let labels = flat_cut(&dendrogram, n, threshold);
    Some(finish(embeddings, labels))
}

/// Cut a kodama dendrogram at `threshold` into flat clusters.
///
/// Union-find over the merge steps whose dissimilarity is below the threshold.
/// Its own function because there is no upstream reference implementation to check
/// against — kodama stops at the dendrogram — so it carries its own tests.
///
/// kodama's cluster ids: `0..n` are the observations, and the cluster formed at
/// step `i` is `n + i`. So a step's operands may be observations or earlier
/// clusters, and resolving them needs the same union-find that produces the answer.
fn flat_cut(dendrogram: &kodama::Dendrogram<f32>, n: usize, threshold: f32) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..n + dendrogram.len()).collect();

    fn find(parent: &mut Vec<usize>, mut node: usize) -> usize {
        while parent[node] != node {
            let grandparent = parent[parent[node]];
            parent[node] = grandparent;
            node = grandparent;
        }
        node
    }

    for (step_index, step) in dendrogram.steps().iter().enumerate() {
        let merged_id = n + step_index;
        if step.dissimilarity >= threshold {
            // Above the cut. The merged node still has to exist as a parent for
            // later steps to reference, but it must not join its two children —
            // so it is left as its own root and the children keep theirs.
            continue;
        }
        let a = find(&mut parent, step.cluster1);
        let b = find(&mut parent, step.cluster2);
        if a != b {
            parent[b] = a;
        }
        // The merged id joins the same component, so a later step referencing it
        // resolves to the right root.
        let root = find(&mut parent, a);
        parent[merged_id] = root;
    }

    // Roots to dense, 0-based labels, numbered by first appearance so `spk_1` is
    // always whoever spoke first.
    let mut mapping: Vec<(usize, usize)> = Vec::new();
    let mut labels = Vec::with_capacity(n);
    for observation in 0..n {
        let root = find(&mut parent, observation);
        let label = match mapping.iter().find(|(candidate, _)| *candidate == root) {
            Some((_, label)) => *label,
            None => {
                let label = mapping.len();
                mapping.push((root, label));
                label
            }
        };
        labels.push(label);
    }
    labels
}

/// Build the result: cluster count and normalised centroids.
fn finish(embeddings: &[Vec<f32>], labels: Vec<usize>) -> Clustering {
    let num_clusters = labels.iter().copied().max().map_or(0, |max| max + 1);
    let dimensions = embeddings.first().map_or(0, Vec::len);

    let mut centroids = vec![vec![0.0f32; dimensions]; num_clusters];
    let mut counts = vec![0usize; num_clusters];
    for (embedding, label) in embeddings.iter().zip(&labels) {
        counts[*label] += 1;
        for (slot, value) in centroids[*label].iter_mut().zip(embedding) {
            *slot += value;
        }
    }
    for (centroid, count) in centroids.iter_mut().zip(&counts) {
        if *count > 0 {
            for value in centroid.iter_mut() {
                *value /= *count as f32;
            }
        }
        normalize(centroid);
    }

    Clustering {
        labels,
        num_clusters,
        centroids,
    }
}

/// Dissolve clusters holding less than `min_speech_ms` of audio into their nearest
/// surviving neighbour.
///
/// `speech_ms` is parallel to `clustering.labels`. Runs before anything is
/// numbered, so a dissolved cluster leaves no gap in the labels.
///
/// If *every* cluster is below the floor — a very short recording — nothing is
/// dissolved. Dissolving them all would leave no clusters at all, which is worse
/// than keeping small ones.
pub fn dissolve_small_clusters(clustering: &mut Clustering, speech_ms: &[i64], min_speech_ms: i64) {
    if clustering.num_clusters <= 1 {
        return;
    }

    let mut totals = vec![0i64; clustering.num_clusters];
    for (label, duration) in clustering.labels.iter().zip(speech_ms) {
        totals[*label] += duration;
    }

    let survivors: Vec<usize> = (0..clustering.num_clusters)
        .filter(|label| totals[*label] >= min_speech_ms)
        .collect();
    if survivors.is_empty() || survivors.len() == clustering.num_clusters {
        return;
    }

    // Each doomed cluster's members go to the nearest surviving centroid.
    let mut reassigned = clustering.labels.clone();
    for (index, label) in clustering.labels.iter().enumerate() {
        if survivors.contains(label) {
            continue;
        }
        let doomed = &clustering.centroids[*label];
        let nearest = survivors
            .iter()
            .copied()
            .min_by(|a, b| {
                cosine_distance(doomed, &clustering.centroids[*a])
                    .total_cmp(&cosine_distance(doomed, &clustering.centroids[*b]))
            })
            .unwrap_or(survivors[0]);
        reassigned[index] = nearest;
    }

    renumber(clustering, reassigned);
}

/// Merge the closest clusters until at most `max` remain.
pub fn cap_speakers(clustering: &mut Clustering, max: usize) {
    let max = max.max(1);
    while clustering.num_clusters > max {
        // Closest pair of centroids.
        let mut best: Option<(usize, usize, f32)> = None;
        for a in 0..clustering.num_clusters {
            for b in (a + 1)..clustering.num_clusters {
                let distance = cosine_distance(&clustering.centroids[a], &clustering.centroids[b]);
                if best.is_none_or(|(_, _, current)| distance < current) {
                    best = Some((a, b, distance));
                }
            }
        }
        let Some((keep, drop, _)) = best else { break };

        let reassigned: Vec<usize> = clustering
            .labels
            .iter()
            .map(|label| if *label == drop { keep } else { *label })
            .collect();
        renumber(clustering, reassigned);
    }
}

/// Rebuild a clustering from reassigned labels, compacting them to `0..k` in
/// first-appearance order and recomputing centroids from scratch.
///
/// Recomputing rather than adjusting: a merged centroid is the mean of its new
/// membership, and incrementally averaging two centroids weights them equally
/// regardless of how many segments each held, which biases every later distance.
fn renumber(clustering: &mut Clustering, reassigned: Vec<usize>) {
    let mut mapping: Vec<(usize, usize)> = Vec::new();
    let mut labels = Vec::with_capacity(reassigned.len());
    for old in &reassigned {
        let label = match mapping.iter().find(|(candidate, _)| candidate == old) {
            Some((_, label)) => *label,
            None => {
                let label = mapping.len();
                mapping.push((*old, label));
                label
            }
        };
        labels.push(label);
    }

    let num_clusters = mapping.len();
    let dimensions = clustering.centroids.first().map_or(0, Vec::len);
    let mut centroids = vec![vec![0.0f32; dimensions]; num_clusters];
    let mut counts = vec![0usize; num_clusters];
    for (old_label, new_label) in reassigned.iter().zip(&labels) {
        counts[*new_label] += 1;
        for (slot, value) in centroids[*new_label]
            .iter_mut()
            .zip(&clustering.centroids[*old_label])
        {
            *slot += value;
        }
    }
    for (centroid, count) in centroids.iter_mut().zip(&counts) {
        if *count > 0 {
            for value in centroid.iter_mut() {
                *value /= *count as f32;
            }
        }
        normalize(centroid);
    }

    clustering.labels = labels;
    clustering.num_clusters = num_clusters;
    clustering.centroids = centroids;
}

/// The nearest centroid to `embedding`, and how confident that choice is.
///
/// Confidence is the **margin** between the nearest and second-nearest centroid,
/// scaled into `[0, 1]`. A margin near zero is what overlapping speech looks like
/// from here — the embedding sits between two voices — so this is the number that
/// makes the transcript's de-emphasis of low-confidence labels mean something
/// rather than being decoration.
///
/// With one cluster there is no second-nearest, so confidence is 1.0: the choice
/// is not uncertain, there simply is no alternative.
pub fn assign_nearest(embedding: &[f32], centroids: &[Vec<f32>]) -> Option<(usize, f32)> {
    if centroids.is_empty() {
        return None;
    }

    let mut distances: Vec<(usize, f32)> = centroids
        .iter()
        .enumerate()
        .map(|(index, centroid)| (index, cosine_distance(embedding, centroid)))
        .collect();
    distances.sort_by(|a, b| a.1.total_cmp(&b.1));

    let (nearest, closest) = distances[0];
    let confidence = match distances.get(1) {
        // The margin is a cosine distance difference, so it lives in [0, 2]; half
        // of it is a generous full-confidence point and keeps ordinary,
        // well-separated speakers near 1.0 instead of bunched at 0.3.
        Some((_, second)) => ((second - closest) / 0.5).clamp(0.0, 1.0),
        None => 1.0,
    };
    Some((nearest, confidence))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic embedding: a unit vector pointing mostly along axis `axis`, with
    /// `jitter` mixed into the next axis to simulate within-speaker variation.
    fn embedding(axis: usize, jitter: f32) -> Vec<f32> {
        let mut vector = vec![0.0f32; 8];
        vector[axis % 8] = 1.0;
        vector[(axis + 1) % 8] = jitter;
        normalize(&mut vector);
        vector
    }

    /* ─────────────────────────── the threshold ramp ─────────────────────── */

    /// The correction that exists because a fixed threshold gave 46 speakers on a
    /// 73-minute recording.
    #[test]
    fn the_threshold_loosens_with_duration() {
        assert_eq!(threshold_for_duration(0), THRESHOLD_SHORT);
        assert_eq!(threshold_for_duration(5 * 60 * 1_000), THRESHOLD_SHORT);
        assert_eq!(threshold_for_duration(RAMP_START_MS), THRESHOLD_SHORT);
        assert_eq!(threshold_for_duration(RAMP_END_MS), THRESHOLD_LONG);
        // A three-hour recording stays at the loose end rather than extrapolating
        // past it, which would eventually merge everyone into one speaker.
        assert_eq!(threshold_for_duration(3 * RAMP_END_MS), THRESHOLD_LONG);
    }

    #[test]
    fn the_ramp_is_monotonic_and_lands_in_between() {
        let midpoint = threshold_for_duration((RAMP_START_MS + RAMP_END_MS) / 2);
        assert!(midpoint > THRESHOLD_SHORT && midpoint < THRESHOLD_LONG);
        let mut previous = 0.0;
        for minutes in 0..90 {
            let value = threshold_for_duration(minutes as i64 * 60 * 1_000);
            assert!(
                value >= previous,
                "the ramp went backwards at {minutes} min"
            );
            previous = value;
        }
    }

    /// A negative duration is a clock artefact, not a reason to pick a wild
    /// threshold.
    #[test]
    fn a_negative_duration_uses_the_tight_threshold() {
        assert_eq!(threshold_for_duration(-1), THRESHOLD_SHORT);
    }

    /* ────────────────────────────── distance ────────────────────────────── */

    #[test]
    fn cosine_distance_is_zero_for_identical_vectors() {
        let a = embedding(0, 0.0);
        assert!(cosine_distance(&a, &a) < 1e-6);
    }

    #[test]
    fn cosine_distance_is_one_for_orthogonal_vectors() {
        let a = embedding(0, 0.0);
        let b = embedding(2, 0.0);
        assert!((cosine_distance(&a, &b) - 1.0).abs() < 1e-6);
    }

    /// Opposed vectors give 2.0, which is why the range is `[0, 2]` and not
    /// `[0, 1]` — a threshold written for the wrong range would cluster everything.
    #[test]
    fn cosine_distance_reaches_two_for_opposed_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((cosine_distance(&a, &b) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn normalizing_a_zero_vector_does_not_produce_nans() {
        let mut zero = vec![0.0f32; 4];
        normalize(&mut zero);
        assert!(zero.iter().all(|value| value.is_finite()));
    }

    /* ────────────────────────────── clustering ────────────────────────── */

    /// The base case: two well-separated voices, several segments each.
    #[test]
    fn two_distinct_voices_become_two_clusters() {
        let embeddings = vec![
            embedding(0, 0.02),
            embedding(0, 0.05),
            embedding(4, 0.02),
            embedding(4, 0.04),
            embedding(0, 0.01),
        ];
        let clustering = cluster_cosine(&embeddings, 0.6).expect("two voices must cluster");
        assert_eq!(clustering.num_clusters, 2);
        // Same voice, same label.
        assert_eq!(clustering.labels[0], clustering.labels[1]);
        assert_eq!(clustering.labels[0], clustering.labels[4]);
        assert_eq!(clustering.labels[2], clustering.labels[3]);
        assert_ne!(clustering.labels[0], clustering.labels[2]);
    }

    /// `None` is a real answer, distinct from one cluster: the caller must not
    /// rename a lone remote voice to "Speaker 1", which would imply others exist.
    #[test]
    fn one_voice_answers_none_rather_than_one_cluster() {
        let embeddings = vec![embedding(0, 0.01), embedding(0, 0.02), embedding(0, 0.015)];
        assert!(cluster_cosine(&embeddings, 0.6).is_none());
    }

    #[test]
    fn fewer_than_two_embeddings_cannot_cluster() {
        assert!(cluster_cosine(&[], 0.6).is_none());
        assert!(cluster_cosine(&[embedding(0, 0.0)], 0.6).is_none());
    }

    /// Numbering by first appearance is what makes `spk_1` always the first
    /// speaker, so a transcript's labels are stable and explicable.
    #[test]
    fn clusters_are_numbered_by_first_appearance() {
        let embeddings = vec![embedding(4, 0.01), embedding(0, 0.01), embedding(4, 0.02)];
        let clustering = cluster_cosine(&embeddings, 0.6).unwrap();
        assert_eq!(clustering.labels[0], 0, "the first embedding is speaker 0");
        assert_eq!(clustering.labels[1], 1);
        assert_eq!(clustering.labels[2], 0);
    }

    /// A looser threshold must never produce *more* clusters. This is the property
    /// the duration ramp relies on being true.
    #[test]
    fn a_looser_threshold_never_finds_more_speakers() {
        let embeddings: Vec<Vec<f32>> = (0..8).map(|i| embedding(i, 0.03)).collect();
        let mut previous = usize::MAX;
        for step in 0..8 {
            let threshold = 0.3 + step as f32 * 0.2;
            let count = cluster_cosine(&embeddings, threshold)
                .map_or(1, |clustering| clustering.num_clusters);
            assert!(
                count <= previous,
                "threshold {threshold} found {count} speakers, more than the {previous} before it"
            );
            previous = count;
        }
    }

    #[test]
    fn every_embedding_gets_exactly_one_label() {
        let embeddings: Vec<Vec<f32>> = (0..6).map(|i| embedding(i, 0.02)).collect();
        let clustering = cluster_cosine(&embeddings, 0.6).unwrap();
        assert_eq!(clustering.labels.len(), embeddings.len());
        assert_eq!(clustering.centroids.len(), clustering.num_clusters);
        assert!(clustering
            .labels
            .iter()
            .all(|label| *label < clustering.num_clusters));
    }

    #[test]
    fn centroids_are_normalized() {
        let embeddings: Vec<Vec<f32>> = (0..6).map(|i| embedding(i, 0.02)).collect();
        let clustering = cluster_cosine(&embeddings, 0.6).unwrap();
        for centroid in &clustering.centroids {
            let norm = centroid.iter().map(|v| v * v).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-5, "centroid norm was {norm}");
        }
    }

    /* ───────────────────────── dissolving small clusters ───────────────── */

    /// What stops one cough from becoming a participant.
    #[test]
    fn a_cluster_with_almost_no_speech_is_dissolved() {
        let embeddings = vec![
            embedding(0, 0.01),
            embedding(0, 0.02),
            embedding(4, 0.01),
            // A single 200 ms blip that clustered on its own.
            embedding(2, 0.01),
        ];
        let mut clustering = cluster_cosine(&embeddings, 0.6).unwrap();
        let before = clustering.num_clusters;
        assert!(before >= 3, "expected the blip to cluster separately");

        dissolve_small_clusters(&mut clustering, &[5_000, 5_000, 5_000, 200], 1_000);
        assert!(
            clustering.num_clusters < before,
            "the 200 ms cluster should have been dissolved"
        );
        // And no gaps in the numbering.
        assert!(clustering
            .labels
            .iter()
            .all(|l| *l < clustering.num_clusters));
        assert_eq!(clustering.centroids.len(), clustering.num_clusters);
    }

    /// A very short recording where every cluster is under the floor must not end
    /// up with no clusters at all.
    #[test]
    fn dissolving_never_removes_every_cluster() {
        let embeddings = vec![embedding(0, 0.01), embedding(4, 0.01)];
        let mut clustering = cluster_cosine(&embeddings, 0.6).unwrap();
        dissolve_small_clusters(&mut clustering, &[100, 100], 5_000);
        assert!(clustering.num_clusters >= 1);
        assert_eq!(clustering.labels.len(), 2);
    }

    #[test]
    fn dissolving_leaves_healthy_clusters_alone() {
        let embeddings = vec![embedding(0, 0.01), embedding(4, 0.01)];
        let mut clustering = cluster_cosine(&embeddings, 0.6).unwrap();
        let before = clustering.clone();
        dissolve_small_clusters(&mut clustering, &[9_000, 9_000], 1_000);
        assert_eq!(clustering.num_clusters, before.num_clusters);
        assert_eq!(clustering.labels, before.labels);
    }

    /* ─────────────────────────────── the cap ─────────────────────────── */

    /// Eleven voices on a call is a clustering failure, not an eleven-person call.
    #[test]
    fn the_speaker_count_is_capped() {
        let embeddings: Vec<Vec<f32>> = (0..8).map(|i| embedding(i, 0.0)).collect();
        let mut clustering = cluster_cosine(&embeddings, 0.4).unwrap();
        assert_eq!(clustering.num_clusters, 8);

        cap_speakers(&mut clustering, 3);
        assert_eq!(clustering.num_clusters, 3);
        assert_eq!(clustering.labels.len(), 8);
        assert_eq!(clustering.centroids.len(), 3);
        assert!(clustering.labels.iter().all(|label| *label < 3));
    }

    #[test]
    fn capping_below_the_current_count_is_a_no_op() {
        let embeddings = vec![embedding(0, 0.0), embedding(4, 0.0)];
        let mut clustering = cluster_cosine(&embeddings, 0.6).unwrap();
        let before = clustering.clone();
        cap_speakers(&mut clustering, MAX_SPEAKERS);
        assert_eq!(clustering, before);
    }

    /// A cap of zero is a caller bug, not a reason to loop forever or produce a
    /// clustering with no clusters.
    #[test]
    fn a_zero_cap_still_leaves_one_speaker() {
        let embeddings = vec![embedding(0, 0.0), embedding(4, 0.0)];
        let mut clustering = cluster_cosine(&embeddings, 0.6).unwrap();
        cap_speakers(&mut clustering, 0);
        assert_eq!(clustering.num_clusters, 1);
    }

    /* ───────────────────────── assignment and confidence ───────────────── */

    #[test]
    fn the_nearest_centroid_wins() {
        let centroids = vec![embedding(0, 0.0), embedding(4, 0.0)];
        let (index, _) = assign_nearest(&embedding(4, 0.02), &centroids).unwrap();
        assert_eq!(index, 1);
    }

    /// The number that makes the transcript's "Uncertain" marker mean something: an
    /// embedding sitting between two voices — overlapping speech — scores near
    /// zero.
    #[test]
    fn an_embedding_between_two_voices_scores_low_confidence() {
        let centroids = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let mut midway = vec![0.5, 0.5];
        normalize(&mut midway);
        let (_, confidence) = assign_nearest(&midway, &centroids).unwrap();
        assert!(
            confidence < 0.15,
            "an ambiguous embedding scored {confidence}"
        );
    }

    #[test]
    fn a_clearly_attributed_embedding_scores_high_confidence() {
        let centroids = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let (index, confidence) = assign_nearest(&[1.0, 0.0], &centroids).unwrap();
        assert_eq!(index, 0);
        assert!(confidence > 0.9, "a clear embedding scored {confidence}");
    }

    /// With one cluster there is no alternative to be uncertain between.
    #[test]
    fn a_single_centroid_is_fully_confident() {
        let centroids = vec![embedding(0, 0.0)];
        let (index, confidence) = assign_nearest(&embedding(4, 0.0), &centroids).unwrap();
        assert_eq!(index, 0);
        assert_eq!(confidence, 1.0);
    }

    #[test]
    fn assigning_against_no_centroids_yields_none() {
        assert!(assign_nearest(&embedding(0, 0.0), &[]).is_none());
    }
}
