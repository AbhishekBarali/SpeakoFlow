use std::{
    collections::VecDeque,
    io::Error,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Condvar, Mutex,
    },
    time::Duration,
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, Sample, SizedSample,
};

use crate::audio_toolkit::{
    audio::{AudioVisualiser, FrameResampler},
    constants,
    vad::{self, VadFrame, VadTail},
    VoiceActivityDetector,
};

enum Cmd {
    Start,
    Stop(mpsc::Sender<Vec<f32>>),
    Shutdown,
}

type FrameCallback = Arc<dyn Fn(&[f32]) + Send + Sync + 'static>;

/// Where the consumer hands 16 kHz audio besides the recording buffer.
#[derive(Clone, Default)]
struct FrameSinks {
    /// Every frame, silence included.
    raw: Option<FrameCallback>,
    /// Only what the VAD keeps.
    speech: Option<FrameCallback>,
}

/// How long opening the input stream may take before it is treated as hung.
/// `open()` runs while the recording state is locked, so a driver that never
/// answers (a wedged Bluetooth headset, a bad WASAPI/CoreAudio driver) used to
/// wedge the recording pipeline and everything that asks whether it is
/// recording. Generous, because a Bluetooth profile switch can take seconds.
const MIC_OPEN_TIMEOUT: Duration = Duration::from_secs(15);

// Audio and controls share a queue: Stop/Shutdown must wake a stalled microphone.
enum RecorderEvent {
    Command(Cmd),
    Samples(Vec<f32>),
    EndOfStream,
}

/// What the VAD has heard so far in the recording under way, measured on the
/// audio's own clock.
///
/// Written by the consumer thread for every 16 kHz frame it processes while
/// recording, and read by the transcription coordinator to tell when a
/// hands-free dictation has gone quiet (`dictation_silence_stop_secs`).
/// Positions count samples since [`AudioRecorder::start`], so silence is
/// measured in audio actually captured: a microphone that is slow to wake, or a
/// consumer thread that falls behind, can never make the user seem to have been
/// quiet for longer than they were. Lock-free on purpose: the recorder's own
/// lock is held for seconds while a microphone opens or drains, and the reader
/// is the thread every shortcut goes through.
#[derive(Debug, Default)]
pub struct SpeechActivity {
    /// Samples processed since the recording started.
    heard: AtomicU64,
    /// The sample position just past the last frame the VAD kept as speech,
    /// or 0 while nothing has been.
    speech_end: AtomicU64,
}

impl SpeechActivity {
    /// Back to "nothing heard yet". Done by [`AudioRecorder::start`] before it
    /// queues `Cmd::Start`, while the consumer is not recording, so it never
    /// races a write and a reader can never see the last recording's silence.
    fn reset(&self) {
        self.speech_end.store(0, Ordering::Relaxed);
        self.heard.store(0, Ordering::Release);
    }

    /// Count one processed frame. Only the consumer thread writes, so a plain
    /// load and store is enough. The speech mark is published before the
    /// length that covers it: a reader that sees the new length also sees the
    /// mark, so it can never pair a new length with an older mark and overstate
    /// the silence by a frame.
    fn record(&self, samples: usize, speech: bool) {
        let end = self.heard.load(Ordering::Relaxed) + samples as u64;
        if speech {
            self.speech_end.store(end, Ordering::Relaxed);
        }
        self.heard.store(end, Ordering::Release);
    }

    /// How long the recording has gone without speech: since the last frame
    /// the VAD kept, or since the recording started if it has kept none. The
    /// VAD's hangover counts as speech, so this starts a few hundred
    /// milliseconds after the last word.
    pub fn quiet_for(&self) -> Duration {
        let heard = self.heard.load(Ordering::Acquire);
        let speech_end = self.speech_end.load(Ordering::Relaxed);
        let quiet = heard.saturating_sub(speech_end);
        Duration::from_micros(
            quiet.saturating_mul(1_000_000) / u64::from(constants::WHISPER_SAMPLE_RATE),
        )
    }

    /// Whether anything in this recording has been kept as speech.
    pub fn heard_speech(&self) -> bool {
        self.speech_end.load(Ordering::Relaxed) > 0
    }
}

pub struct AudioRecorder {
    device: Option<Device>,
    cmd_tx: Option<mpsc::Sender<RecorderEvent>>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
    vad: Option<Arc<Mutex<Box<dyn vad::VoiceActivityDetector>>>>,
    level_cb: Option<Arc<dyn Fn(Vec<f32>) + Send + Sync + 'static>>,
    /// Optional raw-PCM frame callback invoked with each resampled 16 kHz mono
    /// frame while recording, BEFORE/independent of the VAD path. Used to feed
    /// a live/streaming transcriber (which needs the silence to detect chunk
    /// boundaries). `None` unless a streaming consumer is wired up.
    frame_cb: Option<FrameCallback>,
    /// Optional callback invoked with exactly the audio the VAD keeps, in
    /// order, as it is kept: the recording as it will be transcribed. Feeds
    /// long-form chunks that are decoded while the user is still speaking.
    speech_cb: Option<FrameCallback>,
    /// Becomes `true` once the microphone has delivered its first audio frame
    /// after [`AudioRecorder::start`]. Lets callers align the "start speaking"
    /// cue with a stream that is genuinely live, instead of guessing a fixed
    /// warm-up delay (which clips the first words on slow-to-wake devices).
    capture_ready: Arc<(Mutex<bool>, Condvar)>,
    /// Cached preferred input config, keyed by device name. Enumerating a
    /// device's supported configs is a slow syscall on some platforms and, in
    /// on-demand microphone mode, `open()` runs on every recording start.
    /// Reusing the resolved config for the same device removes that cost from
    /// the hot path. Only stored once the device has *accepted* the config,
    /// and cleared whenever an open fails, so a stale rate/format (Bluetooth
    /// profile switch, sample rate changed in the OS) self-heals on the
    /// caller's retry instead of failing every open until restart.
    /// Backport of Handy PR #1582 (faster mic initialization).
    config_cache: Option<(String, cpal::SupportedStreamConfig)>,
    /// Set by cpal's error callback when the active input stream can no longer
    /// capture (device unplugged, USB/Bluetooth dropout). The consumer thread
    /// never exits on its own here — commands share the sample queue — so this
    /// flag is the only signal that the stream must be rebuilt.
    /// Backport of Handy PRs #1874 / #1838.
    stream_error: Arc<AtomicBool>,
    /// What the VAD has heard in the current recording. Shared, so whoever
    /// decides when a recording has gone quiet can read it without this
    /// recorder's lock (see [`AudioRecorder::with_speech_activity`]).
    speech: Arc<SpeechActivity>,
}

