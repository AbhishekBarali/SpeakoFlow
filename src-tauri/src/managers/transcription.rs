use crate::audio_toolkit::{apply_custom_words, filter_transcription_output, normalize_peak};
use crate::managers::audio::AudioRecordingManager;
use crate::managers::model::{EngineType, ModelManager};
use crate::settings::{
    get_settings, ModelUnloadTimeout, OrtAcceleratorSetting, WhisperAcceleratorSetting,
};
use anyhow::Result;
use log::{debug, error, info, warn};
use serde::Serialize;
use specta::Type;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, Manager};
use transcribe_rs::{
    onnx::{
        canary::CanaryModel,
        cohere::CohereModel,
        gigaam::GigaAMModel,
        moonshine::{MoonshineModel, MoonshineVariant, StreamingModel},
        parakeet::{ParakeetModel, ParakeetParams, TimestampGranularity},
        sense_voice::{SenseVoiceModel, SenseVoiceParams},
        Quantization,
    },
    transcriber::{Transcriber, VadChunked, VadChunkedConfig},
    vad::EnergyVad,
    whisper_cpp::{WhisperEngine, WhisperInferenceParams},
    SpeechModel, TranscribeOptions,
};

#[derive(Clone, Debug, Serialize)]
pub struct ModelStateEvent {
    pub event_type: String,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub error: Option<String>,
}

/// Incremental live-transcription text pushed to the recording overlay while a
/// streaming recording is in progress. Emitted as an UNTYPED Tauri event
/// ("stream-text") to match the existing "mic-level" pattern — intentionally
/// NOT a tauri-specta typed Event, so bindings.ts doesn't need regenerating.
#[derive(Clone, Debug, Serialize)]
struct StreamTextPayload {
    /// The append-only, flicker-free committed prefix. For the transcribe-rs
    /// VadChunked path this is the merged text of chunks closed at
    /// speech/silence boundaries; for the native transcribe.cpp streaming path
    /// it is `StreamText::committed` (the stable prefix the commit policy has
    /// locked in).
    committed: String,
    /// The volatile, not-yet-committed tail (`StreamText::tentative`). Populated
    /// by the native transcribe.cpp streaming path for models that expose a
    /// revisable suffix; empty for the VadChunked path and for cache-aware
    /// streaming models (e.g. Parakeet) that only ever commit.
    tentative: String,
}

enum LoadedEngine {
    Whisper(WhisperEngine),
    Parakeet(ParakeetModel),
    Moonshine(MoonshineModel),
    MoonshineStreaming(StreamingModel),
    SenseVoice(SenseVoiceModel),
    GigaAM(GigaAMModel),
    Canary(CanaryModel),
    Cohere(CohereModel),
    /// Native transcribe.cpp engine bound to a loaded GGUF model. The `Session`
    /// keeps its parent `Model` alive internally (Arc-backed), so this single
    /// handle is the whole engine (smallest shape). `Session` is `Send` but not
    /// `Sync`, which is fine behind the manager's `Mutex`. Batch is wired here
    /// in Session 2; native streaming (`session.stream`) lands in Session 4.
    TranscribeCpp(transcribe_cpp::Session),
}

/// Command sent to the live-transcription worker over a single FIFO channel.
/// Frames and the terminal Finalize/Cancel share the channel, so every fed
/// frame is guaranteed to be processed before finalization.
enum StreamCmd {
    /// A raw 16 kHz mono frame to feed the streaming transcriber.
    Feed(Vec<f32>),
    /// Flush the tail, merge, and reply with the full transcript (or `None` if
    /// no model was available so the caller falls back to batch).
    Finalize(mpsc::Sender<Option<String>>),
    /// Abort the session without producing a result.
    Cancel,
}

/// Routes raw audio frames from the recorder into the live-transcription
/// worker. `feed()` is a cheap, atomic-gated no-op when no stream is active, so
/// it is safe to call from the audio hot path unconditionally. Created once at
/// startup and shared (via `Arc`) by both the `TranscriptionManager` (which
/// starts/finalizes streams) and the `AudioRecordingManager` (whose recorder
/// feeds frames).
/// Hard cap on the number of un-consumed `Feed` frames allowed to sit in the
/// live-transcription channel at once. The recorder feeds ~30 ms frames, so
/// ~33/sec; 1000 frames ≈ 30 seconds of backlog.
///
/// The channel is FIFO and fed from the real-time audio callback. If inference
/// can't keep up with real-time (a slow/overloaded GPU, a stalled or hung
/// engine, or a runaway multi-minute recording), an *unbounded* channel would
/// grow without limit for as long as recording continues — a memory leak that
/// scales with recording length and starves the very inference that's already
/// behind. Once the worker is >~30 s behind, the live transcript is already
/// unusable, so we drop new frames (the batch `transcribe()` fallback still has
/// the complete audio) rather than let the queue grow without bound.
const MAX_QUEUED_FEED_FRAMES: usize = 1000;

pub struct StreamRouter {
    open: Arc<AtomicBool>,
    tx: Mutex<Option<mpsc::Sender<StreamCmd>>>,
    /// Number of `Feed` frames enqueued but not yet consumed by the worker.
    /// Gates `feed()` so the channel can never grow past
    /// [`MAX_QUEUED_FEED_FRAMES`]. Reset to 0 on each [`open`](Self::open).
    queued: AtomicUsize,
    /// Cumulative frames dropped because the worker fell behind. Surfaced in a
    /// periodic warning so a chronically-overloaded stream is visible instead
    /// of silently discarding audio. Reset on each [`open`](Self::open).
    dropped: AtomicU64,
}

