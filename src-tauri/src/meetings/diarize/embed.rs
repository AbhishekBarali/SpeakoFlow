//! Speaker embeddings from an ONNX model, via `ort`.
//!
//! Nothing here is new machinery. `ort` 2.0.0-rc.12 is already linked on every
//! target — `vad-rs` and `transcribe-rs`'s `onnx` feature both pin it, with
//! `download-binaries`, so the ONNX Runtime is a statically linked prebuilt — and
//! this is the same `Session::builder().commit_from_file()` path the Silero VAD
//! already takes, with two deliberate differences:
//!
//! * **More intra-op threads.** The VAD runs on a 30 ms frame in real time, where
//!   one thread avoids contention with the audio callbacks. This is an offline
//!   batch pass over a whole recording with nothing waiting on it, so it should use
//!   the machine.
//! * **Batched windows.** A per-window `run` spends most of its time in call
//!   overhead. Batching is also why the caller cuts fixed-length windows: unequal
//!   lengths cannot share a tensor.
//!
//! Inputs and outputs are addressed **positionally**, not by name. The WeSpeaker
//! export calls them `feats` and `embs`, but the k2-fsa re-exports carry added
//! metadata and other speaker models in the same family use different names — and
//! a hardcoded name is a latent break that only appears when someone swaps the
//! model. Every candidate has exactly one input and one output, so position is both
//! sufficient and stable.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use log::debug;
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Value;

use super::cluster::normalize;
use super::fbank::FbankComputer;

/// Upper bound on threads for the embedding session.
///
/// Half the cores, capped. Diarization runs right after a call ends, which is
/// exactly when the user is likely to still be doing something — and taking every
/// core to label a transcript they are not reading yet is the wrong trade.
const MAX_THREADS: usize = 4;

/// A loaded speaker-embedding model.
pub struct SpeakerEmbedder {
    session: Session,
    fbank: FbankComputer,
    /// Read from the graph rather than assumed: 256 for ResNet34, 192 for CAM++
    /// and 3D-Speaker. Taking it from the model means swapping the model needs no
    /// code change.
    embedding_dim: usize,
}