impl AudioRecorder {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(AudioRecorder {
            device: None,
            cmd_tx: None,
            worker_handle: None,
            vad: None,
            level_cb: None,
            frame_cb: None,
            speech_cb: None,
            capture_ready: Arc::new((Mutex::new(false), Condvar::new())),
            config_cache: None,
            stream_error: Arc::new(AtomicBool::new(false)),
            speech: Arc::new(SpeechActivity::default()),
        })
    }

    pub fn with_vad(mut self, vad: Box<dyn VoiceActivityDetector>) -> Self {
        self.vad = Some(Arc::new(Mutex::new(vad)));
        self
    }

    /// Report what the VAD hears into `speech`, which the caller keeps. The
    /// recorder lives behind a lock that opening or draining the microphone
    /// holds for seconds, so a handle taken up front is the only way to read
    /// this without waiting on that.
    pub fn with_speech_activity(mut self, speech: Arc<SpeechActivity>) -> Self {
        self.speech = speech;
        self
    }

    pub fn with_level_callback<F>(mut self, cb: F) -> Self
    where
        F: Fn(Vec<f32>) + Send + Sync + 'static,
    {
        self.level_cb = Some(Arc::new(cb));
        self
    }

    /// Register a callback invoked with each resampled 16 kHz mono frame while
    /// recording. Frames are delivered pre-VAD (including silence) so a
    /// streaming transcriber can detect its own chunk boundaries.
    pub fn with_frame_callback<F>(mut self, cb: F) -> Self
    where
        F: Fn(&[f32]) + Send + Sync + 'static,
    {
        self.frame_cb = Some(Arc::new(cb));
        self
    }

    /// Register a callback invoked with the audio the VAD keeps, as it keeps
    /// it: the same samples, in the same order, as the buffer [`stop`]
    /// returns.
    ///
    /// [`stop`]: AudioRecorder::stop
    pub fn with_speech_callback<F>(mut self, cb: F) -> Self
    where
        F: Fn(&[f32]) + Send + Sync + 'static,
    {
        self.speech_cb = Some(Arc::new(cb));
        self
    }

    /// Set how much trailing silence the VAD keeps after speech, for the
    /// recordings that start from now on. Called before [`start`], since a
    /// recording's tail follows the model that will transcribe it.
    ///
    /// [`start`]: AudioRecorder::start
    pub fn set_vad_tail(&self, tail: VadTail) {
        if let Some(vad) = &self.vad {
            vad.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .set_hangover_frames(tail.hangover_frames());
        }
    }

    pub fn open(&mut self, device: Option<Device>) -> Result<(), Box<dyn std::error::Error>> {
        if self.worker_handle.is_some() {
            if !self.needs_reopen() {
                return Ok(()); // already open
            }
            log::warn!("Capture stream failed; rebuilding microphone stream");
            let _ = self.close();
        }

        self.stream_error.store(false, Ordering::Relaxed);

        let (sample_tx, sample_rx) = mpsc::channel::<RecorderEvent>();
        let cmd_tx = sample_tx.clone();
        let (init_tx, init_rx) = mpsc::sync_channel::<Result<(), String>>(1);

        let host = crate::audio_toolkit::get_cpal_host();
        let device = match device {
            Some(dev) => dev,
            None => host
                .default_input_device()
                .ok_or_else(|| Error::new(std::io::ErrorKind::NotFound, "No input device found"))?,
        };

        // Resolve the preferred input config once, reusing the cached value for
        // the same device. In on-demand mode `open()` runs on every recording
        // start, and enumerating supported configs is a slow syscall on some
        // platforms — Handy PR #1582 measured mic-init being dominated by it.
        // `build_stream`/`play()` still run per open (they aren't cacheable),
        // but the enumeration drops to ~0 on repeat recordings of one device.
        let device_name = device.name().unwrap_or_default();
        // Reuse the cached config if it matches this device; the clone releases
        // the borrow on `self.config_cache` before we may reassign it below.
        let cached_config = match self.config_cache.as_ref() {
            Some((cached_name, cached_cfg)) if *cached_name == device_name => {
                Some(cached_cfg.clone())
            }
            _ => None,
        };
        let config = match cached_config {
            Some(cfg) => {
                log::debug!("Reusing cached input config for '{device_name}'");
                cfg
            }
            None => AudioRecorder::get_preferred_config(&device).map_err(|e| {
                Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to fetch preferred config: {e}"),
                )
            })?,
        };
        // Cached only after the device accepts it (see the init result below).
        let config_to_cache = (!device_name.is_empty()).then(|| (device_name, config.clone()));

        let thread_device = device.clone();
        let vad = self.vad.clone();
        // Move the optional level callback into the worker thread
        let level_cb = self.level_cb.clone();
        // Move the optional raw-frame and speech callbacks into the worker
        // thread (live transcription)
        let sinks = FrameSinks {
            raw: self.frame_cb.clone(),
            speech: self.speech_cb.clone(),
        };
        let capture_ready = self.capture_ready.clone();
        let stream_error = self.stream_error.clone();
        let speech = self.speech.clone();

        let worker = std::thread::spawn(move || {
            let stop_flag = Arc::new(AtomicBool::new(false));
            let stop_flag_for_stream = stop_flag.clone();
            // Set while the stream is open but not recording (always-on mic,
            // or the lazy-close window after a dictation): the callback then
            // returns at once instead of copying each block and waking the
            // consumer thread just to discard it. Separate from `stop_flag`
            // so the stop/end-of-stream handshake is untouched. `Cmd::Start`
            // clears it, `Cmd::Stop` sets it again after replying.
            let idle = Arc::new(AtomicBool::new(true));
            let idle_for_stream = idle.clone();
            let init_result = (|| -> Result<(cpal::Stream, u32), String> {
                // `config` was resolved (and cached) in `open()` before this
                // worker was spawned, so we don't re-enumerate the device here.
                // Backport of Handy PR #1582 (faster mic initialization).
                let sample_rate = config.sample_rate().0;
                let channels = config.channels() as usize;

                log::info!(
                    "Using device: {:?}\nSample rate: {}\nChannels: {}\nFormat: {:?}",
                    thread_device.name(),
                    sample_rate,
                    channels,
                    config.sample_format()
                );

                let stream = match config.sample_format() {
                    cpal::SampleFormat::U8 => AudioRecorder::build_stream::<u8>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        idle_for_stream,
                        stop_flag_for_stream,
                        stream_error.clone(),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    cpal::SampleFormat::I8 => AudioRecorder::build_stream::<i8>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        idle_for_stream,
                        stop_flag_for_stream,
                        stream_error.clone(),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    cpal::SampleFormat::I16 => AudioRecorder::build_stream::<i16>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        idle_for_stream,
                        stop_flag_for_stream,
                        stream_error.clone(),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    cpal::SampleFormat::I32 => AudioRecorder::build_stream::<i32>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        idle_for_stream,
                        stop_flag_for_stream,
                        stream_error.clone(),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    cpal::SampleFormat::F32 => AudioRecorder::build_stream::<f32>(
                        &thread_device,
                        &config,
                        sample_tx,
                        channels,
                        idle_for_stream,
                        stop_flag_for_stream,
                        stream_error.clone(),
                    )
                    .map_err(|e| format!("Failed to build input stream: {e}"))?,
                    sample_format => {
                        return Err(format!("Unsupported sample format: {sample_format:?}"));
                    }
                };

                stream
                    .play()
                    .map_err(|e| format!("Failed to start microphone stream: {e}"))?;

                Ok((stream, sample_rate))
            })();

            match init_result {
                Ok((stream, sample_rate)) => {
                    let _ = init_tx.send(Ok(()));
                    // Keep the stream alive while we process samples.
                    run_consumer(
                        sample_rate,
                        vad,
                        sample_rx,
                        level_cb,
                        sinks,
                        idle,
                        stop_flag,
                        capture_ready,
                        speech,
                    );
                    drop(stream);
                }
                Err(error_message) => {
                    log::error!("{error_message}");
                    let _ = init_tx.send(Err(error_message));
                }
            }
        });

        match init_rx.recv_timeout(MIC_OPEN_TIMEOUT) {
            Ok(Ok(())) => {
                if let Some(entry) = config_to_cache {
                    self.config_cache = Some(entry);
                }
                self.device = Some(device);
                self.cmd_tx = Some(cmd_tx);
                self.worker_handle = Some(worker);
                Ok(())
            }
            Ok(Err(error_message)) => {
                let _ = worker.join();
                // A failed open may mean the cached config went stale (device
                // re-plugged, rate/format changed in the OS). Drop it so the
                // caller's retry re-queries the device.
                self.config_cache = None;
                let kind = if is_microphone_access_denied(&error_message) {
                    std::io::ErrorKind::PermissionDenied
                } else {
                    std::io::ErrorKind::Other
                };
                Err(Box::new(Error::new(kind, error_message)))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // The driver never answered. The worker cannot be interrupted
                // inside cpal, so it is detached; the queued Shutdown makes it
                // close the stream and exit if the open ever completes.
                let _ = cmd_tx.send(RecorderEvent::Command(Cmd::Shutdown));
                drop(worker);
                self.config_cache = None;
                Err(Box::new(Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "The microphone did not start within {}s",
                        MIC_OPEN_TIMEOUT.as_secs()
                    ),
                )))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = worker.join();
                self.config_cache = None;
                Err(Box::new(Error::other(
                    "Failed to initialize microphone worker: the worker exited",
                )))
            }
        }
    }

    pub fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Reset capture-readiness before signalling Start so a stale `true`
        // from a previous recording can't satisfy a waiter before the mic is
        // actually delivering samples for this session.
        {
            let (lock, _) = &*self.capture_ready;
            *lock.lock().unwrap() = false;
        }
        // Same reasoning for what the VAD has heard: a reader polling between
        // this call and the consumer picking up `Start` must see a recording
        // that has just begun, not the last one's trailing silence.
        self.speech.reset();
        if let Some(tx) = &self.cmd_tx {
            tx.send(RecorderEvent::Command(Cmd::Start))?;
        }
        Ok(())
    }

    /// Shared handle that flips to `true` once the microphone delivers its
    /// first audio frame after [`AudioRecorder::start`]. Callers can wait on
    /// the [`Condvar`] to time the "start speaking" cue to a live stream.
    pub fn capture_ready_handle(&self) -> Arc<(Mutex<bool>, Condvar)> {
        self.capture_ready.clone()
    }

    pub fn stop(&self) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        let (resp_tx, resp_rx) = mpsc::channel();
        let tx = self.cmd_tx.as_ref().ok_or_else(|| {
            Error::new(std::io::ErrorKind::NotConnected, "Microphone is not open")
        })?;
        tx.send(RecorderEvent::Command(Cmd::Stop(resp_tx)))?;
        Ok(resp_rx.recv()?) // wait for the samples
    }

    /// True when the active capture stream must be rebuilt before the next
    /// recording: cpal reported a stream error (device unplugged, USB or
    /// Bluetooth dropout), or the worker thread has exited. Without this an
    /// always-on or lazily-closed stream kept handing back silent, empty
    /// recordings until the app was restarted.
    pub fn needs_reopen(&self) -> bool {
        self.stream_error.load(Ordering::Relaxed)
            || self
                .worker_handle
                .as_ref()
                .is_some_and(|handle| handle.is_finished())
    }

    pub fn close(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(tx) = self.cmd_tx.take() {
            let _ = tx.send(RecorderEvent::Command(Cmd::Shutdown));
        }
        if let Some(h) = self.worker_handle.take() {
            let _ = h.join();
        }
        self.device = None;
        Ok(())
    }

    fn build_stream<T>(
        device: &cpal::Device,
        config: &cpal::SupportedStreamConfig,
        sample_tx: mpsc::Sender<RecorderEvent>,
        channels: usize,
        idle: Arc<AtomicBool>,
        stop_flag: Arc<AtomicBool>,
        stream_error: Arc<AtomicBool>,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        T: Sample + SizedSample + Send + 'static,
        f32: cpal::FromSample<T>,
    {
        let mut output_buffer = Vec::new();
        let mut eos_sent = false;

        let stream_cb = move |data: &[T], _: &cpal::InputCallbackInfo| {
            handle_input_block(
                data,
                channels,
                &idle,
                &stop_flag,
                &mut eos_sent,
                &mut output_buffer,
                &sample_tx,
            );
        };

        device.build_input_stream(
            &config.clone().into(),
            stream_cb,
            move |err| {
                log::error!("Stream error: {}", err);
                stream_error.store(true, Ordering::Relaxed);
            },
            None,
        )
    }

    fn get_preferred_config(
        device: &cpal::Device,
    ) -> Result<cpal::SupportedStreamConfig, Box<dyn std::error::Error>> {
        // Use the device's native/default sample rate and let the FrameResampler
        // in run_consumer() downsample to 16kHz. This avoids forcing hardware into
        // a non-native rate which can cause issues on some devices (Bluetooth
        // codecs, certain ALSA drivers, etc.).
        let default_config = device.default_input_config()?;
        let target_rate = default_config.sample_rate();

        // Try to find the best sample format at the device's default rate
        let supported_configs = match device.supported_input_configs() {
            Ok(configs) => configs,
            Err(e) => {
                log::warn!("Could not enumerate input configs ({e}), using device default");
                return Ok(default_config);
            }
        };
        let mut best_config: Option<cpal::SupportedStreamConfigRange> = None;

        for config_range in supported_configs {
            if config_range.min_sample_rate() <= target_rate
                && config_range.max_sample_rate() >= target_rate
            {
                match best_config {
                    None => best_config = Some(config_range),
                    Some(ref current) => {
                        // Prioritize F32 > I16 > I32 > others
                        let score = |fmt: cpal::SampleFormat| match fmt {
                            cpal::SampleFormat::F32 => 4,
                            cpal::SampleFormat::I16 => 3,
                            cpal::SampleFormat::I32 => 2,
                            _ => 1,
                        };

                        if score(config_range.sample_format()) > score(current.sample_format()) {
                            best_config = Some(config_range);
                        }
                    }
                }
            }
        }

        if let Some(config) = best_config {
            return Ok(config.with_sample_rate(target_rate));
        }

        // Fall back to device default if no config matched (exotic/virtual devices)
        log::warn!(
            "No supported config matched device default rate {:?}, using default config",
            target_rate
        );
        Ok(default_config)
    }
}