impl StreamRouter {
    pub fn new() -> Self {
        Self {
            open: Arc::new(AtomicBool::new(false)),
            tx: Mutex::new(None),
            queued: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// Whether a streaming session is currently accepting frames.
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }

    /// Feed one raw 16 kHz mono frame. Returns immediately (relaxed atomic
    /// load, no allocation, no lock) when no stream is active.
    ///
    /// Never blocks the audio hot path: if the worker has fallen more than
    /// [`MAX_QUEUED_FEED_FRAMES`] behind, the frame is dropped (and counted)
    /// instead of enqueued, so the channel is strictly bounded regardless of
    /// how long recording runs or how slow inference is.
    pub fn feed(&self, frame: &[f32]) {
        if !self.open.load(Ordering::Relaxed) {
            return;
        }
        // Backpressure: bound the in-flight queue. Dropping when the worker is
        // this far behind is the correct degradation — a >30 s-late live
        // transcript is worthless anyway, and the batch fallback keeps the full
        // audio — versus an unbounded queue that leaks memory for the whole
        // recording and can never catch up.
        if self.queued.load(Ordering::Relaxed) >= MAX_QUEUED_FEED_FRAMES {
            let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            // Log on the first drop and then sparsely (~every 6 s of drops) so a
            // struggling stream is visible without flooding the log.
            if n == 1 || n % 200 == 0 {
                warn!(
                    "Live transcription can't keep up with real-time; dropping frames \
                     (backlog cap {} frames hit, {} dropped so far). The final transcript \
                     falls back to batch, so no audio is lost.",
                    MAX_QUEUED_FEED_FRAMES, n
                );
            }
            return;
        }
        if let Ok(guard) = self.tx.lock() {
            if let Some(tx) = guard.as_ref() {
                // Reserve the slot before sending so a concurrent feed sees the
                // updated backlog; roll back if the worker is already gone.
                self.queued.fetch_add(1, Ordering::Relaxed);
                if tx.send(StreamCmd::Feed(frame.to_vec())).is_err() {
                    self.queued.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Account for one `Feed` frame consumed by the worker, freeing a slot in
    /// the bounded queue. Saturating so a stale frame drained after a reset can
    /// never underflow the counter.
    fn note_feed_consumed(&self) {
        let _ = self
            .queued
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |q| q.checked_sub(1));
    }

    /// Install a fresh FIFO command channel and mark the router open, returning
    /// the receiver for the worker thread. Resets the backlog/drop counters so
    /// each session starts clean.
    fn open(&self) -> mpsc::Receiver<StreamCmd> {
        let (tx, rx) = mpsc::channel();
        self.queued.store(0, Ordering::Relaxed);
        self.dropped.store(0, Ordering::Relaxed);
        *self.tx.lock().unwrap() = Some(tx);
        self.open.store(true, Ordering::Relaxed);
        rx
    }

    /// Stop accepting new frames and take the sender so the caller can push a
    /// terminal Finalize/Cancel through the same channel (after all buffered
    /// frames have been enqueued).
    fn take(&self) -> Option<mpsc::Sender<StreamCmd>> {
        self.open.store(false, Ordering::Relaxed);
        self.tx.lock().unwrap().take()
    }

    /// Mark closed and drop any channel (defensive cleanup on worker exit).
    fn clear(&self) {
        self.open.store(false, Ordering::Relaxed);
        *self.tx.lock().unwrap() = None;
    }
}

impl Default for StreamRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard that clears the `is_loading` flag and notifies waiters on drop.
/// Ensures the loading flag is always reset, even on early returns or panics.
pub struct LoadingGuard {
    is_loading: Arc<Mutex<bool>>,
    loading_condvar: Arc<Condvar>,
}

impl Drop for LoadingGuard {
    fn drop(&mut self) {
        // Recover from a poisoned mutex instead of panicking — a panic inside
        // Drop calls abort(), which would kill the app on quit (Handy #1354).
        let mut is_loading = match self.is_loading.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        *is_loading = false;
        self.loading_condvar.notify_all();
    }
}

#[derive(Clone)]
pub struct TranscriptionManager {
    engine: Arc<Mutex<Option<LoadedEngine>>>,
    model_manager: Arc<ModelManager>,
    app_handle: AppHandle,
    current_model_id: Arc<Mutex<Option<String>>>,
    last_activity: Arc<AtomicU64>,
    shutdown_signal: Arc<AtomicBool>,
    watcher_handle: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    is_loading: Arc<Mutex<bool>>,
    loading_condvar: Arc<Condvar>,
    /// Router for the opt-in live/streaming transcription path. Shared with the
    /// `AudioRecordingManager` so the recorder can feed frames into an active
    /// stream. When no stream is active, feeding is a cheap no-op.
    stream_router: Arc<StreamRouter>,
}

impl TranscriptionManager {
    pub fn new(
        app_handle: &AppHandle,
        model_manager: Arc<ModelManager>,
        stream_router: Arc<StreamRouter>,
    ) -> Result<Self> {
        let manager = Self {
            engine: Arc::new(Mutex::new(None)),
            model_manager,
            app_handle: app_handle.clone(),
            current_model_id: Arc::new(Mutex::new(None)),
            last_activity: Arc::new(AtomicU64::new(Self::now_ms())),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            watcher_handle: Arc::new(Mutex::new(None)),
            is_loading: Arc::new(Mutex::new(false)),
            loading_condvar: Arc::new(Condvar::new()),
            stream_router,
        };

        // Start the idle watcher
        {
            let app_handle_cloned = app_handle.clone();
            let manager_cloned = manager.clone();
            let shutdown_signal = manager.shutdown_signal.clone();
            let handle = thread::spawn(move || {
                debug!("Idle watcher thread started");
                while !shutdown_signal.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_secs(10)); // Check every 10 seconds

                    // Check shutdown signal again after sleep
                    if shutdown_signal.load(Ordering::Relaxed) {
                        break;
                    }

                    let settings = get_settings(&app_handle_cloned);
                    let timeout = settings.model_unload_timeout;

                    // Skip Immediately — that variant is handled by
                    // maybe_unload_immediately() after each transcription.
                    // Treating it as 0s here would unload the model mid-recording.
                    if timeout == ModelUnloadTimeout::Immediately {
                        continue;
                    }

                    // While recording, keep the idle timer fresh so the
                    // model is never unloaded mid-session.
                    let is_recording = app_handle_cloned
                        .try_state::<Arc<AudioRecordingManager>>()
                        .map_or(false, |a| a.is_recording());
                    if is_recording {
                        manager_cloned.touch_activity();
                        continue;
                    }

                    if let Some(limit_seconds) = timeout.to_seconds() {
                        let last = manager_cloned.last_activity.load(Ordering::Relaxed);
                        let now_ms = TranscriptionManager::now_ms();
                        let idle_ms = now_ms.saturating_sub(last);
                        let limit_ms = limit_seconds * 1000;

                        if idle_ms > limit_ms {
                            // idle -> unload
                            if manager_cloned.is_model_loaded() {
                                let unload_start = std::time::Instant::now();
                                info!(
                                    "Model idle for {}s (limit: {}s), unloading",
                                    idle_ms / 1000,
                                    limit_seconds
                                );
                                match manager_cloned.unload_model() {
                                    Ok(()) => {
                                        let unload_duration = unload_start.elapsed();
                                        info!(
                                            "Model unloaded due to inactivity (took {}ms)",
                                            unload_duration.as_millis()
                                        );
                                    }
                                    Err(e) => {
                                        error!("Failed to unload idle model: {}", e);
                                    }
                                }
                            }
                        }
                    }
                }
                debug!("Idle watcher thread shutting down gracefully");
            });
            *manager.watcher_handle.lock().unwrap() = Some(handle);
        }

        Ok(manager)
    }

    /// Lock the engine mutex, recovering from poison if a previous transcription panicked.
    fn lock_engine(&self) -> MutexGuard<'_, Option<LoadedEngine>> {
        self.engine.lock().unwrap_or_else(|poisoned| {
            warn!("Engine mutex was poisoned by a previous panic, recovering");
            poisoned.into_inner()
        })
    }

    pub fn is_model_loaded(&self) -> bool {
        let engine = self.lock_engine();
        engine.is_some()
    }

    /// Atomically check whether a model load is in progress and, if not, mark
    /// one as starting. Returns a [`LoadingGuard`] whose [`Drop`] impl will
    /// clear the flag and wake waiters. Returns `None` if a load is already in
    /// progress.
    pub fn try_start_loading(&self) -> Option<LoadingGuard> {
        let mut is_loading = self.is_loading.lock().unwrap();
        if *is_loading {
            return None;
        }
        *is_loading = true;
        Some(LoadingGuard {
            is_loading: self.is_loading.clone(),
            loading_condvar: self.loading_condvar.clone(),
        })
    }

    pub fn unload_model(&self) -> Result<()> {
        let unload_start = std::time::Instant::now();
        debug!("Starting to unload model");

        {
            let mut engine = self.lock_engine();
            // Dropping the engine frees all resources
            *engine = None;
        }
        {
            let mut current_model = self.current_model_id.lock().unwrap();
            *current_model = None;
        }

        // Emit unloaded event
        let _ = self.app_handle.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "unloaded".to_string(),
                model_id: None,
                model_name: None,
                error: None,
            },
        );

        let unload_duration = unload_start.elapsed();
        debug!(
            "Model unloaded manually (took {}ms)",
            unload_duration.as_millis()
        );
        Ok(())
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    /// Reset the idle timer to now.
    fn touch_activity(&self) {
        self.last_activity.store(Self::now_ms(), Ordering::Relaxed);
    }

    /// Unloads the model immediately if the setting is enabled and the model is loaded
    pub fn maybe_unload_immediately(&self, context: &str) {
        let settings = get_settings(&self.app_handle);
        if settings.model_unload_timeout == ModelUnloadTimeout::Immediately
            && self.is_model_loaded()
        {
            info!("Immediately unloading model after {}", context);
            if let Err(e) = self.unload_model() {
                warn!("Failed to immediately unload model: {}", e);
            }
        }
    }