impl SpeakerEmbedder {
    /// Open the model.
    ///
    /// Fails loudly with the path on a corrupt file, so the caller can mark the
    /// catalog entry not-downloaded and let a truncated download self-heal rather
    /// than failing forever.
    pub fn new(model_path: &Path, fbank: FbankComputer) -> Result<Self> {
        let threads = (num_cpus_hint() / 2).clamp(1, MAX_THREADS);
        // `ort::Error` is neither `Send` nor `Sync`, so it cannot cross into
        // `anyhow` with `?`. Mapped to a message at every boundary instead, which
        // is also what `vad-rs` does with the same runtime.
        let session = Session::builder()
            .map_err(|e| anyhow!("Could not create an ONNX session builder: {e}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| anyhow!("Could not set the ONNX optimization level: {e}"))?
            .with_intra_threads(threads)
            .map_err(|e| anyhow!("Could not set ONNX thread count: {e}"))?
            .commit_from_file(model_path)
            .map_err(|e| anyhow!("Could not load the speaker model from {model_path:?}: {e}"))?;

        if session.inputs().len() != 1 || session.outputs().len() != 1 {
            return Err(anyhow!(
                "expected a speaker model with one input and one output, got {} and {}",
                session.inputs().len(),
                session.outputs().len()
            ));
        }

        // The embedding width is not knowable from the graph's declared output
        // shape when that dimension is symbolic, so it is discovered on the first
        // real inference instead and left at zero until then.
        let mut embedder = Self {
            session,
            fbank,
            embedding_dim: 0,
        };
        // A synthetic 1.5 s window: proves the model runs and settles the width
        // before any real audio depends on it. Cheap, and it turns "diarization
        // silently produced nothing" into a load-time error.
        let probe = vec![0.01f32; 24_000];
        let probed = embedder.embed_batch(&[probe])?;
        embedder.embedding_dim = probed.first().map_or(0, Vec::len);
        if embedder.embedding_dim == 0 {
            return Err(anyhow!(
                "the speaker model at {model_path:?} produced an empty embedding"
            ));
        }
        debug!(
            "Speaker embedder ready: {} dimensions, {threads} thread(s)",
            embedder.embedding_dim
        );
        Ok(embedder)
    }

    pub fn embedding_dim(&self) -> usize {
        self.embedding_dim
    }

    /// Embed a batch of **equal-length** windows, L2-normalised.
    ///
    /// Normalising here rather than at the call site so every consumer gets vectors
    /// on which cosine distance is `1 - dot` — the identity the clustering
    /// thresholds are expressed in.
    ///
    /// Windows of differing length are rejected rather than padded: padding a short
    /// window with silence changes its features, and a caller that produced unequal
    /// windows has a bug worth surfacing.
    pub fn embed_batch(&mut self, windows: &[Vec<f32>]) -> Result<Vec<Vec<f32>>> {
        if windows.is_empty() {
            return Ok(Vec::new());
        }

        // Features first, so a window that yields no frames is caught before a
        // tensor is built around it.
        let mut batch: Vec<Vec<Vec<f32>>> = Vec::with_capacity(windows.len());
        for window in windows {
            let features = self.fbank.compute(window);
            if features.is_empty() {
                return Err(anyhow!(
                    "a {} sample window is too short to produce features",
                    window.len()
                ));
            }
            batch.push(features);
        }

        let frames = batch[0].len();
        let bins = self.fbank.num_mel_bins();
        if batch.iter().any(|features| features.len() != frames) {
            return Err(anyhow!(
                "batched windows must be the same length; got {:?} frames",
                batch.iter().map(Vec::len).collect::<Vec<_>>()
            ));
        }

        // `[batch, frames, mel_bins]`, row-major, which is what the model declares.
        let mut flat = Vec::with_capacity(batch.len() * frames * bins);
        for features in &batch {
            for row in features {
                flat.extend_from_slice(row);
            }
        }

        let tensor = ndarray::Array3::from_shape_vec((batch.len(), frames, bins), flat)
            .context("shaping the speaker model input tensor")?;
        let value = Value::from_array(tensor)
            .map_err(|e| anyhow!("Could not build the speaker model input tensor: {e}"))?;

        // Positional, so no tensor name is hardcoded.
        let outputs = self
            .session
            .run(ort::inputs![value])
            .map_err(|e| anyhow!("The speaker embedding model failed to run: {e}"))?;
        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("Could not read the speaker embeddings: {e}"))?;

        // The output is `[batch, dim]`; deriving `dim` from the data length rather
        // than trusting the declared shape keeps this working when that dimension
        // is symbolic.
        let dim = data.len() / batch.len();
        if dim == 0 {
            return Err(anyhow!(
                "the speaker model returned {} values for {} window(s), shape {shape:?}",
                data.len(),
                batch.len()
            ));
        }

        Ok(data
            .chunks(dim)
            .take(batch.len())
            .map(|chunk| {
                let mut embedding = chunk.to_vec();
                normalize(&mut embedding);
                embedding
            })
            .collect())
    }
}

/// Available parallelism, or a conservative guess.
///
/// `std::thread::available_parallelism` rather than a crate; it answers `Err` in a
/// container with no CPU affinity, where two is a safer assumption than one (which
/// would serialise the pass) or sixteen (which would oversubscribe).
fn num_cpus_hint() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The thread budget must never be zero (which `with_intra_threads` rejects)
    /// and never take the whole machine while the user is still working.
    #[test]
    fn the_thread_budget_is_bounded() {
        let threads = (num_cpus_hint() / 2).clamp(1, MAX_THREADS);
        assert!(threads >= 1);
        assert!(threads <= MAX_THREADS);
    }

    #[test]
    fn the_cpu_hint_is_never_zero() {
        assert!(num_cpus_hint() >= 1);
    }
}