/// Body of the cpal input callback, extracted so it can be tested without a
/// device. Converts the block to mono and forwards it.
///
/// The block that first observes the stop flag was captured *before* the stop,
/// so it is still forwarded, followed by the end-of-stream sentinel. Dropping
/// it (as this callback used to) lost up to one callback period of tail audio
/// on every recording — 10-100 ms, worst on Bluetooth — which is the clipped
/// last syllable on a push-to-talk release. Later blocks are dropped until the
/// flag clears. Backport of Handy PR #1958.
///
/// While `idle` is set (stream open, not recording) every block is dropped
/// before any conversion or send. Recording state never changes inside an
/// idle span: `Cmd::Stop` sets `idle` only after its end-of-stream drain has
/// replied, and `Cmd::Start` clears it before the first block is wanted.
fn handle_input_block<T>(
    data: &[T],
    channels: usize,
    idle: &AtomicBool,
    stop_flag: &AtomicBool,
    eos_sent: &mut bool,
    output_buffer: &mut Vec<f32>,
    sample_tx: &mpsc::Sender<RecorderEvent>,
) where
    T: Sample,
    f32: cpal::FromSample<T>,
{
    if idle.load(Ordering::Relaxed) {
        return;
    }
    let stopping = stop_flag.load(Ordering::Relaxed);
    if stopping && *eos_sent {
        return;
    }

    output_buffer.clear();

    if channels == 1 {
        output_buffer.extend(data.iter().map(|&sample| sample.to_sample::<f32>()));
    } else {
        let frame_count = data.len() / channels;
        output_buffer.reserve(frame_count);

        for frame in data.chunks_exact(channels) {
            let mono_sample = frame
                .iter()
                .map(|&sample| sample.to_sample::<f32>())
                .sum::<f32>()
                / channels as f32;
            output_buffer.push(mono_sample);
        }
    }

    // A failed send means the consumer thread is gone. During shutdown that is
    // expected (the consumer exits before the stream is dropped), so only
    // report it when capture was supposed to be live.
    if sample_tx
        .send(RecorderEvent::Samples(output_buffer.clone()))
        .is_err()
        && !stopping
    {
        log::error!("Failed to send samples");
    }

    if stopping {
        let _ = sample_tx.send(RecorderEvent::EndOfStream);
        *eos_sent = true;
    } else {
        *eos_sent = false;
    }
}