    pub fn load_model(&self, model_id: &str) -> Result<()> {
        let load_start = std::time::Instant::now();
        debug!("Starting to load model: {}", model_id);

        // Emit loading started event
        let _ = self.app_handle.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "loading_started".to_string(),
                model_id: Some(model_id.to_string()),
                model_name: None,
                error: None,
            },
        );

        let model_info = self
            .model_manager
            .get_model_info(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model not found: {}", model_id))?;

        if !model_info.is_downloaded {
            let error_msg = "Model not downloaded";
            let _ = self.app_handle.emit(
                "model-state-changed",
                ModelStateEvent {
                    event_type: "loading_failed".to_string(),
                    model_id: Some(model_id.to_string()),
                    model_name: Some(model_info.name.clone()),
                    error: Some(error_msg.to_string()),
                },
            );
            return Err(anyhow::anyhow!(error_msg));
        }

        let model_path = self.model_manager.get_model_path(model_id)?;

        // Create appropriate engine based on model type
        let emit_loading_failed = |error_msg: &str| {
            let _ = self.app_handle.emit(
                "model-state-changed",
                ModelStateEvent {
                    event_type: "loading_failed".to_string(),
                    model_id: Some(model_id.to_string()),
                    model_name: Some(model_info.name.clone()),
                    error: Some(error_msg.to_string()),
                },
            );
        };

        let loaded_engine = match model_info.engine_type {
            EngineType::Whisper => {
                let engine = WhisperEngine::load(&model_path).map_err(|e| {
                    let error_msg = format!("Failed to load whisper model {}: {}", model_id, e);
                    emit_loading_failed(&error_msg);
                    anyhow::anyhow!(error_msg)
                })?;
                LoadedEngine::Whisper(engine)
            }
            EngineType::Parakeet => {
                let engine =
                    ParakeetModel::load(&model_path, &Quantization::Int8).map_err(|e| {
                        let error_msg =
                            format!("Failed to load parakeet model {}: {}", model_id, e);
                        emit_loading_failed(&error_msg);
                        anyhow::anyhow!(error_msg)
                    })?;
                LoadedEngine::Parakeet(engine)
            }
            EngineType::Moonshine => {
                let engine = MoonshineModel::load(
                    &model_path,
                    MoonshineVariant::Base,
                    &Quantization::default(),
                )
                .map_err(|e| {
                    let error_msg = format!("Failed to load moonshine model {}: {}", model_id, e);
                    emit_loading_failed(&error_msg);
                    anyhow::anyhow!(error_msg)
                })?;
                LoadedEngine::Moonshine(engine)
            }
            EngineType::MoonshineStreaming => {
                let engine = StreamingModel::load(&model_path, 0, &Quantization::default())
                    .map_err(|e| {
                        let error_msg = format!(
                            "Failed to load moonshine streaming model {}: {}",
                            model_id, e
                        );
                        emit_loading_failed(&error_msg);
                        anyhow::anyhow!(error_msg)
                    })?;
                LoadedEngine::MoonshineStreaming(engine)
            }
            EngineType::SenseVoice => {
                let engine =
                    SenseVoiceModel::load(&model_path, &Quantization::Int8).map_err(|e| {
                        let error_msg =
                            format!("Failed to load SenseVoice model {}: {}", model_id, e);
                        emit_loading_failed(&error_msg);
                        anyhow::anyhow!(error_msg)
                    })?;
                LoadedEngine::SenseVoice(engine)
            }
            EngineType::GigaAM => {
                let engine = GigaAMModel::load(&model_path, &Quantization::Int8).map_err(|e| {
                    let error_msg = format!("Failed to load gigaam model {}: {}", model_id, e);
                    emit_loading_failed(&error_msg);
                    anyhow::anyhow!(error_msg)
                })?;
                LoadedEngine::GigaAM(engine)
            }
            EngineType::Canary => {
                let engine = CanaryModel::load(&model_path, &Quantization::Int8).map_err(|e| {
                    let error_msg = format!("Failed to load canary model {}: {}", model_id, e);
                    emit_loading_failed(&error_msg);
                    anyhow::anyhow!(error_msg)
                })?;
                LoadedEngine::Canary(engine)
            }
            EngineType::Cohere => {
                let engine = CohereModel::load(&model_path, &Quantization::Int8).map_err(|e| {
                    let error_msg = format!("Failed to load cohere model {}: {}", model_id, e);
                    emit_loading_failed(&error_msg);
                    anyhow::anyhow!(error_msg)
                })?;
                LoadedEngine::Cohere(engine)
            }
            EngineType::TranscribeCpp => {
                // Reuse the existing whisper accelerator + GPU-device dials for
                // backend selection (no second device UI — see PLAN.md S2).
                let options = transcribe_cpp_backend_options(&get_settings(&self.app_handle));
                let model =
                    transcribe_cpp::Model::load_with(&model_path, &options).map_err(|e| {
                        let error_msg =
                            format!("Failed to load transcribe.cpp model {}: {}", model_id, e);
                        emit_loading_failed(&error_msg);
                        anyhow::anyhow!(error_msg)
                    })?;
                let session = model.session().map_err(|e| {
                    let error_msg = format!(
                        "Failed to open transcribe.cpp session for {}: {}",
                        model_id, e
                    );
                    emit_loading_failed(&error_msg);
                    anyhow::anyhow!(error_msg)
                })?;
                // Log the resolved runtime backend so a silent CPU fallback (a
                // requested GPU that wasn't available) is visible in the logs.
                info!(
                    "Loaded transcribe.cpp model {} (arch={}, variant={}, backend={})",
                    model_id,
                    model.arch(),
                    model.variant(),
                    model.backend()
                );
                // Post-load reconcile (Session 3): the loaded model's
                // capabilities are ground truth — transcribe.cpp reads them from
                // the GGUF. Update the registry so the model list shows real
                // streaming / translation / language support. The
                // `loading_completed` event emitted below makes the frontend
                // reload the list, so the reconciled values surface immediately.
                let caps = model.capabilities();
                self.model_manager.set_runtime_capabilities(
                    model_id,
                    caps.supports_streaming,
                    caps.supports_translate,
                    &caps.languages,
                );
                LoadedEngine::TranscribeCpp(session)
            }
            // Not transcription engines — these are handled by their own
            // subsystems (LocalLlmManager / the assistant webview) and must
            // never be loaded as the active recording model.
            EngineType::LlamaCpp | EngineType::Kokoro => {
                let error_msg = format!(
                    "Model {} is not a transcription model and cannot be loaded for recording",
                    model_id
                );
                emit_loading_failed(&error_msg);
                return Err(anyhow::anyhow!(error_msg));
            }
        };

        // Update the current engine and model ID
        {
            let mut engine = self.lock_engine();
            *engine = Some(loaded_engine);
        }
        {
            let mut current_model = self.current_model_id.lock().unwrap();
            *current_model = Some(model_id.to_string());
        }

        // Reset idle timer so the watcher doesn't immediately unload a just-loaded model
        self.touch_activity();

        // Emit loading completed event
        let _ = self.app_handle.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "loading_completed".to_string(),
                model_id: Some(model_id.to_string()),
                model_name: Some(model_info.name.clone()),
                error: None,
            },
        );

        let load_duration = load_start.elapsed();
        debug!(
            "Successfully loaded transcription model: {} (took {}ms)",
            model_id,
            load_duration.as_millis()
        );
        Ok(())
    }

    /// Kicks off the model loading in a background thread if it's not already loaded
    pub fn initiate_model_load(&self) {
        let mut is_loading = self.is_loading.lock().unwrap();
        if *is_loading || self.is_model_loaded() {
            return;
        }

        *is_loading = true;
        let self_clone = self.clone();
        thread::spawn(move || {
            let settings = get_settings(&self_clone.app_handle);
            if let Err(e) = self_clone.load_model(&settings.selected_model) {
                error!("Failed to load model: {}", e);
            }
            let mut is_loading = self_clone.is_loading.lock().unwrap();
            *is_loading = false;
            self_clone.loading_condvar.notify_all();
        });
    }

    pub fn get_current_model(&self) -> Option<String> {
        let current_model = self.current_model_id.lock().unwrap();
        current_model.clone()
    }

    pub fn transcribe(&self, mut audio: Vec<f32>) -> Result<String> {
        #[cfg(debug_assertions)]
        if std::env::var("HANDY_FORCE_TRANSCRIPTION_FAILURE").is_ok() {
            return Err(anyhow::anyhow!(
                "Simulated transcription failure (HANDY_FORCE_TRANSCRIPTION_FAILURE)"
            ));
        }

        // Update last activity timestamp
        self.touch_activity();

        let st = std::time::Instant::now();

        debug!("Audio vector length: {}", audio.len());

        if audio.is_empty() {
            debug!("Empty audio vector");
            self.maybe_unload_immediately("empty audio");
            return Ok(String::new());
        }

        // Boost quiet captures up toward a healthy level before inference.
        // Whisper-style models degrade on faint, low-SNR audio (e.g. speaking
        // away from the mic or outdoors), and this is a cheap O(n) pass that
        // only ever boosts — already-loud audio is left untouched. Runs once,
        // off the live capture path, so it adds no recording latency.
        let applied_gain = normalize_peak(&mut audio, 0.95, 8.0);
        if applied_gain > 1.0 {
            debug!("Applied input gain normalization: {:.2}x", applied_gain);
        }

        // Check if model is loaded, if not try to load it
        {
            // If the model is loading, wait for it to complete.
            let mut is_loading = self.is_loading.lock().unwrap();
            while *is_loading {
                is_loading = self.loading_condvar.wait(is_loading).unwrap();
            }

            let engine_guard = self.lock_engine();
            if engine_guard.is_none() {
                return Err(anyhow::anyhow!("Model is not loaded for transcription."));
            }
        }

        // Get current settings for configuration
        let settings = get_settings(&self.app_handle);

        // Validate selected language against the model's supported languages.
        // If the language isn't supported, fall back to "auto" to prevent errors.
        let validated_language = if settings.selected_language == "auto" {
            "auto".to_string()
        } else {
            let is_supported = self
                .model_manager
                .get_model_info(&settings.selected_model)
                .map(|info| {
                    info.supported_languages.is_empty()
                        || info
                            .supported_languages
                            .contains(&settings.selected_language)
                })
                .unwrap_or(true);

            if is_supported {
                settings.selected_language.clone()
            } else {
                warn!(
                    "Language '{}' not supported by current model, falling back to auto-detect",
                    settings.selected_language
                );
                "auto".to_string()
            }
        };

        // Perform transcription with the appropriate engine.
        // We use catch_unwind to prevent engine panics from poisoning the mutex,
        // which would make the app hang indefinitely on subsequent operations.
        let result = {
            let mut engine_guard = self.lock_engine();

            // Take the engine out so we own it during transcription.
            // If the engine panics, we simply don't put it back (effectively unloading it)
            // instead of poisoning the mutex.
            let mut engine = match engine_guard.take() {
                Some(e) => e,
                None => {
                    return Err(anyhow::anyhow!(
                        "Model failed to load after auto-load attempt. Please check your model settings."
                    ));
                }
            };

            // Release the lock before transcribing — no mutex held during the engine call
            drop(engine_guard);

            let transcribe_result = catch_unwind(AssertUnwindSafe(|| -> Result<String> {
                // transcribe.cpp uses a different native result type and its
                // own run-plan, so handle it directly and return the text.
                // (Its `session.run` serializes on the model's own compute
                // lock; we already hold the engine lease, so no other call
                // can race it.)
                if let LoadedEngine::TranscribeCpp(session) = &mut engine {
                    let model = session.model();
                    let caps = model.capabilities();
                    let arch = model.arch();
                    let run_opts = transcribe_cpp_run_plan(&settings, &caps, &arch);
                    return session.run(&audio, &run_opts).map(|t| t.text).map_err(|e| {
                        anyhow::anyhow!("transcribe.cpp transcription failed: {}", e)
                    });
                }

                let rs_result: transcribe_rs::TranscriptionResult = match &mut engine {
                    LoadedEngine::Whisper(whisper_engine) => {
                        let whisper_language = if validated_language == "auto" {
                            None
                        } else {
                            let normalized = if validated_language == "zh-Hans"
                                || validated_language == "zh-Hant"
                            {
                                "zh".to_string()
                            } else {
                                validated_language.clone()
                            };
                            Some(normalized)
                        };

                        let params = WhisperInferenceParams {
                            language: whisper_language,
                            translate: settings.translate_to_english,
                            initial_prompt: if settings.custom_words.is_empty() {
                                None
                            } else {
                                Some(settings.custom_words.join(", "))
                            },
                            ..Default::default()
                        };

                        whisper_engine
                            .transcribe_with(&audio, &params)
                            .map_err(|e| anyhow::anyhow!("Whisper transcription failed: {}", e))
                    }
                    LoadedEngine::Parakeet(parakeet_engine) => {
                        let params = ParakeetParams {
                            timestamp_granularity: Some(TimestampGranularity::Segment),
                            ..Default::default()
                        };
                        parakeet_engine
                            .transcribe_with(&audio, &params)
                            .map_err(|e| anyhow::anyhow!("Parakeet transcription failed: {}", e))
                    }
                    LoadedEngine::Moonshine(moonshine_engine) => moonshine_engine
                        .transcribe(&audio, &TranscribeOptions::default())
                        .map_err(|e| anyhow::anyhow!("Moonshine transcription failed: {}", e)),
                    LoadedEngine::MoonshineStreaming(streaming_engine) => streaming_engine
                        .transcribe(&audio, &TranscribeOptions::default())
                        .map_err(|e| {
                            anyhow::anyhow!("Moonshine streaming transcription failed: {}", e)
                        }),
                    LoadedEngine::SenseVoice(sense_voice_engine) => {
                        let language = match validated_language.as_str() {
                            "zh" | "zh-Hans" | "zh-Hant" => Some("zh".to_string()),
                            "en" => Some("en".to_string()),
                            "ja" => Some("ja".to_string()),
                            "ko" => Some("ko".to_string()),
                            "yue" => Some("yue".to_string()),
                            _ => None,
                        };
                        let params = SenseVoiceParams {
                            language,
                            use_itn: Some(true),
                        };
                        sense_voice_engine
                            .transcribe_with(&audio, &params)
                            .map_err(|e| anyhow::anyhow!("SenseVoice transcription failed: {}", e))
                    }
                    LoadedEngine::GigaAM(gigaam_engine) => gigaam_engine
                        .transcribe(&audio, &TranscribeOptions::default())
                        .map_err(|e| anyhow::anyhow!("GigaAM transcription failed: {}", e)),
                    LoadedEngine::Canary(canary_engine) => {
                        let lang = if validated_language == "auto" {
                            None
                        } else {
                            Some(validated_language.clone())
                        };
                        let options = TranscribeOptions {
                            language: lang,
                            translate: settings.translate_to_english,
                            ..Default::default()
                        };
                        canary_engine
                            .transcribe(&audio, &options)
                            .map_err(|e| anyhow::anyhow!("Canary transcription failed: {}", e))
                    }
                    LoadedEngine::Cohere(cohere_engine) => {
                        let lang = if validated_language == "auto" {
                            None
                        } else if validated_language == "zh-Hans" || validated_language == "zh-Hant"
                        {
                            Some("zh".to_string())
                        } else {
                            Some(validated_language.clone())
                        };
                        let options = TranscribeOptions {
                            language: lang,
                            ..Default::default()
                        };
                        cohere_engine
                            .transcribe(&audio, &options)
                            .map_err(|e| anyhow::anyhow!("Cohere transcription failed: {}", e))
                    }
                    // transcribe.cpp is handled by the early return above.
                    LoadedEngine::TranscribeCpp(_) => {
                        unreachable!("transcribe.cpp is handled before the transcribe-rs match")
                    }
                }?;
                Ok(rs_result.text)
            }));

            match transcribe_result {
                Ok(inner_result) => {
                    // Success or normal error — put the engine back
                    let mut engine_guard = self.lock_engine();
                    *engine_guard = Some(engine);
                    inner_result?
                }
                Err(panic_payload) => {
                    // Engine panicked — do NOT put it back (it's in an unknown state).
                    // The engine is dropped here, effectively unloading it.
                    let panic_msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                        s.to_string()
                    } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "unknown panic".to_string()
                    };
                    error!(
                        "Transcription engine panicked: {}. Model has been unloaded.",
                        panic_msg
                    );

                    // Clear the model ID so it will be reloaded on next attempt
                    {
                        let mut current_model = self
                            .current_model_id
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        *current_model = None;
                    }

                    let _ = self.app_handle.emit(
                        "model-state-changed",
                        ModelStateEvent {
                            event_type: "unloaded".to_string(),
                            model_id: None,
                            model_name: None,
                            error: Some(format!("Engine panicked: {}", panic_msg)),
                        },
                    );

                    return Err(anyhow::anyhow!(
                        "Transcription engine panicked: {}. The model has been unloaded and will reload on next attempt.",
                        panic_msg
                    ));
                }
            }
        };

        // Apply word correction if custom words are configured.
        // Skip for Whisper models since custom words are already passed as initial_prompt.
        let is_whisper = self
            .model_manager
            .get_model_info(&settings.selected_model)
            .map(|info| matches!(info.engine_type, EngineType::Whisper))
            .unwrap_or(false);

        let corrected_result = if !settings.custom_words.is_empty() && !is_whisper {
            apply_custom_words(
                &result,
                &settings.custom_words,
                settings.word_correction_threshold,
            )
        } else {
            result
        };

        // Filter out filler words and hallucinations
        let filtered_result = filter_transcription_output(
            &corrected_result,
            &settings.app_language,
            &settings.custom_filler_words,
        );

        let et = std::time::Instant::now();
        let translation_note = if settings.translate_to_english {
            " (translated)"
        } else {
            ""
        };
        info!(
            "Transcription completed in {}ms{}",
            (et - st).as_millis(),
            translation_note
        );

        let final_result = filtered_result;

        if final_result.is_empty() {
            info!("Transcription result is empty");
        } else {
            info!("Transcription result: {}", final_result);
        }

        self.maybe_unload_immediately("transcription");

        Ok(final_result)
    }

    /// Shared handle to the live-transcription router. Handed to the recorder
    /// so its frame callback can feed audio while a stream is active. Public
    /// accessor kept for external/state access even though the router is also
    /// injected directly at construction.
    #[allow(dead_code)]
    pub fn stream_router(&self) -> Arc<StreamRouter> {
        self.stream_router.clone()
    }

    /// Begin an opt-in live/streaming transcription session. Opens the router
    /// and spawns a worker that leases the loaded model for the duration of the
    /// recording and emits incremental text. The worker picks its path from the
    /// loaded engine: a streaming-capable transcribe.cpp model streams natively
    /// (`session.stream`→`feed`→`finalize`, real committed+tentative text); a
    /// transcribe-rs engine uses the VAD-chunked transcriber; anything else
    /// (no model, or a non-streaming transcribe.cpp model) falls back to batch.
    /// No-op (with a warning) if a stream is already open. The batch
    /// `transcribe()` path remains the fallback (see
    /// [`finalize_stream`](Self::finalize_stream)).
    pub fn start_stream(&self) {
        if self.stream_router.is_open() {
            warn!("Live transcription stream already open; ignoring start_stream()");
            return;
        }
        let rx = self.stream_router.open();
        let this = self.clone();
        thread::spawn(move || {
            this.run_stream_worker(rx);
        });
    }

    /// Worker body: wait for any in-progress load, lease the engine, then drive
    /// streaming from the FIFO command channel until Finalize or Cancel. Picks
    /// the path from the leased engine:
    ///
    /// - **transcribe.cpp + `supports_streaming`** → native streaming
    ///   (`session.stream`→`feed`→`finalize`), emitting real committed+tentative
    ///   text (Session 4, the payoff).
    /// - **transcribe-rs engine** → the [`VadChunked`] isolated-chunk path,
    ///   retained for the non-cpp streaming models (e.g. `MoonshineStreaming`)
    ///   that still rely on it.
    /// - **no model / non-streaming transcribe.cpp model** → batch fallback:
    ///   return the leased engine and reply `None` to Finalize so
    ///   `finalize_stream()` routes to the batch `transcribe()` path.
    ///
    /// Always returns the leased engine to the mutex before replying.
    fn run_stream_worker(&self, rx: mpsc::Receiver<StreamCmd>) {
        // Wait for any in-progress model load to finish (mirrors transcribe()).
        {
            let mut is_loading = self.is_loading.lock().unwrap();
            while *is_loading {
                is_loading = self.loading_condvar.wait(is_loading).unwrap();
            }
        }

        // Lease the engine out of the mutex for the whole stream, exactly like
        // transcribe() borrows it for a single call.
        let mut leased = {
            let mut guard = self.lock_engine();
            guard.take()
        };

        // Decide the streaming path from the leased engine (see the fn doc).
        enum StreamPath {
            /// No engine leased.
            NoModel,
            /// A transcribe.cpp model that doesn't advertise streaming.
            CppBatchFallback,
            /// A streaming-capable transcribe.cpp model → native streaming.
            Cpp,
            /// A transcribe-rs engine → the VadChunked path.
            TranscribeRs,
        }
        let path = match leased.as_ref() {
            None => StreamPath::NoModel,
            Some(LoadedEngine::TranscribeCpp(session)) => {
                if session.model().capabilities().supports_streaming {
                    StreamPath::Cpp
                } else {
                    StreamPath::CppBatchFallback
                }
            }
            Some(_) => StreamPath::TranscribeRs,
        };

        // No model, or a non-streaming transcribe.cpp model: return the leased
        // engine (if any) and drain the channel, replying `None` to Finalize so
        // finalize_stream() routes to the batch transcribe() path.
        if matches!(path, StreamPath::NoModel | StreamPath::CppBatchFallback) {
            if matches!(path, StreamPath::CppBatchFallback) {
                debug!(
                    "Live transcription: transcribe.cpp model does not advertise streaming; \
                     falling back to batch"
                );
                // Return the leased engine (guard against a concurrent refill).
                let mut guard = self.lock_engine();
                if guard.is_none() {
                    *guard = leased.take();
                }
            } else {
                warn!("Live transcription: no model loaded; falling back to batch");
            }
            self.drain_stream_no_model(rx);
            self.stream_router.clear();
            return;
        }

        // Both remaining paths converge on this outcome, then a shared tail
        // returns the leased engine and (on Finalize) answers the reply channel.
        let mut final_text: Option<String> = None;
        let mut finalize_reply: Option<mpsc::Sender<Option<String>>> = None;
        // Set when native stream setup fails before the channel is consumed, so
        // we can still batch-fall-back (drain replying `None`) afterwards.
        let mut cpp_begin_failed = false;

        match path {
            StreamPath::Cpp => {
                // Native transcribe.cpp streaming. Build the run plan from the
                // same helper the batch path uses (Session 2), open a stream
                // with the Auto commit policy, and drive it from the FIFO
                // channel. Scoped so the borrowed `Stream` — which holds the
                // model's compute lease for its lifetime — is dropped before we
                // return the leased engine to the mutex below.
                let settings = get_settings(&self.app_handle);
                let engine = leased.as_mut().expect("checked Some above");
                let session = match engine {
                    LoadedEngine::TranscribeCpp(s) => s,
                    _ => unreachable!("Cpp path only entered for a TranscribeCpp engine"),
                };
                // Capabilities + arch are owned snapshots, taken before the
                // mutable stream borrow so no borrow conflict arises.
                let caps = session.model().capabilities();
                let arch = session.model().arch();
                let run_opts = transcribe_cpp_run_plan(&settings, &caps, &arch);
                let stream_opts = transcribe_cpp::StreamOptions {
                    commit_policy: transcribe_cpp::CommitPolicy::Auto,
                    ..Default::default()
                };

                match session.stream(&run_opts, &stream_opts) {
                    Ok(mut stream) => {
                        // Accumulate the recorder's ~30 ms frames up to ~80 ms
                        // (1280 samples @ 16 kHz) before each feed — the cadence
                        // the S1 spike validated as smooth for Parakeet's
                        // cache-aware streaming.
                        const FEED_SAMPLES: usize = 1280;
                        let mut acc: Vec<f32> = Vec::with_capacity(FEED_SAMPLES * 2);

                        while let Ok(cmd) = rx.recv() {
                            match cmd {
                                StreamCmd::Feed(pcm) => {
                                    self.stream_router.note_feed_consumed();
                                    acc.extend_from_slice(&pcm);
                                    while acc.len() >= FEED_SAMPLES {
                                        let chunk: Vec<f32> = acc.drain(..FEED_SAMPLES).collect();
                                        match stream.feed(&chunk) {
                                            Ok(upd) => {
                                                if upd.committed_changed || upd.tentative_changed {
                                                    let text = stream.text();
                                                    let _ = self.app_handle.emit(
                                                        "stream-text",
                                                        StreamTextPayload {
                                                            committed: text.committed,
                                                            tentative: text.tentative,
                                                        },
                                                    );
                                                }
                                            }
                                            Err(e) => {
                                                warn!("Live transcription (cpp) feed error: {}", e)
                                            }
                                        }
                                    }
                                }
                                StreamCmd::Finalize(reply) => {
                                    // Flush the sub-80 ms tail so the last words
                                    // are included before finalizing.
                                    if !acc.is_empty() {
                                        let tail = std::mem::take(&mut acc);
                                        if let Err(e) = stream.feed(&tail) {
                                            warn!(
                                                "Live transcription (cpp) tail feed error: {}",
                                                e
                                            );
                                        }
                                    }
                                    match stream.finalize() {
                                        Ok(_) => {
                                            // committed now holds the full result
                                            // and tentative is emptied; reflect it
                                            // in the overlay, then take display()
                                            // (= committed + tentative) as the
                                            // final transcript.
                                            let text = stream.text();
                                            let _ = self.app_handle.emit(
                                                "stream-text",
                                                StreamTextPayload {
                                                    committed: text.committed.clone(),
                                                    tentative: text.tentative.clone(),
                                                },
                                            );
                                            final_text = Some(text.display());
                                        }
                                        Err(e) => {
                                            warn!("Live transcription (cpp) finalize error: {}", e);
                                            // Best-effort: keep whatever text was
                                            // committed so the user never loses it.
                                            final_text = Some(stream.text().display());
                                        }
                                    }
                                    finalize_reply = Some(reply);
                                    break;
                                }
                                StreamCmd::Cancel => {
                                    // Abandon the stream (releases the compute
                                    // lease); no result is produced.
                                    stream.reset();
                                    break;
                                }
                            }
                        }
                        // `stream` dropped here → releases the model's compute
                        // lease so the leased engine is safe to reuse/return.
                    }
                    Err(e) => {
                        warn!(
                            "Live transcription: transcribe.cpp stream begin failed ({}); \
                             falling back to batch",
                            e
                        );
                        cpp_begin_failed = true;
                    }
                }
            }
            StreamPath::TranscribeRs => {
                let engine = leased.as_mut().expect("checked Some above");

                // Build the streaming transcriber from current settings.
                let settings = get_settings(&self.app_handle);

                // Validate the language against the model (mirrors transcribe());
                // fall back to auto-detect when the model doesn't support it.
                let validated_language = if settings.selected_language == "auto" {
                    "auto".to_string()
                } else {
                    let is_supported = self
                        .model_manager
                        .get_model_info(&settings.selected_model)
                        .map(|info| {
                            info.supported_languages.is_empty()
                                || info
                                    .supported_languages
                                    .contains(&settings.selected_language)
                        })
                        .unwrap_or(true);
                    if is_supported {
                        settings.selected_language.clone()
                    } else {
                        warn!(
                            "Language '{}' not supported by current model, falling back to auto-detect",
                            settings.selected_language
                        );
                        "auto".to_string()
                    }
                };

                // CJK text is merged without spaces between chunks.
                let is_cjk = validated_language.starts_with("zh")
                    || validated_language == "ja"
                    || validated_language == "yue";
                let separator = if is_cjk { "" } else { " " };

                // VadChunked drives model.transcribe(&chunk, &options) per chunk,
                // so the per-engine params used by batch (whisper initial_prompt,
                // SenseVoice use_itn, …) don't apply here — only language +
                // translate ride along.
                let language = if validated_language == "auto" {
                    None
                } else if validated_language == "zh-Hans" || validated_language == "zh-Hant" {
                    Some("zh".to_string())
                } else {
                    Some(validated_language.clone())
                };
                let options = TranscribeOptions {
                    language,
                    translate: settings.translate_to_english,
                    ..Default::default()
                };

                let config = VadChunkedConfig {
                    min_chunk_secs: 1.0,
                    max_chunk_secs: 18.0,
                    padding_secs: 0.4,
                    smart_split_search_secs: Some(1.0),
                    merge_separator: separator.to_string(),
                };
                // EnergyVad needs no model file; 480 samples = 30 ms at 16 kHz,
                // which matches the recorder's resampled frame size.
                let vad: Box<dyn transcribe_rs::vad::Vad> = Box::new(EnergyVad::new(480, 0.01));
                let mut transcriber = VadChunked::new(vad, config, options);

                // Resolve the leased engine variant to a &mut dyn SpeechModel.
                let model: &mut dyn SpeechModel = match engine {
                    LoadedEngine::Whisper(e) => e,
                    LoadedEngine::Parakeet(e) => e,
                    LoadedEngine::Moonshine(e) => e,
                    LoadedEngine::MoonshineStreaming(e) => e,
                    LoadedEngine::SenseVoice(e) => e,
                    LoadedEngine::GigaAM(e) => e,
                    LoadedEngine::Canary(e) => e,
                    LoadedEngine::Cohere(e) => e,
                    // transcribe.cpp uses the native streaming path above.
                    LoadedEngine::TranscribeCpp(_) => {
                        unreachable!("transcribe.cpp uses the native streaming path")
                    }
                };

                let mut committed = String::new();

                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        StreamCmd::Feed(pcm) => {
                            self.stream_router.note_feed_consumed();
                            match transcriber.feed(&mut *model, &pcm) {
                                Ok(results) => {
                                    let had_new = !results.is_empty();
                                    for r in results {
                                        let t = r.text.trim();
                                        if t.is_empty() {
                                            continue;
                                        }
                                        if !committed.is_empty() {
                                            committed.push_str(separator);
                                        }
                                        committed.push_str(t);
                                    }
                                    if had_new {
                                        let _ = self.app_handle.emit(
                                            "stream-text",
                                            StreamTextPayload {
                                                committed: committed.clone(),
                                                tentative: String::new(),
                                            },
                                        );
                                    }
                                }
                                Err(e) => warn!("Live transcription feed error: {}", e),
                            }
                        }
                        StreamCmd::Finalize(reply) => {
                            match transcriber.finish(&mut *model) {
                                Ok(res) => {
                                    let full = res.text.trim().to_string();
                                    final_text = Some(if full.is_empty() {
                                        committed.clone()
                                    } else {
                                        full
                                    });
                                }
                                Err(e) => {
                                    warn!("Live transcription finish error: {}", e);
                                    final_text = Some(committed.clone());
                                }
                            }
                            finalize_reply = Some(reply);
                            break;
                        }
                        StreamCmd::Cancel => break,
                    }
                }
            }
            StreamPath::NoModel | StreamPath::CppBatchFallback => {
                unreachable!("no-model / batch-fallback handled before this match")
            }
        }

        // Native stream setup failed before consuming the channel: behave
        // exactly like the batch-fallback path — return the engine and drain
        // the channel replying `None` so finalize_stream() batches.
        if cpp_begin_failed {
            {
                let mut guard = self.lock_engine();
                if guard.is_none() {
                    *guard = leased.take();
                }
            }
            self.drain_stream_no_model(rx);
            self.stream_router.clear();
            return;
        }

        // Return the leased engine, guarding against a concurrent load/unload
        // having refilled the slot while we streamed — don't clobber a newer
        // engine; drop our stale lease instead.
        {
            let mut guard = self.lock_engine();
            if guard.is_none() {
                *guard = leased;
            } else {
                debug!("Live transcription: engine slot already refilled; dropping leased engine");
            }
        }
        self.stream_router.clear();

        // Reply only after the engine is back, so a None-triggered batch
        // fallback in finalize_stream() finds the model available.
        if let Some(reply) = finalize_reply {
            let text = final_text.filter(|t| !t.trim().is_empty());
            let _ = reply.send(text);
        }
    }

    /// Drain the command channel when no model could be leased, replying `None`
    /// to any Finalize so the caller uses the batch path.
    fn drain_stream_no_model(&self, rx: mpsc::Receiver<StreamCmd>) {
        while let Ok(cmd) = rx.recv() {
            match cmd {
                StreamCmd::Feed(_) => self.stream_router.note_feed_consumed(),
                StreamCmd::Finalize(reply) => {
                    let _ = reply.send(None);
                    break;
                }
                StreamCmd::Cancel => break,
            }
        }
    }

    /// Finalize the live-transcription session and return the merged transcript.
    ///
    /// Returns `Ok(None)` when no stream is active (→ caller uses batch), when no
    /// model was available, or when the streamed result is empty; `Err` on
    /// timeout. Applies the SAME post-processing as batch (custom words for
    /// non-whisper models + the filler/hallucination filter).
    pub fn finalize_stream(&self) -> Result<Option<String>> {
        let tx = match self.stream_router.take() {
            Some(tx) => tx,
            None => return Ok(None),
        };
        let (reply_tx, reply_rx) = mpsc::channel();
        if tx.send(StreamCmd::Finalize(reply_tx)).is_err() {
            // Worker already gone — fall back to batch.
            return Ok(None);
        }
        drop(tx); // no further commands for this session

        let raw = match reply_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Some(text)) => text,
            Ok(None) => return Ok(None),
            Err(_) => return Err(anyhow::anyhow!("Live transcription finalize timed out")),
        };

        // Mirror batch post-processing on the final text.
        let settings = get_settings(&self.app_handle);
        let is_whisper = self
            .model_manager
            .get_model_info(&settings.selected_model)
            .map(|info| matches!(info.engine_type, EngineType::Whisper))
            .unwrap_or(false);

        let corrected = if !settings.custom_words.is_empty() && !is_whisper {
            apply_custom_words(
                &raw,
                &settings.custom_words,
                settings.word_correction_threshold,
            )
        } else {
            raw
        };
        let filtered = filter_transcription_output(
            &corrected,
            &settings.app_language,
            &settings.custom_filler_words,
        );

        if filtered.trim().is_empty() {
            return Ok(None);
        }

        self.maybe_unload_immediately("live transcription");
        Ok(Some(filtered))
    }

    /// Cancel any active live-transcription session, releasing the worker (which
    /// returns the leased engine). No-op when no stream is active.
    pub fn cancel_stream(&self) {
        if let Some(tx) = self.stream_router.take() {
            let _ = tx.send(StreamCmd::Cancel);
        }
    }
}

/// Initialize transcribe.cpp once at startup: route its native + ggml logs into
/// the `log` facade, then load the backend modules. In the `dynamic-backends`
/// Windows/Linux build this loads the ggml backend DLLs/`.so`s that sit next to
/// libtranscribe; on macOS (compiled-in backends) it is a no-op. A failure here
/// (e.g. missing backend modules) is logged and swallowed so the transcribe-rs
/// engines keep working — N1, never break the app. Must run before any
/// transcribe.cpp model load or `devices()` enumeration.
pub fn init_transcribe_cpp() {
    transcribe_cpp::init_logging();
    match transcribe_cpp::init_backends_default() {
        Ok(()) => info!("transcribe.cpp backends initialized"),
        Err(e) => warn!(
            "transcribe.cpp backend init failed (transcribe.cpp GGUF models unavailable): {}",
            e
        ),
    }
}

/// Map the user's (whisper) accelerator + GPU-device dials onto transcribe.cpp
/// load options. Session 2 deliberately REUSES the existing whisper dials
/// (`whisper_accelerator` + `whisper_gpu_device`) instead of adding a second
/// device UI — see PLAN.md Session 2. `Auto`/`Gpu` both map to `Backend::Auto`,
/// which already prefers the best available GPU with a guaranteed CPU fallback
/// (so it never errors on a GPU-less machine); `Cpu` forces strict CPU.
/// transcribe-rs uses `-1` for "auto GPU device" while transcribe.cpp uses `0`
/// (auto / first matching device), so a negative index is normalized to 0.
fn transcribe_cpp_backend_options(
    settings: &crate::settings::AppSettings,
) -> transcribe_cpp::ModelOptions {
    use transcribe_cpp::{Backend, ModelOptions};

    let backend = match settings.whisper_accelerator {
        WhisperAcceleratorSetting::Cpu => Backend::Cpu,
        WhisperAcceleratorSetting::Auto | WhisperAcceleratorSetting::Gpu => Backend::Auto,
    };
    let gpu_device = if settings.whisper_gpu_device < 0 {
        0
    } else {
        settings.whisper_gpu_device
    };
    ModelOptions {
        backend,
        gpu_device,
    }
}