pub fn is_microphone_access_denied(error_message: &str) -> bool {
    let normalized = error_message.to_lowercase();
    normalized.contains("access is denied")
        || normalized.contains("permission denied")
        || normalized.contains("0x80070005")
}

pub fn is_no_input_device_error(error_message: &str) -> bool {
    let normalized = error_message.to_lowercase();
    normalized.contains("no input device found")
        || (normalized.contains("failed to fetch preferred config")
            && normalized.contains("coreaudio"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn stop_and_shutdown_work_when_microphone_never_delivers_audio() {
        let (tx, rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            run_consumer(
                16_000,
                None,
                rx,
                None,
                FrameSinks::default(),
                Arc::new(AtomicBool::new(true)),
                Arc::new(AtomicBool::new(false)),
                Arc::new((Mutex::new(false), Condvar::new())),
                Arc::default(),
            );
            done_tx.send(()).unwrap();
        });
        let (reply_tx, reply_rx) = mpsc::channel();
        tx.send(RecorderEvent::Command(Cmd::Start)).unwrap();
        tx.send(RecorderEvent::Command(Cmd::Stop(reply_tx)))
            .unwrap();
        assert!(reply_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .is_empty());
        tx.send(RecorderEvent::Command(Cmd::Shutdown)).unwrap();
        // Keep the producer connected: no audio, disconnect, or timer should
        // be required to wake the consumer and close the microphone.
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn stopping_an_unopened_microphone_returns_an_error() {
        assert!(AudioRecorder::new().unwrap().stop().is_err());
    }

    #[test]
    fn unopened_recorder_does_not_need_reopen() {
        // No worker has been spawned yet, so there is nothing to rebuild.
        // Guards against every first open() taking the rebuild path.
        let recorder = AudioRecorder::new().expect("recorder");
        assert!(!recorder.needs_reopen());
    }

    #[test]
    fn stream_error_requires_reopen() {
        let recorder = AudioRecorder::new().expect("recorder");
        recorder.stream_error.store(true, Ordering::Relaxed);
        assert!(recorder.needs_reopen());
    }

    #[test]
    fn boundary_block_forwarded_before_eos() {
        let (tx, rx) = mpsc::channel();
        let idle = AtomicBool::new(false);
        let stop_flag = AtomicBool::new(false);
        let mut eos_sent = false;
        let mut scratch = Vec::new();
        let mut push = |flag: &AtomicBool, eos: &mut bool, block: &[f32]| {
            handle_input_block::<f32>(block, 1, &idle, flag, eos, &mut scratch, &tx)
        };

        // Running: blocks forwarded, no sentinel.
        push(&stop_flag, &mut eos_sent, &[0.1]);
        assert!(matches!(rx.try_recv(), Ok(RecorderEvent::Samples(_))));
        assert!(rx.try_recv().is_err());

        // The block observing the stop flag is still forwarded, then EOS.
        stop_flag.store(true, Ordering::Relaxed);
        push(&stop_flag, &mut eos_sent, &[0.5, 0.5]);
        match rx.try_recv() {
            Ok(RecorderEvent::Samples(samples)) => assert_eq!(samples, vec![0.5, 0.5]),
            _ => panic!("boundary block must be forwarded, not dropped"),
        }
        assert!(matches!(rx.try_recv(), Ok(RecorderEvent::EndOfStream)));

        // Later blocks are dropped until the flag clears, then capture resumes.
        push(&stop_flag, &mut eos_sent, &[0.9]);
        assert!(rx.try_recv().is_err(), "blocks after EOS must be dropped");
        stop_flag.store(false, Ordering::Relaxed);
        push(&stop_flag, &mut eos_sent, &[0.2]);
        assert!(matches!(rx.try_recv(), Ok(RecorderEvent::Samples(_))));
        assert!(rx.try_recv().is_err(), "no sentinel while running");
    }

    #[test]
    fn idle_stream_ships_nothing_and_keeps_the_stop_handshake() {
        let (tx, rx) = mpsc::channel();
        let idle = AtomicBool::new(true);
        let stop = AtomicBool::new(false);
        let mut eos_sent = false;
        let mut scratch = Vec::new();
        let mut push = |idle: &AtomicBool, stop: &AtomicBool, eos: &mut bool, block: &[f32]| {
            handle_input_block::<f32>(block, 1, idle, stop, eos, &mut scratch, &tx)
        };

        // Freshly opened (or between recordings): nothing reaches the consumer.
        push(&idle, &stop, &mut eos_sent, &[0.9]);
        assert!(
            rx.try_recv().is_err(),
            "an idle stream must not ship blocks"
        );

        // Start: blocks flow. Stop: boundary block, then exactly one EOS.
        idle.store(false, Ordering::Relaxed);
        push(&idle, &stop, &mut eos_sent, &[0.1]);
        assert!(matches!(rx.try_recv(), Ok(RecorderEvent::Samples(_))));
        stop.store(true, Ordering::Relaxed);
        push(&idle, &stop, &mut eos_sent, &[0.2]);
        assert!(matches!(rx.try_recv(), Ok(RecorderEvent::Samples(_))));
        assert!(matches!(rx.try_recv(), Ok(RecorderEvent::EndOfStream)));

        // After the reply the consumer parks the callback, then clears stop:
        // still nothing shipped, and no stray sentinel left for the next Stop.
        idle.store(true, Ordering::Relaxed);
        stop.store(false, Ordering::Relaxed);
        push(&idle, &stop, &mut eos_sent, &[0.3]);
        assert!(rx.try_recv().is_err());

        // Next recording: the first block flows and re-arms the sentinel, so
        // its Stop gets its own EOS.
        idle.store(false, Ordering::Relaxed);
        push(&idle, &stop, &mut eos_sent, &[0.4]);
        assert!(matches!(rx.try_recv(), Ok(RecorderEvent::Samples(_))));
        assert!(rx.try_recv().is_err(), "no sentinel while running");
        stop.store(true, Ordering::Relaxed);
        push(&idle, &stop, &mut eos_sent, &[0.5]);
        assert!(matches!(rx.try_recv(), Ok(RecorderEvent::Samples(_))));
        assert!(matches!(rx.try_recv(), Ok(RecorderEvent::EndOfStream)));
    }

    #[test]
    fn warm_idle_microphone_does_not_publish_levels_or_stream_audio() {
        let (sample_tx, sample_rx) = mpsc::channel();
        // One second of audio from a warm 48 kHz microphone, without Start.
        for _ in 0..100 {
            sample_tx
                .send(RecorderEvent::Samples(vec![0.2; 480]))
                .unwrap();
        }
        drop(sample_tx);
        let levels = Arc::new(AtomicUsize::new(0));
        let frames = Arc::new(AtomicUsize::new(0));
        let level_count = levels.clone();
        let frame_count = frames.clone();
        let ready = Arc::new((Mutex::new(false), Condvar::new()));
        run_consumer(
            48_000,
            None,
            sample_rx,
            Some(Arc::new(move |_| {
                level_count.fetch_add(1, Ordering::Relaxed);
            })),
            FrameSinks {
                raw: Some(Arc::new(move |_| {
                    frame_count.fetch_add(1, Ordering::Relaxed);
                })),
                speech: None,
            },
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(false)),
            ready.clone(),
            Arc::default(),
        );
        assert_eq!(levels.load(Ordering::Relaxed), 0);
        assert_eq!(frames.load(Ordering::Relaxed), 0);
        assert!(!*ready.0.lock().unwrap());
    }

    #[test]
    fn recording_after_idle_preserves_streaming_and_stop_tail() {
        let (sample_tx, sample_rx) = mpsc::channel();
        let cmd_tx = sample_tx.clone();
        let (reply_tx, reply_rx) = mpsc::channel();
        let (level_tx, level_rx) = mpsc::channel();
        let (frame_tx, frame_rx) = mpsc::channel();
        // An opened stream starts idle, exactly like `open()` sets it.
        let idle = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let ready = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_idle = idle.clone();
        let worker_stop = stop.clone();
        let worker_ready = ready.clone();
        let worker = std::thread::spawn(move || {
            run_consumer(
                16_000,
                None,
                sample_rx,
                Some(Arc::new(move |levels| {
                    let _ = level_tx.send(levels);
                })),
                FrameSinks {
                    raw: Some(Arc::new(move |frame| {
                        let _ = frame_tx.send(frame.to_vec());
                    })),
                    speech: None,
                },
                worker_idle,
                worker_stop,
                worker_ready,
                Arc::default(),
            )
        });

        // Queued idle audio precedes Start and must never enter the output.
        sample_tx
            .send(RecorderEvent::Samples(vec![0.9; 480]))
            .unwrap();
        cmd_tx.send(RecorderEvent::Command(Cmd::Start)).unwrap();
        sample_tx
            .send(RecorderEvent::Samples(vec![0.2; 960]))
            .unwrap();
        level_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(*ready.0.lock().unwrap());
        assert!(
            !idle.load(Ordering::Relaxed),
            "Start resumes the capture callback"
        );

        // Stop consumes this chunk and flushes the partial final frame before
        // replying. Both the batch transcript and cloud stream need that tail.
        cmd_tx
            .send(RecorderEvent::Command(Cmd::Stop(reply_tx)))
            .unwrap();
        sample_tx
            .send(RecorderEvent::Samples(vec![0.3; 240]))
            .unwrap();
        sample_tx.send(RecorderEvent::EndOfStream).unwrap();
        let output = reply_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(sample_tx);
        drop(cmd_tx);
        worker.join().unwrap();
        let streamed: Vec<f32> = frame_rx.try_iter().flatten().collect();
        assert_eq!(streamed, output);
        assert_eq!(&output[..960], &[0.2; 960]);
        assert_eq!(&output[960..1200], &[0.3; 240]);
        assert!(output[1200..].iter().all(|s| *s == 0.0));
        // Between recordings the capture callback is parked (it drops blocks
        // instead of shipping them to be discarded) and the stop flag is clear
        // again, ready for the next Start.
        assert!(idle.load(Ordering::Relaxed));
        assert!(!stop.load(Ordering::Relaxed));
    }

    #[test]
    fn silence_is_counted_from_the_last_speech_on_the_audio_clock() {
        let speech = SpeechActivity::default();
        assert_eq!(speech.quiet_for(), Duration::ZERO);
        assert!(!speech.heard_speech());
        // Room tone before the first word is silence too: someone who never
        // speaks is quiet from the moment the recording starts.
        speech.record(4_800, false);
        assert_eq!(speech.quiet_for(), Duration::from_millis(300));
        // Speech resets it, and the silence after it grows from there.
        speech.record(480, true);
        assert_eq!(speech.quiet_for(), Duration::ZERO);
        assert!(speech.heard_speech());
        speech.record(16_000, false);
        assert_eq!(speech.quiet_for(), Duration::from_secs(1));
        // The next recording starts from nothing, not from this one's silence.
        speech.reset();
        assert_eq!(speech.quiet_for(), Duration::ZERO);
        assert!(!speech.heard_speech());
    }

    /// Calls a frame speech when anything in it is loud: enough to drive the
    /// consumer without loading a model.
    struct LoudIsSpeech;
    impl VoiceActivityDetector for LoudIsSpeech {
        fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> anyhow::Result<VadFrame<'a>> {
            Ok(if frame.iter().any(|s| s.abs() > 0.5) {
                VadFrame::Speech(frame)
            } else {
                VadFrame::Noise
            })
        }
    }

    #[test]
    fn the_consumer_reports_the_silence_after_the_last_word() {
        let (tx, rx) = mpsc::channel();
        let speech = Arc::new(SpeechActivity::default());
        let worker_speech = speech.clone();
        let vad: Box<dyn VoiceActivityDetector> = Box::new(LoudIsSpeech);
        let worker = std::thread::spawn(move || {
            run_consumer(
                16_000,
                Some(Arc::new(Mutex::new(vad))),
                rx,
                None,
                FrameSinks::default(),
                Arc::new(AtomicBool::new(true)),
                Arc::new(AtomicBool::new(false)),
                Arc::new((Mutex::new(false), Condvar::new())),
                worker_speech,
            )
        });

        // Two 30 ms frames of speech, then three of silence.
        tx.send(RecorderEvent::Command(Cmd::Start)).unwrap();
        tx.send(RecorderEvent::Samples(vec![0.9; 960])).unwrap();
        tx.send(RecorderEvent::Samples(vec![0.0; 1440])).unwrap();
        let (reply_tx, reply_rx) = mpsc::channel();
        tx.send(RecorderEvent::Command(Cmd::Stop(reply_tx)))
            .unwrap();
        tx.send(RecorderEvent::EndOfStream).unwrap();
        let kept = reply_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(tx);
        worker.join().unwrap();

        assert_eq!(kept, vec![0.9; 960], "only the speech is kept");
        assert!(speech.heard_speech());
        assert_eq!(speech.quiet_for(), Duration::from_millis(90));
    }

    #[test]
    fn a_streaming_tail_keeps_more_silence_but_not_a_later_silence_stop() {
        use crate::audio_toolkit::vad::SmoothedVad;

        let (tx, rx) = mpsc::channel();
        let speech = Arc::new(SpeechActivity::default());
        let worker_speech = speech.clone();
        let mut smoothed = SmoothedVad::new(Box::new(LoudIsSpeech), 15, 15, 2);
        smoothed.set_hangover_frames(VadTail::Streaming.hangover_frames());
        let vad: Box<dyn VoiceActivityDetector> = Box::new(smoothed);
        let (speech_tx, speech_rx) = mpsc::channel::<Vec<f32>>();
        let sinks = FrameSinks {
            raw: None,
            speech: Some(Arc::new(move |kept: &[f32]| {
                let _ = speech_tx.send(kept.to_vec());
            })),
        };
        let worker = std::thread::spawn(move || {
            run_consumer(
                16_000,
                Some(Arc::new(Mutex::new(vad))),
                rx,
                None,
                sinks,
                Arc::new(AtomicBool::new(true)),
                Arc::new(AtomicBool::new(false)),
                Arc::new((Mutex::new(false), Condvar::new())),
                worker_speech,
            )
        });

        // Two 30 ms frames of speech, then sixty of silence.
        tx.send(RecorderEvent::Command(Cmd::Start)).unwrap();
        tx.send(RecorderEvent::Samples(vec![0.9; 960])).unwrap();
        tx.send(RecorderEvent::Samples(vec![0.0; 60 * 480]))
            .unwrap();
        let (reply_tx, reply_rx) = mpsc::channel();
        tx.send(RecorderEvent::Command(Cmd::Stop(reply_tx)))
            .unwrap();
        tx.send(RecorderEvent::EndOfStream).unwrap();
        let kept = reply_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(tx);
        worker.join().unwrap();

        // The onset releases both speech frames, then 55 frames of tail.
        assert_eq!(kept.len(), (2 + 55) * 480);
        // The speech sink saw exactly the recording, in order.
        let streamed: Vec<f32> = speech_rx.try_iter().flatten().collect();
        assert_eq!(streamed, kept);
        // The silence clock still starts 15 frames after the last word: 62
        // frames heard, speech through frame 16.
        assert_eq!(speech.quiet_for(), Duration::from_millis((62 - 17) * 30));
    }

    #[test]
    fn detects_access_is_denied() {
        assert!(is_microphone_access_denied("Access is denied"));
    }

    #[test]
    fn detects_permission_denied() {
        assert!(is_microphone_access_denied("permission denied"));
    }

    #[test]
    fn detects_windows_error_code() {
        assert!(is_microphone_access_denied("WASAPI error: 0x80070005"));
    }

    #[test]
    fn does_not_match_unrelated_errors() {
        assert!(!is_microphone_access_denied("device not found"));
    }

    #[test]
    fn detects_no_input_device() {
        assert!(is_no_input_device_error("No input device found"));
    }

    #[test]
    fn detects_coreaudio_config_error() {
        assert!(is_no_input_device_error(
            "Failed to fetch preferred config: A backend-specific error has occurred: An unknown error unknown to the coreaudio-rs API occurred"
        ));
    }

    #[test]
    fn does_not_match_other_errors_for_no_device() {
        assert!(!is_no_input_device_error("permission denied"));
        assert!(!is_no_input_device_error("device not found"));
    }
}