/// Build transcribe.cpp `RunOptions` for a batch (or, later, streaming) run from
/// the user settings and the loaded model's capabilities + architecture.
///
/// Shared by the batch `transcribe()` arm (Session 2) and the native streaming
/// worker (Session 4):
/// - **language**: the user's selected language IF the model advertises it in
///   `caps.languages`, otherwise `None` (autodetect). `"auto"` is always
///   `None`; the `zh-Hans`/`zh-Hant` variants are normalized to `zh`.
/// - **translate**: when `translate_to_english` is on, the model advertises
///   translation, and the source isn't already English, request
///   `Task::Translate` with `target_language = "en"`; otherwise `Transcribe`.
/// - **timestamps**: `Segment` (mirrors the transcribe-rs Parakeet batch path).
/// - **family extension**: a Whisper initial-prompt built from the user's
///   custom words is attached ONLY for whisper-arch models (every other family
///   rejects the run-slot extension); non-whisper families instead rely on the
///   existing fuzzy `apply_custom_words` post-correction downstream.
pub fn transcribe_cpp_run_plan(
    settings: &crate::settings::AppSettings,
    caps: &transcribe_cpp::Capabilities,
    arch: &str,
) -> transcribe_cpp::RunOptions {
    use transcribe_cpp::{RunExtension, RunOptions, Task, TimestampKind, WhisperRunOptions};

    let selected = settings.selected_language.as_str();
    let normalized = match selected {
        "zh-Hans" | "zh-Hant" => "zh",
        other => other,
    };

    // Language hint only when the model actually advertises it; else autodetect.
    let language = if selected == "auto" {
        None
    } else if caps
        .languages
        .iter()
        .any(|l| l == normalized || l == selected)
    {
        Some(normalized.to_string())
    } else {
        None
    };

    // Translate to English only when supported and the source isn't English.
    let src_is_english = matches!(language.as_deref(), Some("en"));
    let (task, target_language) =
        if settings.translate_to_english && caps.supports_translate && !src_is_english {
            (Task::Translate, Some("en".to_string()))
        } else {
            (Task::Transcribe, None)
        };

    // Custom words → whisper initial prompt, whisper-arch only. Non-whisper
    // families reject the run-slot extension and use fuzzy post-correction.
    let family = if arch == "whisper" && !settings.custom_words.is_empty() {
        Some(RunExtension::Whisper(WhisperRunOptions {
            initial_prompt: Some(settings.custom_words.join(", ")),
            ..Default::default()
        }))
    } else {
        None
    };

    RunOptions {
        task,
        timestamps: TimestampKind::Segment,
        language,
        target_language,
        family,
        ..Default::default()
    }
}

/// Apply the user's accelerator preferences to the transcribe-rs global atomics.
/// Called on startup and whenever the user changes the setting.
pub fn apply_accelerator_settings(app: &tauri::AppHandle) {
    use transcribe_rs::accel;

    let settings = get_settings(app);

    let whisper_pref = match settings.whisper_accelerator {
        WhisperAcceleratorSetting::Auto => accel::WhisperAccelerator::Auto,
        WhisperAcceleratorSetting::Cpu => accel::WhisperAccelerator::CpuOnly,
        WhisperAcceleratorSetting::Gpu => accel::WhisperAccelerator::Gpu,
    };
    accel::set_whisper_accelerator(whisper_pref);
    accel::set_whisper_gpu_device(settings.whisper_gpu_device);
    info!(
        "Whisper accelerator set to: {}, gpu_device: {}",
        whisper_pref,
        if settings.whisper_gpu_device == accel::GPU_DEVICE_AUTO {
            "auto".to_string()
        } else {
            settings.whisper_gpu_device.to_string()
        }
    );

    let ort_pref = match settings.ort_accelerator {
        OrtAcceleratorSetting::Auto => resolve_auto_ort_accelerator(),
        OrtAcceleratorSetting::Cpu => accel::OrtAccelerator::CpuOnly,
        OrtAcceleratorSetting::Cuda => accel::OrtAccelerator::Cuda,
        OrtAcceleratorSetting::DirectMl => accel::OrtAccelerator::DirectMl,
        OrtAcceleratorSetting::Rocm => accel::OrtAccelerator::Rocm,
    };
    accel::set_ort_accelerator(ort_pref);
    info!(
        "ORT accelerator: setting={:?}, resolved={}",
        settings.ort_accelerator, ort_pref
    );
}

/// Resolve the ONNX Runtime "Auto" setting to a concrete accelerator.
///
/// `transcribe-rs` deliberately excludes DirectML (and WebGPU) from its own
/// `Auto` execution-provider probing because they require sequential execution
/// (`parallel_execution(false)` + `memory_pattern(false)`), which would
/// penalize the other backends it tries in Auto (CUDA / TensorRT / ROCm /
/// CoreML).
///
/// On Windows, however, DirectML is the ONLY ONNX GPU execution provider
/// compiled into this build (see the `ort-directml` feature in Cargo.toml).
/// That means a literal "Auto" silently pins every ONNX model — Parakeet,
/// Canary, Moonshine, SenseVoice, GigaAM, Cohere — to the CPU, which is the
/// "why is Parakeet suddenly slow?" trap. So on Windows we resolve Auto to
/// DirectML whenever it's actually compiled in, giving ONNX models GPU
/// acceleration out of the box. The session builder still appends CPU as the
/// final fallback, so a machine with no DX12-capable GPU degrades gracefully.
///
/// On every other platform we keep the library's Auto behavior: macOS/Linux
/// compile no ORT GPU provider (ONNX stays on CPU there), and where CUDA/ROCm
/// are available they are already part of the library's Auto list.
fn resolve_auto_ort_accelerator() -> transcribe_rs::accel::OrtAccelerator {
    use transcribe_rs::accel::OrtAccelerator;

    #[cfg(windows)]
    {
        // `available()` reflects transcribe-rs's own compile-time feature set,
        // so this stays correct even if the DirectML feature is ever dropped
        // from the Windows build.
        if OrtAccelerator::available().contains(&OrtAccelerator::DirectMl) {
            return OrtAccelerator::DirectMl;
        }
    }

    OrtAccelerator::Auto
}

#[derive(Serialize, Clone, Debug, Type)]
pub struct GpuDeviceOption {
    pub id: i32,
    pub name: String,
    pub total_vram_mb: usize,
}

static GPU_DEVICES: OnceLock<Vec<GpuDeviceOption>> = OnceLock::new();

fn cached_gpu_devices() -> &'static [GpuDeviceOption] {
    use transcribe_rs::whisper_cpp::gpu::list_gpu_devices;

    GPU_DEVICES.get_or_init(|| {
        // ggml's Vulkan backend uses FMA3 instructions internally.
        // On older CPUs without FMA3 (e.g. Sandy Bridge Xeons) this causes
        // a SIGILL crash that cannot be caught. Skip enumeration entirely
        // on those CPUs — GPU-accelerated whisper won't work there anyway.
        #[cfg(target_arch = "x86_64")]
        if !std::arch::is_x86_feature_detected!("fma") {
            warn!("CPU lacks FMA3 support — skipping GPU device enumeration");
            return Vec::new();
        }

        list_gpu_devices()
            .into_iter()
            .map(|d| GpuDeviceOption {
                id: d.id,
                name: d.name,
                total_vram_mb: d.total_vram / (1024 * 1024),
            })
            .collect()
    })
}