fn run_consumer(
    in_sample_rate: u32,
    vad: Option<Arc<Mutex<Box<dyn vad::VoiceActivityDetector>>>>,
    sample_rx: mpsc::Receiver<RecorderEvent>,
    level_cb: Option<Arc<dyn Fn(Vec<f32>) + Send + Sync + 'static>>,
    sinks: FrameSinks,
    idle: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
    capture_ready: Arc<(Mutex<bool>, Condvar)>,
    speech: Arc<SpeechActivity>,
) {
    let mut frame_resampler = FrameResampler::new(
        in_sample_rate as usize,
        constants::WHISPER_SAMPLE_RATE as usize,
        Duration::from_millis(30),
    );

    let mut processed_samples = Vec::<f32>::new();
    let mut recording = false;
    // Set on Cmd::Start; cleared once we signal readiness for the session.
    let mut awaiting_first_frame = false;

    // ---------- spectrum visualisation setup ---------------------------- //
    const BUCKETS: usize = 16;
    // Scale the FFT window to the device sample rate so the analysis window
    // (~33 ms) and frequency resolution (~30 Hz/bin) stay roughly constant
    // across devices. A fixed 512-sample window collapses the low vocal
    // buckets onto a single bin at 48 kHz (e.g. built-in laptop mics), and
    // would stutter at ~4-8 updates/sec on an 8-16 kHz Bluetooth headset.
    // Targets: 48 kHz -> 2048, 16 kHz -> 512, 8 kHz -> 256.
    let target_window = (f64::from(in_sample_rate) / 30.0).round() as usize;
    let window_size = [256usize, 512, 1024, 2048]
        .into_iter()
        .min_by_key(|w| w.abs_diff(target_window))
        .unwrap();
    let mut visualizer = AudioVisualiser::new(
        in_sample_rate,
        window_size,
        BUCKETS,
        400.0,  // vocal_min_hz
        4000.0, // vocal_max_hz
    );

    /// Hangover frames that still count as speech for [`SpeechActivity`]:
    /// the offline tail, whatever the recording's own tail is. A streaming
    /// model's longer tail keeps more silence in the audio, but must not
    /// push "Stop after silence" a second further out than it has always
    /// been.
    const ACTIVITY_TAIL_FRAMES: usize = vad::VAD_OFFLINE_TAIL_FRAMES;

    fn handle_frame(
        samples: &[f32],
        vad: &Option<Arc<Mutex<Box<dyn vad::VoiceActivityDetector>>>>,
        out_buf: &mut Vec<f32>,
        speech: &SpeechActivity,
        sinks: &FrameSinks,
    ) {
        // Keep silence in the streaming path: the provider uses it to detect
        // speech boundaries independently of the batch VAD.
        if let Some(cb) = &sinks.raw {
            cb(samples);
        }

        // A frame the VAD could not judge is kept, and so is every frame when
        // there is no VAD: both count as speech, so neither can ever end a
        // recording on silence nobody measured.
        let is_speech = if let Some(vad_arc) = vad {
            let mut det = vad_arc.lock().unwrap();
            let kept = match det.push_frame(samples).unwrap_or(VadFrame::Speech(samples)) {
                VadFrame::Speech(buf) => {
                    out_buf.extend_from_slice(buf);
                    if let Some(cb) = &sinks.speech {
                        cb(buf);
                    }
                    true
                }
                VadFrame::Noise => false,
            };
            kept && det.tail_frames() <= ACTIVITY_TAIL_FRAMES
        } else {
            out_buf.extend_from_slice(samples);
            if let Some(cb) = &sinks.speech {
                cb(samples);
            }
            true
        };
        speech.record(samples.len(), is_speech);
    }

    let mut pending_commands = VecDeque::new();
    loop {
        let event = match pending_commands.pop_front() {
            Some(cmd) => RecorderEvent::Command(cmd),
            None => match sample_rx.recv() {
                Ok(event) => event,
                Err(_) => break,
            },
        };
        let cmd = match event {
            RecorderEvent::Command(cmd) => cmd,
            RecorderEvent::EndOfStream => continue,
            RecorderEvent::Samples(raw) => {
                // First real audio frame after a Start means the device is genuinely
                // delivering audio for this session. Signal readiness so the UI/cue can
                // tell the user it's safe to speak (instead of guessing a fixed delay).
                if recording && awaiting_first_frame {
                    let (lock, cvar) = &*capture_ready;
                    *lock.lock().unwrap() = true;
                    cvar.notify_all();
                    awaiting_first_frame = false;
                }

                // A warm microphone still delivers chunks between recordings. Drain
                // them so Start/Stop commands stay responsive, but don't resample or
                // calculate/broadcast a waveform nobody can use. Start resets both
                // processors, so no idle audio is needed for the next recording.
                if recording {
                    if let Some(buckets) = visualizer.feed(&raw) {
                        if let Some(cb) = &level_cb {
                            cb(buckets);
                        }
                    }

                    frame_resampler.push(&raw, &mut |frame: &[f32]| {
                        handle_frame(frame, &vad, &mut processed_samples, &speech, &sinks)
                    });
                }

                continue;
            }
        };
        {
            match cmd {
                Cmd::Start => {
                    stop_flag.store(false, Ordering::Relaxed);
                    idle.store(false, Ordering::Relaxed);
                    processed_samples.clear();
                    // Clear resampler state left over from any previous
                    // recording (partial input chunk, pending output frame, and
                    // the rubato FFT overlap buffers) so stale samples can't
                    // bleed into — and corrupt the first ~30ms of — this one.
                    // Backport of Handy PR #1344 (audio crosstalk).
                    frame_resampler.reset();
                    recording = true;
                    awaiting_first_frame = true;
                    visualizer.reset();
                    if let Some(v) = &vad {
                        v.lock().unwrap().reset();
                    }
                }
                Cmd::Stop(reply_tx) => {
                    recording = false;
                    stop_flag.store(true, Ordering::Relaxed);

                    // Drain all remaining audio until the producer confirms end-of-stream.
                    // The cpal callback sees the stop flag, sends EndOfStream, and goes
                    // silent — guaranteeing every captured sample is in the channel
                    // ahead of the sentinel.
                    //
                    // Feed this tail into the live-transcription callback too (not
                    // just the batch buffer): otherwise the audio captured between
                    // the last steady-state loop iteration and this Stop — plus the
                    // resampler's flushed remainder — never reaches the streaming
                    // worker, so the last words go missing from the live transcript.
                    // The stream router is still open here (finalize_stream() only
                    // closes it after stop_recording() returns), so these frames are
                    // enqueued ahead of the terminal Finalize. Most visible on a
                    // push-to-talk (hold) release, where the user stops the instant
                    // they finish the final word. feed() is a cheap no-op when no
                    // live stream is active.
                    let deadline = std::time::Instant::now() + Duration::from_secs(2);
                    loop {
                        // If the device never delivered a frame, there is no
                        // in-flight audio tail to wait for. Drain anything
                        // already queued, then return empty immediately.
                        let remaining = if awaiting_first_frame {
                            Duration::ZERO
                        } else {
                            deadline.saturating_duration_since(std::time::Instant::now())
                        };
                        match sample_rx.recv_timeout(remaining) {
                            Ok(RecorderEvent::Samples(remaining)) => {
                                frame_resampler.push(&remaining, &mut |frame: &[f32]| {
                                    handle_frame(
                                        frame,
                                        &vad,
                                        &mut processed_samples,
                                        &speech,
                                        &sinks,
                                    )
                                });
                            }
                            Ok(RecorderEvent::EndOfStream) => break,
                            Ok(RecorderEvent::Command(cmd)) => pending_commands.push_back(cmd),
                            Err(_) => {
                                if !awaiting_first_frame {
                                    log::warn!(
                                        "Timed out waiting for EndOfStream from audio callback"
                                    );
                                }
                                break;
                            }
                        }
                    }

                    frame_resampler.finish(&mut |frame: &[f32]| {
                        handle_frame(frame, &vad, &mut processed_samples, &speech, &sinks)
                    });

                    let _ = reply_tx.send(std::mem::take(&mut processed_samples));

                    // Park the callback until the next `Cmd::Start` (idle first,
                    // so it never ships a block between the two stores), then
                    // clear the stop flag as before. The consumer discards
                    // anything between recordings, so letting the callback keep
                    // shipping blocks only cost a copy and two thread wakeups
                    // per audio period (forever, in always-on microphone mode).
                    idle.store(true, Ordering::Relaxed);
                    stop_flag.store(false, Ordering::Relaxed);
                }
                Cmd::Shutdown => {
                    stop_flag.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
    }
}