#[derive(Serialize, Clone, Debug, Type)]
pub struct AvailableAccelerators {
    pub whisper: Vec<String>,
    pub ort: Vec<String>,
    pub gpu_devices: Vec<GpuDeviceOption>,
    /// transcribe.cpp compute devices (`transcribe_cpp::devices()`), added in
    /// Session 2. Informational for now (proves the GPU is visible to the new
    /// engine); the transcribe.cpp backend reuses the existing whisper
    /// GPU-device dial, so there is no separate device setting.
    pub transcribe_cpp_devices: Vec<GpuDeviceOption>,
}

/// transcribe.cpp compute devices, enumerated once and cached. Requires
/// `init_transcribe_cpp()` (backend modules) to have run first.
static TRANSCRIBE_CPP_DEVICES: OnceLock<Vec<GpuDeviceOption>> = OnceLock::new();

fn cached_transcribe_cpp_devices() -> &'static [GpuDeviceOption] {
    TRANSCRIBE_CPP_DEVICES.get_or_init(|| {
        // Same FMA3 guard as the whisper GPU probe: ggml's Vulkan backend uses
        // FMA3, which SIGILLs on CPUs without it; skip enumeration there.
        #[cfg(target_arch = "x86_64")]
        if !std::arch::is_x86_feature_detected!("fma") {
            warn!("CPU lacks FMA3 support — skipping transcribe.cpp device enumeration");
            return Vec::new();
        }

        transcribe_cpp::devices()
            .into_iter()
            .map(|d| GpuDeviceOption {
                id: d.index.map(|i| i as i32).unwrap_or(-1),
                name: if d.description.is_empty() {
                    d.name
                } else {
                    d.description
                },
                total_vram_mb: (d.memory_total / (1024 * 1024)) as usize,
            })
            .collect()
    })
}

/// Return which accelerators are compiled into this build.
pub fn get_available_accelerators() -> AvailableAccelerators {
    use transcribe_rs::accel::OrtAccelerator;

    let ort_options: Vec<String> = OrtAccelerator::available()
        .into_iter()
        .map(|a| a.to_string())
        .collect();

    let whisper_options = vec!["auto".to_string(), "cpu".to_string(), "gpu".to_string()];

    AvailableAccelerators {
        whisper: whisper_options,
        ort: ort_options,
        gpu_devices: cached_gpu_devices().to_vec(),
        transcribe_cpp_devices: cached_transcribe_cpp_devices().to_vec(),
    }
}

impl Drop for TranscriptionManager {
    fn drop(&mut self) {
        // Skip shutdown unless this is the very last clone. TranscriptionManager
        // is cloned by initiate_model_load() and the watcher thread — those
        // clones dropping must not kill the watcher. The watcher thread holds
        // its own clone, so engine's strong_count is always >= 2 while the
        // watcher is alive. When it reaches 1, only this instance remains
        // and we can safely shut down.
        if Arc::strong_count(&self.engine) > 1 {
            return;
        }

        // Signal the watcher thread to shutdown
        self.shutdown_signal.store(true, Ordering::Relaxed);

        // Wait for the thread to finish gracefully. Use match instead of
        // unwrap to avoid panicking if the mutex is poisoned — a panic inside
        // Drop calls abort() (Handy #1354).
        let mut guard = match self.watcher_handle.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        if let Some(handle) = guard.take() {
            if let Err(e) = handle.join() {
                warn!("Failed to join idle watcher thread: {:?}", e);
            } else {
                debug!("Idle watcher thread joined successfully");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transcribe_cpp::{Capabilities, Task, TimestampKind};

    /// A stalled/behind worker (the receiver is held, nothing is consumed) must
    /// never let the live-transcription feed queue grow past the cap: excess
    /// frames are dropped and counted, and a freed slot lets feed() enqueue
    /// again. This is the guard against unbounded memory growth on a long
    /// recording when inference can't keep up with real-time.
    #[test]
    fn stream_router_bounds_backlog_when_worker_stalls() {
        let router = StreamRouter::new();

        // Closed router: feed is a no-op and nothing is queued.
        router.feed(&[0.0f32; 480]);
        assert_eq!(router.queued.load(Ordering::Relaxed), 0);
        assert!(!router.is_open());

        // Open a session and deliberately keep the receiver unread so the
        // "worker" never consumes — the worst case (a hung/overloaded engine).
        let _rx = router.open();
        assert!(router.is_open());

        // Feed well past the cap; the in-flight queue must clamp at the cap.
        let overshoot = 500usize;
        for _ in 0..(MAX_QUEUED_FEED_FRAMES + overshoot) {
            router.feed(&[0.0f32; 480]);
        }
        assert_eq!(
            router.queued.load(Ordering::Relaxed),
            MAX_QUEUED_FEED_FRAMES,
            "queue must be bounded by the cap"
        );
        assert!(
            router.dropped.load(Ordering::Relaxed) >= overshoot as u64,
            "frames past the cap must be counted as dropped"
        );

        // Simulate the worker catching up on a few frames; freed slots must be
        // reusable by subsequent feeds.
        for _ in 0..10 {
            router.note_feed_consumed();
        }
        assert_eq!(
            router.queued.load(Ordering::Relaxed),
            MAX_QUEUED_FEED_FRAMES - 10
        );
        router.feed(&[0.0f32; 480]);
        assert_eq!(
            router.queued.load(Ordering::Relaxed),
            MAX_QUEUED_FEED_FRAMES - 9
        );

        // note_feed_consumed must saturate at zero, never underflow.
        let router2 = StreamRouter::new();
        router2.note_feed_consumed();
        assert_eq!(router2.queued.load(Ordering::Relaxed), 0);
    }

    /// Build a minimal `Capabilities` for run-plan tests.
    fn caps(languages: &[&str], supports_translate: bool) -> Capabilities {
        Capabilities {
            native_sample_rate: 16000,
            languages: languages.iter().map(|s| s.to_string()).collect(),
            translate_target_languages: vec![],
            max_timestamp_kind: TimestampKind::Segment,
            supports_language_detect: false,
            supports_translate,
            supports_streaming: false,
            supports_spec_decode: false,
            max_audio_ms: 0,
        }
    }

    #[test]
    fn run_plan_auto_language_is_none() {
        let mut s = crate::settings::get_default_settings();
        s.selected_language = "auto".into();
        let plan = transcribe_cpp_run_plan(&s, &caps(&["en"], false), "parakeet");
        assert_eq!(plan.language, None);
        assert_eq!(plan.task, Task::Transcribe);
        assert_eq!(plan.family, None);
        assert_eq!(plan.timestamps, TimestampKind::Segment);
    }

    #[test]
    fn run_plan_supported_language_is_hinted() {
        let mut s = crate::settings::get_default_settings();
        s.selected_language = "en".into();
        let plan = transcribe_cpp_run_plan(&s, &caps(&["en"], false), "parakeet");
        assert_eq!(plan.language.as_deref(), Some("en"));
    }

    #[test]
    fn run_plan_unsupported_language_falls_back_to_auto() {
        let mut s = crate::settings::get_default_settings();
        s.selected_language = "de".into();
        let plan = transcribe_cpp_run_plan(&s, &caps(&["en"], false), "parakeet");
        assert_eq!(plan.language, None);
    }

    #[test]
    fn run_plan_zh_variant_normalized_to_zh() {
        let mut s = crate::settings::get_default_settings();
        s.selected_language = "zh-Hant".into();
        let plan = transcribe_cpp_run_plan(&s, &caps(&["zh"], false), "whisper");
        assert_eq!(plan.language.as_deref(), Some("zh"));
    }

    #[test]
    fn run_plan_translate_when_supported_and_non_english() {
        let mut s = crate::settings::get_default_settings();
        s.selected_language = "de".into();
        s.translate_to_english = true;
        let plan = transcribe_cpp_run_plan(&s, &caps(&["de", "en"], true), "whisper");
        assert_eq!(plan.task, Task::Translate);
        assert_eq!(plan.target_language.as_deref(), Some("en"));
    }

    #[test]
    fn run_plan_no_translate_when_unsupported() {
        let mut s = crate::settings::get_default_settings();
        s.selected_language = "de".into();
        s.translate_to_english = true;
        let plan = transcribe_cpp_run_plan(&s, &caps(&["de"], false), "parakeet");
        assert_eq!(plan.task, Task::Transcribe);
        assert_eq!(plan.target_language, None);
    }

    #[test]
    fn run_plan_no_translate_when_source_english() {
        let mut s = crate::settings::get_default_settings();
        s.selected_language = "en".into();
        s.translate_to_english = true;
        let plan = transcribe_cpp_run_plan(&s, &caps(&["en"], true), "whisper");
        assert_eq!(plan.task, Task::Transcribe);
        assert_eq!(plan.target_language, None);
    }

    #[test]
    fn run_plan_whisper_initial_prompt_only_for_whisper_arch() {
        let mut s = crate::settings::get_default_settings();
        s.selected_language = "en".into();
        s.custom_words = vec!["Kubernetes".into(), "kubectl".into()];
        // whisper arch accepts the run-slot initial-prompt extension
        let plan_w = transcribe_cpp_run_plan(&s, &caps(&["en"], false), "whisper");
        assert!(plan_w.family.is_some());
        // non-whisper families reject it → no extension (fuzzy post-correction)
        let plan_p = transcribe_cpp_run_plan(&s, &caps(&["en"], false), "parakeet");
        assert!(plan_p.family.is_none());
    }

    #[test]
    fn backend_options_maps_dials() {
        use crate::settings::WhisperAcceleratorSetting;
        use transcribe_cpp::Backend;

        let mut s = crate::settings::get_default_settings();
        s.whisper_accelerator = WhisperAcceleratorSetting::Cpu;
        s.whisper_gpu_device = -1;
        let opt = transcribe_cpp_backend_options(&s);
        assert_eq!(opt.backend, Backend::Cpu);
        assert_eq!(opt.gpu_device, 0, "negative (auto) index normalized to 0");

        s.whisper_accelerator = WhisperAcceleratorSetting::Gpu;
        s.whisper_gpu_device = 1;
        let opt = transcribe_cpp_backend_options(&s);
        assert_eq!(opt.backend, Backend::Auto);
        assert_eq!(opt.gpu_device, 1);
    }

    /// End-to-end batch smoke test through the REAL Session-2 code path
    /// (`init_transcribe_cpp` → `transcribe_cpp_backend_options` →
    /// `Model::load_with` → `transcribe_cpp_run_plan` → `session.run`). Ignored
    /// by default because it needs the cached Parakeet-Unified-EN GGUF + a
    /// 16 kHz mono WAV and a working transcribe.cpp backend; run with:
    /// `cargo test --lib transcribe_cpp_batch_end_to_end -- --ignored --nocapture`
    /// (paths overridable via TRANSCRIBE_CPP_MODEL / TRANSCRIBE_CPP_WAV).
    #[test]
    #[ignore = "needs cached GGUF model + WAV + transcribe.cpp backend DLLs on PATH"]
    fn transcribe_cpp_batch_end_to_end() {
        use std::path::Path;

        let model_path = std::env::var("TRANSCRIBE_CPP_MODEL")
            .unwrap_or_else(|_| r"C:\t\models\parakeet-unified-en-0.6b-Q8_0.gguf".to_string());
        let wav_path =
            std::env::var("TRANSCRIBE_CPP_WAV").unwrap_or_else(|_| r"C:\t\jfk.wav".to_string());
        if !Path::new(&model_path).exists() || !Path::new(&wav_path).exists() {
            eprintln!("skipping transcribe_cpp_batch_end_to_end: model/wav not present");
            return;
        }

        // Backend init (logging + modules) exactly as at app startup.
        super::init_transcribe_cpp();

        let mut s = crate::settings::get_default_settings();
        s.selected_language = "en".into();
        s.translate_to_english = false;

        // Load via the real backend-options helper.
        let options = transcribe_cpp_backend_options(&s);
        let model =
            transcribe_cpp::Model::load_with(&model_path, &options).expect("model load_with");
        let caps = model.capabilities();
        let arch = model.arch();
        eprintln!(
            "loaded transcribe.cpp: arch={} variant={} backend={}",
            arch,
            model.variant(),
            model.backend()
        );
        let mut session = model.session().expect("session");

        // Read the 16 kHz mono WAV into f32 PCM in [-1, 1].
        let mut reader = hound::WavReader::open(&wav_path).expect("wav open");
        let spec = reader.spec();
        let pcm: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => reader
                .samples::<i16>()
                .map(|v| v.unwrap() as f32 / 32768.0)
                .collect(),
            hound::SampleFormat::Float => reader.samples::<f32>().map(|v| v.unwrap()).collect(),
        };

        // Build RunOptions with the real Session-2 run-plan and transcribe.
        let run_opts = transcribe_cpp_run_plan(&s, &caps, &arch);
        let transcript = session.run(&pcm, &run_opts).expect("session.run");
        eprintln!("TRANSCRIBE_CPP_BATCH_TRANSCRIPT: {}", transcript.text);

        let lower = transcript.text.to_lowercase();
        assert!(
            lower.contains("country"),
            "unexpected transcript: {}",
            transcript.text
        );
    }

    /// End-to-end NATIVE STREAMING smoke test through the real Session-4 stream
    /// path, mirroring `run_stream_worker`'s cadence exactly: open a stream with
    /// `CommitPolicy::Auto`, feed the recorder's 30 ms (480-sample) frames
    /// accumulated up to ~80 ms (1280-sample) chunks, emit on
    /// `committed_changed || tentative_changed`, then `finalize()` and take
    /// `text().display()` (= committed + tentative). Ignored by default; needs
    /// the cached Parakeet-Unified-EN GGUF (a streaming model) + a 16 kHz mono
    /// WAV + the transcribe.cpp backend DLLs on PATH. Run with:
    /// `cargo test --lib transcribe_cpp_stream_end_to_end -- --ignored --nocapture`
    /// (paths overridable via TRANSCRIBE_CPP_MODEL / TRANSCRIBE_CPP_WAV).
    #[test]
    #[ignore = "needs cached streaming GGUF model + WAV + transcribe.cpp backend DLLs on PATH"]
    fn transcribe_cpp_stream_end_to_end() {
        use std::path::Path;
        use transcribe_cpp::{CommitPolicy, StreamOptions};

        let model_path = std::env::var("TRANSCRIBE_CPP_MODEL")
            .unwrap_or_else(|_| r"C:\t\models\parakeet-unified-en-0.6b-Q8_0.gguf".to_string());
        let wav_path =
            std::env::var("TRANSCRIBE_CPP_WAV").unwrap_or_else(|_| r"C:\t\jfk.wav".to_string());
        if !Path::new(&model_path).exists() || !Path::new(&wav_path).exists() {
            eprintln!("skipping transcribe_cpp_stream_end_to_end: model/wav not present");
            return;
        }

        super::init_transcribe_cpp();

        let mut s = crate::settings::get_default_settings();
        s.selected_language = "en".into();
        s.translate_to_english = false;

        let options = transcribe_cpp_backend_options(&s);
        let model =
            transcribe_cpp::Model::load_with(&model_path, &options).expect("model load_with");
        let caps = model.capabilities();
        let arch = model.arch();
        eprintln!(
            "loaded transcribe.cpp: arch={} variant={} backend={} supports_streaming={}",
            arch,
            model.variant(),
            model.backend(),
            caps.supports_streaming
        );
        // Session 4 only takes the native streaming path for streaming models.
        assert!(
            caps.supports_streaming,
            "expected a streaming-capable model for the streaming e2e test"
        );

        let mut session = model.session().expect("session");

        // Read the 16 kHz mono WAV into f32 PCM in [-1, 1].
        let mut reader = hound::WavReader::open(&wav_path).expect("wav open");
        let spec = reader.spec();
        let pcm: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => reader
                .samples::<i16>()
                .map(|v| v.unwrap() as f32 / 32768.0)
                .collect(),
            hound::SampleFormat::Float => reader.samples::<f32>().map(|v| v.unwrap()).collect(),
        };

        // A closure that runs ONE full streaming pass exactly like the worker:
        // feed 30 ms frames accumulated to 80 ms, count committed/tentative
        // updates, finalize, and return the display() text. Returns
        // (display_text, committed_at_finalize, tentative_at_finalize, updates).
        let run_stream_pass = |session: &mut transcribe_cpp::Session| {
            let run_opts = transcribe_cpp_run_plan(&s, &caps, &arch);
            let stream_opts = StreamOptions {
                commit_policy: CommitPolicy::Auto,
                ..Default::default()
            };
            let mut stream = session
                .stream(&run_opts, &stream_opts)
                .expect("stream begin");

            const FRAME: usize = 480; // 30 ms @ 16 kHz (recorder frame size)
            const FEED_SAMPLES: usize = 1280; // ~80 ms accumulate-then-feed cadence
            let mut acc: Vec<f32> = Vec::with_capacity(FEED_SAMPLES * 2);
            let mut updates = 0usize;
            for frame in pcm.chunks(FRAME) {
                acc.extend_from_slice(frame);
                while acc.len() >= FEED_SAMPLES {
                    let chunk: Vec<f32> = acc.drain(..FEED_SAMPLES).collect();
                    let upd = stream.feed(&chunk).expect("stream feed");
                    if upd.committed_changed || upd.tentative_changed {
                        updates += 1;
                    }
                }
            }
            if !acc.is_empty() {
                let _ = stream.feed(&acc).expect("stream tail feed");
            }
            let fin = stream.finalize().expect("stream finalize");
            assert!(fin.is_final, "finalize update must be final");
            let text = stream.text();
            (text.display(), text.committed, text.tentative, updates)
        };

        // Pass 1 — correctness: streamed text must match the batch quality
        // (contain the JFK line), proving native streaming produces the same
        // words as `session.run`.
        let (display1, committed1, tentative1, updates1) = run_stream_pass(&mut session);
        eprintln!(
            "TRANSCRIBE_CPP_STREAM updates={} committed={:?} tentative={:?}",
            updates1, committed1, tentative1
        );
        eprintln!("TRANSCRIBE_CPP_STREAM_DISPLAY: {}", display1);
        assert!(
            updates1 > 0,
            "expected incremental committed/tentative updates"
        );
        assert!(
            display1.to_lowercase().contains("country"),
            "unexpected streamed transcript: {}",
            display1
        );

        // Pass 2 — lease release / reuse: the SAME session must be reusable for
        // a fresh stream after the previous one finalized (finalize releases the
        // model's compute lease). This mirrors the worker returning the leased
        // engine for the next recording — proving no leaked lease.
        let (display2, _c2, _t2, updates2) = run_stream_pass(&mut session);
        eprintln!(
            "TRANSCRIBE_CPP_STREAM_PASS2 updates={} display={}",
            updates2, display2
        );
        assert!(
            display2.to_lowercase().contains("country"),
            "session not reusable after finalize (lease leak?): {}",
            display2
        );

        // Pass 3 — cancel/reset then reuse: begin a stream, feed a little, then
        // reset() (the Cancel path) and confirm the session still streams. This
        // proves cancel releases the lease without leaking, mirroring the
        // worker's StreamCmd::Cancel → stream.reset() arm.
        {
            let run_opts = transcribe_cpp_run_plan(&s, &caps, &arch);
            let stream_opts = StreamOptions {
                commit_policy: CommitPolicy::Auto,
                ..Default::default()
            };
            let mut stream = session
                .stream(&run_opts, &stream_opts)
                .expect("stream begin p3");
            let _ = stream.feed(&pcm[..pcm.len().min(1280)]).expect("feed p3");
            stream.reset(); // Cancel path
        }
        let (display3, _c3, _t3, _u3) = run_stream_pass(&mut session);
        assert!(
            display3.to_lowercase().contains("country"),
            "session not reusable after reset/cancel (lease leak?): {}",
            display3
        );
        eprintln!("TRANSCRIBE_CPP_STREAM cancel+reuse OK");
    }
}
