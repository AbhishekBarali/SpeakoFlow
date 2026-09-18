//! System-audio ("loopback") capture — the *other* side of a call.
//!
//! The microphone path in [`super::recorder`] hears only the user. A meeting
//! recorder additionally needs what the computer is *playing*: the remote
//! participants. That is a different OS facility on every platform, and this
//! module is the whole of that difference.
//!
//! **The output contract is identical to the microphone path**: 16 kHz mono
//! `f32` frames of 30 ms, delivered to a callback. Everything downstream
//! (VAD, transcription, storage) therefore treats the two sources the same and
//! needs no knowledge of where the audio came from.
//!
//! # Why this stays a separate stream
//!
//! It would be less code to mix system audio into the microphone stream and
//! transcribe one track. Doing that throws away the single most valuable piece
//! of speaker information available, for free and at perfect accuracy: **which
//! device the samples arrived on**. Microphone means the user; loopback means
//! everyone else. No model can be wrong about that, and no amount of speaker
//! diarization recovers it once the two are summed. So the two streams stay
//! separate all the way to storage, and "who is me" is a channel fact rather
//! than an inference. See `docs/meeting-notes-plan.md`, invariant 1.
//!
//! # Platform support
//!
//! | Platform | Mechanism | Status |
//! |---|---|---|
//! | Windows | WASAPI loopback on the default render endpoint | native |
//! | Linux | PulseAudio/PipeWire `.monitor` source (a normal capture device) | native |
//! | macOS | a virtual loopback device (BlackHole et al.), if installed | partial |
//!
//! Windows and Linux need nothing installed. macOS is the gap: Core Audio
//! process taps (macOS 14.2+) are the correct native answer and landed in
//! `cpal` 0.17, but this crate is pinned to 0.16 and bumping it would put the
//! *microphone* path at risk to fix the loopback path. Until that bump happens
//! deliberately, macOS resolves a virtual audio device if the user has one and
//! otherwise reports [`LoopbackError::Unsupported`] with an actionable message.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::FrameResampler;
use crate::audio_toolkit::constants;

/// Length of one delivered frame. Matches the microphone path so a single
/// downstream consumer can accept either source.
const FRAME_DURATION: Duration = Duration::from_millis(30);

/// Why loopback capture could not start.
///
/// These are deliberately distinct because the *caller* must say different
/// things to the user for each. Silently degrading a meeting recording to
/// microphone-only is the one behaviour to avoid: the user would come back to a
/// transcript containing only their own half of the conversation.
#[derive(Debug, Clone)]
pub enum LoopbackError {
    /// No mechanism exists on this platform/configuration. Carries a message
    /// written for the user, not the log.
    Unsupported(String),
    /// The OS refused. On macOS this is the overwhelmingly likely failure and
    /// has a specific, non-obvious cause — see [`MACOS_PERMISSION_HELP`].
    PermissionDenied(String),
    /// The mechanism exists but no usable endpoint was found (e.g. no active
    /// output device).
    NoDevice(String),
    /// Anything else, with the underlying error preserved.
    Failed(String),
}

impl std::fmt::Display for LoopbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(m) => write!(f, "{m}"),
            Self::PermissionDenied(m) => write!(f, "{m}"),
            Self::NoDevice(m) => write!(f, "{m}"),
            Self::Failed(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for LoopbackError {}

/// The macOS permission trap, spelled out because it produces a *silent*
/// failure that looks like a bug in this app.
///
/// macOS exposes two separate permissions: "Screen Recording and System Audio"
/// and "System Audio Only". Holding the former without the latter means the
/// permission dialog **never appears** and the capture returns an unbroken
/// stream of digital silence. Screenpipe concluded upstream `cpal` loopback
/// "didn't work" for exactly this reason. Anything that captures system audio
/// on macOS must state this rather than let the user discover an empty
/// recording after their meeting.
pub const MACOS_PERMISSION_HELP: &str = "macOS needs the \"System Audio Only\" permission, which is separate from \"Screen Recording and System Audio\". Open System Settings > Privacy & Security > Screen & System Audio Recording and add SpeakoFlow under System Audio Only. Without it macOS records silence without ever asking.";

/// A running loopback capture. Dropping this stops the capture.
pub struct LoopbackStream {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    /// Human-readable description of what is being captured, for logs and UI.
    source_name: String,
}

impl LoopbackStream {
    /// What this stream is capturing (endpoint or device name).
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    /// Stop capturing and wait for the worker to finish.
    ///
    /// Joining matters: the worker owns COM state and an audio client, and a
    /// meeting that stops and immediately restarts must not race two capture
    /// threads against one endpoint.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            if let Err(e) = handle.join() {
                log::warn!("Loopback worker panicked on shutdown: {e:?}");
            }
        }
    }
}

impl Drop for LoopbackStream {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Whether this build/platform can capture system audio at all.
///
/// Cheap enough to call from settings UI to decide whether to offer the
/// feature. A `true` here does not guarantee [`start_loopback`] succeeds —
/// permissions are only knowable by trying.
pub fn loopback_supported() -> bool {
    #[cfg(target_os = "windows")]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        find_monitor_source().is_some()
    }
    #[cfg(target_os = "macos")]
    {
        find_virtual_loopback_device().is_some()
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        false
    }
}

/// Begin capturing system audio, delivering 16 kHz mono 30 ms frames to `cb`.
///
/// The callback runs on the capture thread and must not block: anything slow
/// belongs behind a channel. It is called only while audio is flowing, so a
/// silent system produces silent frames rather than no frames — downstream
/// chunking depends on seeing the silence.
pub fn start_loopback<F>(cb: F) -> Result<LoopbackStream, LoopbackError>
where
    F: FnMut(&[f32]) + Send + 'static,
{
    #[cfg(target_os = "windows")]
    {
        windows_impl::start(cb)
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        cpal_device_impl::start(cb)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        let _ = cb;
        Err(LoopbackError::Unsupported(
            "System audio capture is not supported on this platform.".into(),
        ))
    }
}

/// Begin capturing the **microphone** with the same no-accumulation contract as
/// [`start_loopback`], delivering 16 kHz mono 30 ms frames to `cb`.
///
/// This exists instead of reusing [`super::AudioRecorder`] for one reason:
/// `AudioRecorder` accumulates the whole recording in memory so it can return
/// it from `stop()`, which is correct for a dictation measured in seconds and
/// ruinous for a meeting measured in hours — two hours of 16 kHz mono `f32` is
/// roughly 460 MB resident. A meeting streams to disk and to the transcriber
/// instead, so it needs a capture that holds nothing.
///
/// Passing `None` uses the default input device.
pub fn start_mic_capture<F>(
    device: Option<cpal::Device>,
    cb: F,
) -> Result<LoopbackStream, LoopbackError>
where
    F: FnMut(&[f32]) + Send + 'static,
{
    use cpal::traits::{DeviceTrait, HostTrait};

    let device = match device {
        Some(d) => d,
        None => crate::audio_toolkit::get_cpal_host()
            .default_input_device()
            .ok_or_else(|| {
                LoopbackError::NoDevice("No microphone available to record from.".into())
            })?,
    };
    let name = device.name().unwrap_or_else(|_| "Microphone".to_string());
    cpal_device_impl::start_on_device(device, name, cb)
}

/// Fold an interleaved multi-channel frame down to one mono sample.
///
/// Averaging **only the first two channels** rather than all of them is
/// deliberate. Multi-channel endpoints (5.1/7.1 render mixes, and microphone
/// arrays on the capture side) carry auxiliary channels — LFE, surrounds,
/// beam-forming residue — that are not simply "more of the same signal".
/// Averaging those in attenuates the actual dialogue and, for array mics, can
/// destructively interfere with it. Meetily hit exactly this and fixed it the
/// same way.
#[inline]
fn downmix(frame: &[f32]) -> f32 {
    match frame.len() {
        0 => 0.0,
        1 => frame[0],
        _ => (frame[0] + frame[1]) * 0.5,
    }
}

/// Convert an interleaved buffer of `channels`-wide frames to mono.
fn interleaved_to_mono(src: &[f32], channels: usize, out: &mut Vec<f32>) {
    out.clear();
    if channels <= 1 {
        out.extend_from_slice(src);
        return;
    }
    out.reserve(src.len() / channels + 1);
    for frame in src.chunks_exact(channels) {
        out.push(downmix(frame));
    }
}

/* ───────────────────────────── Windows ───────────────────────────── */

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    /// WASAPI buffer request, in 100 ns units. 200 ms is generous on purpose:
    /// this is a background capture competing with a video call for CPU, and an
    /// overrun costs audio we cannot get back. Latency is irrelevant here
    /// because nothing is monitoring this stream live.
    const BUFFER_DURATION_100NS: i64 = 200 * 10_000;

    /// How long to sleep when the endpoint has no packet ready. Small enough
    /// that the buffer cannot fill during a scheduling gap, large enough not to
    /// spin a core for the length of a meeting.
    const IDLE_POLL: Duration = Duration::from_millis(5);

    pub fn start<F>(mut cb: F) -> Result<LoopbackStream, LoopbackError>
    where
        F: FnMut(&[f32]) + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_worker = stop.clone();

        // Resolving the endpoint name happens on the worker (COM lives there),
        // so the name is reported back through a rendezvous channel before the
        // capture loop begins. That also lets `start` fail synchronously with a
        // real error rather than returning a stream that dies immediately.
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<Result<String, LoopbackError>>(1);

        let handle = std::thread::Builder::new()
            .name("loopback-wasapi".into())
            .spawn(move || {
                unsafe {
                    // COM must be initialised on this thread. If some other
                    // component already did it with a compatible model this is
                    // a no-op; we deliberately ignore the "already initialised"
                    // HRESULT rather than treat it as failure.
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                    let result = run_capture(&stop_for_worker, &init_tx, &mut cb);
                    if let Err(e) = result {
                        // If we never got as far as reporting readiness, the
                        // error travels through the init channel instead.
                        let _ = init_tx.try_send(Err(e.clone()));
                        log::error!("WASAPI loopback capture ended with error: {e}");
                    }
                    CoUninitialize();
                }
            })
            .map_err(|e| LoopbackError::Failed(format!("Failed to spawn loopback thread: {e}")))?;

        // Bounded wait: a wedged COM call must not hang meeting startup.
        match init_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(source_name)) => {
                log::info!("System audio capture started on '{source_name}'");
                Ok(LoopbackStream {
                    stop,
                    handle: Some(handle),
                    source_name,
                })
            }
            Ok(Err(e)) => {
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                Err(LoopbackError::Failed(
                    "Timed out starting system audio capture.".into(),
                ))
            }
        }
    }

    unsafe fn run_capture<F>(
        stop: &AtomicBool,
        init_tx: &std::sync::mpsc::SyncSender<Result<String, LoopbackError>>,
        cb: &mut F,
    ) -> Result<(), LoopbackError>
    where
        F: FnMut(&[f32]) + Send + 'static,
    {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| {
                LoopbackError::Failed(format!("Could not open the audio device enumerator: {e}"))
            })?;

        // eConsole, not eMultimedia: we want the endpoint the user is actually
        // listening to for a call, which is what Windows routes "console"
        // (communications-adjacent, default) audio to.
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| {
                LoopbackError::NoDevice(format!(
                    "No active audio output device to capture from: {e}"
                ))
            })?;

        let source_name = endpoint_name(&device);

        let client: IAudioClient =
            device
                .Activate::<IAudioClient>(CLSCTX_ALL, None)
                .map_err(|e| {
                    LoopbackError::Failed(format!("Could not activate the audio client: {e}"))
                })?;

        // Shared-mode mix format. This is what the endpoint is actually mixing
        // at, so no format negotiation is needed or permitted in loopback.
        let mix_format = client
            .GetMixFormat()
            .map_err(|e| LoopbackError::Failed(format!("Could not read the mix format: {e}")))?;
        if mix_format.is_null() {
            return Err(LoopbackError::Failed(
                "Audio endpoint reported no mix format.".into(),
            ));
        }

        let channels = (*mix_format).nChannels as usize;
        let sample_rate = (*mix_format).nSamplesPerSec;
        let bits = (*mix_format).wBitsPerSample;
        let block_align = (*mix_format).nBlockAlign as usize;

        log::info!("WASAPI loopback: '{source_name}' {sample_rate} Hz, {channels} ch, {bits}-bit");

        // Shared-mode mix formats are 32-bit float in practice on every
        // supported Windows version; 16-bit integer is accepted as a defensive
        // fallback. Deciding on `wBitsPerSample` alone avoids having to unpack
        // WAVEFORMATEXTENSIBLE and compare subformat GUIDs for no practical
        // gain.
        let sample_kind = match bits {
            32 => SampleKind::F32,
            16 => SampleKind::I16,
            other => {
                return Err(LoopbackError::Failed(format!(
                    "Unsupported system audio sample size: {other}-bit"
                )));
            }
        };

        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                BUFFER_DURATION_100NS,
                0,
                mix_format,
                None,
            )
            .map_err(|e| {
                // A loopback initialise failing on an otherwise healthy endpoint
                // is nearly always an exclusive-mode holder (some DAWs, some
                // game audio engines) owning the device.
                LoopbackError::Failed(format!(
                    "Could not start loopback on this output device (another app may have exclusive control of it): {e}"
                ))
            })?;

        let capture: IAudioCaptureClient = client.GetService().map_err(|e| {
            LoopbackError::Failed(format!("Could not obtain the capture service: {e}"))
        })?;

        client
            .Start()
            .map_err(|e| LoopbackError::Failed(format!("Could not start the audio client: {e}")))?;

        // Past this point failures are logged and end the capture rather than
        // being reported to the caller, who has already been told we started.
        let _ = init_tx.try_send(Ok(source_name));

        let mut resampler = FrameResampler::new(
            sample_rate as usize,
            constants::WHISPER_SAMPLE_RATE as usize,
            FRAME_DURATION,
        );
        let mut mono = Vec::<f32>::new();
        let mut scratch = Vec::<f32>::new();

        while !stop.load(Ordering::Relaxed) {
            let packet_frames = match capture.GetNextPacketSize() {
                Ok(n) => n,
                Err(e) => {
                    log::warn!("Loopback GetNextPacketSize failed: {e}");
                    break;
                }
            };

            if packet_frames == 0 {
                std::thread::sleep(IDLE_POLL);
                continue;
            }

            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;

            if let Err(e) = capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) {
                log::warn!("Loopback GetBuffer failed: {e}");
                break;
            }

            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
            let sample_count = frames as usize * channels;

            scratch.clear();
            if silent || data.is_null() {
                // WASAPI is allowed to hand back a silent marker with no data.
                // Emitting real zeros keeps the timeline continuous, which is
                // what lets a transcriber find chunk boundaries and what keeps
                // segment timestamps aligned with wall-clock meeting time.
                scratch.resize(sample_count, 0.0);
            } else {
                debug_assert!(block_align == 0 || block_align == channels * (bits as usize / 8));
                match sample_kind {
                    SampleKind::F32 => {
                        let src = std::slice::from_raw_parts(data as *const f32, sample_count);
                        scratch.extend_from_slice(src);
                    }
                    SampleKind::I16 => {
                        let src = std::slice::from_raw_parts(data as *const i16, sample_count);
                        scratch.extend(src.iter().map(|s| *s as f32 / i16::MAX as f32));
                    }
                }
            }

            // Release before doing any work: the endpoint buffer is a shared
            // resource and holding it across a resample invites an overrun.
            if let Err(e) = capture.ReleaseBuffer(frames) {
                log::warn!("Loopback ReleaseBuffer failed: {e}");
                break;
            }

            interleaved_to_mono(&scratch, channels, &mut mono);
            resampler.push(&mono, &mut |frame: &[f32]| cb(frame));
        }

        let _ = client.Stop();
        resampler.finish(&mut |frame: &[f32]| cb(frame));
        log::info!("System audio capture stopped");
        Ok(())
    }

    enum SampleKind {
        F32,
        I16,
    }

    /// Label for the captured endpoint.
    ///
    /// Deliberately a constant rather than the endpoint's friendly name:
    /// reading that requires `PKEY_Device_FriendlyName` from the
    /// `Win32_Devices_FunctionDiscovery` namespace, which this crate does not
    /// enable, and the name is purely cosmetic. Adding a whole windows-rs
    /// feature to put "Speakers (Realtek)" in a log line is not worth the
    /// compile-time cost.
    unsafe fn endpoint_name(_device: &windows::Win32::Media::Audio::IMMDevice) -> String {
        "System audio".to_string()
    }
}

/* ─────────────────────── cpal-based device capture ─────────────────────── */

/// Capture from an ordinary `cpal` input device.
///
/// Used for two different jobs, which is why it is not named after either:
///
/// * the **microphone**, on every platform ([`start_mic_capture`]);
/// * **loopback** on Linux and macOS, where the system mix is presented as a
///   normal capture device — a PulseAudio/PipeWire `.monitor` source, or a
///   virtual driver such as BlackHole. Windows loopback does not come through
///   here; it uses WASAPI directly.
mod cpal_device_impl {
    use super::*;
    use cpal::traits::{DeviceTrait, StreamTrait};

    /// Resolve a loopback-capable input device, then capture from it.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn start<F>(cb: F) -> Result<LoopbackStream, LoopbackError>
    where
        F: FnMut(&[f32]) + Send + 'static,
    {
        #[cfg(target_os = "linux")]
        let (device, source_name) = super::find_monitor_source().ok_or_else(|| {
            LoopbackError::Unsupported(
                "No audio monitor source found. PulseAudio or PipeWire provides one; on a bare ALSA setup there is nothing to capture system audio from.".into(),
            )
        })?;

        #[cfg(target_os = "macos")]
        let (device, source_name) = super::find_virtual_loopback_device().ok_or_else(|| {
            LoopbackError::Unsupported(format!(
                "macOS cannot capture system audio without a virtual audio device. Install BlackHole (free) and select it, or route the call through a multi-output device. {}",
                MACOS_PERMISSION_HELP
            ))
        })?;

        start_on_device(device, source_name, cb)
    }

    /// Capture from an already-resolved device.
    pub fn start_on_device<F>(
        device: cpal::Device,
        source_name: String,
        mut cb: F,
    ) -> Result<LoopbackStream, LoopbackError>
    where
        F: FnMut(&[f32]) + Send + 'static,
    {
        let config = device.default_input_config().map_err(|e| {
            LoopbackError::Failed(format!("Could not read the capture device config: {e}"))
        })?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;

        log::info!(
            "Capturing '{source_name}': {sample_rate} Hz, {channels} ch, {:?}",
            config.sample_format()
        );

        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_worker = stop.clone();
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel::<Result<(), LoopbackError>>(1);

        // cpal streams are `!Send` on some backends, so the stream is built and
        // owned entirely on this worker thread rather than returned to the
        // caller.
        let handle = std::thread::Builder::new()
            .name("meeting-capture".into())
            .spawn(move || {
                let mut resampler = FrameResampler::new(
                    sample_rate as usize,
                    constants::WHISPER_SAMPLE_RATE as usize,
                    FRAME_DURATION,
                );
                let mut mono = Vec::<f32>::new();

                let err_fn = |e| log::warn!("Capture stream error: {e}");

                let stream = match config.sample_format() {
                    cpal::SampleFormat::F32 => device.build_input_stream(
                        &config.clone().into(),
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            interleaved_to_mono(data, channels, &mut mono);
                            resampler.push(&mono, &mut |frame: &[f32]| cb(frame));
                        },
                        err_fn,
                        None,
                    ),
                    cpal::SampleFormat::I16 => {
                        let mut buf = Vec::<f32>::new();
                        device.build_input_stream(
                            &config.clone().into(),
                            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                                buf.clear();
                                buf.extend(data.iter().map(|s| *s as f32 / i16::MAX as f32));
                                interleaved_to_mono(&buf, channels, &mut mono);
                                resampler.push(&mono, &mut |frame: &[f32]| cb(frame));
                            },
                            err_fn,
                            None,
                        )
                    }
                    other => {
                        let _ = init_tx.try_send(Err(LoopbackError::Failed(format!(
                            "Unsupported capture sample format: {other:?}"
                        ))));
                        return;
                    }
                };

                let stream = match stream {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = init_tx.try_send(Err(classify_cpal_error(&e.to_string())));
                        return;
                    }
                };

                if let Err(e) = stream.play() {
                    let _ = init_tx.try_send(Err(classify_cpal_error(&e.to_string())));
                    return;
                }

                let _ = init_tx.try_send(Ok(()));

                // Hold the stream alive; the callback does the work.
                while !stop_for_worker.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                }

                // Pause before drop so callbacks stop firing immediately rather
                // than racing teardown. Meetily's microphone stayed live after
                // "stop" for exactly the want of this.
                let _ = stream.pause();
                drop(stream);
                log::info!("Capture stopped");
            })
            .map_err(|e| LoopbackError::Failed(format!("Failed to spawn capture thread: {e}")))?;

        match init_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(LoopbackStream {
                stop,
                handle: Some(handle),
                source_name,
            }),
            Ok(Err(e)) => {
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                let _ = handle.join();
                Err(LoopbackError::Failed(
                    "Timed out starting audio capture.".into(),
                ))
            }
        }
    }

    /// Map a backend error string onto a cause the user can act on.
    ///
    /// On macOS a permission failure and a genuine device failure are
    /// indistinguishable in the returned error, and the permission case is far
    /// more likely, so it wins the tie and carries the explanation.
    fn classify_cpal_error(msg: &str) -> LoopbackError {
        let lowered = msg.to_lowercase();
        if lowered.contains("denied")
            || lowered.contains("permission")
            || lowered.contains("not authorized")
            || lowered.contains("unauthorized")
        {
            #[cfg(target_os = "macos")]
            return LoopbackError::PermissionDenied(format!("{msg}. {MACOS_PERMISSION_HELP}"));
            #[cfg(not(target_os = "macos"))]
            return LoopbackError::PermissionDenied(msg.to_string());
        }
        LoopbackError::Failed(msg.to_string())
    }
}

/// Find a PulseAudio/PipeWire monitor source.
///
/// Monitor sources are named `<sink>.monitor` and are surfaced by the ALSA/Pulse
/// bridge as ordinary capture devices, which is why Linux needs no special API
/// here. Preference order puts an explicit `.monitor` suffix ahead of a fuzzy
/// name match so we do not accidentally select a physical device that merely
/// has "monitor" in its product name.
#[cfg(target_os = "linux")]
fn find_monitor_source() -> Option<(cpal::Device, String)> {
    use cpal::traits::DeviceTrait;

    let devices = super::list_input_devices().ok()?;

    let exact = devices
        .iter()
        .find(|d| d.name.ends_with(".monitor"))
        .or_else(|| devices.iter().find(|d| d.name.contains("monitor")))
        .or_else(|| {
            devices
                .iter()
                .find(|d| d.name.to_lowercase().contains("loopback"))
        })?;

    let name = exact.device.name().unwrap_or_else(|_| exact.name.clone());
    Some((exact.device.clone(), name))
}

/// Find an installed virtual audio device on macOS.
///
/// Ordered by how likely the device is to be a *loopback* rather than something
/// that merely sounds like one. BlackHole is first because it is the free,
/// widely recommended option for exactly this job.
#[cfg(target_os = "macos")]
fn find_virtual_loopback_device() -> Option<(cpal::Device, String)> {
    use cpal::traits::DeviceTrait;

    /// Substrings of known virtual-audio driver names, most-preferred first.
    const KNOWN: &[&str] = &[
        "blackhole",
        "loopback audio",
        "existential audio",
        "soundflower",
        "ishowu",
        "vb-cable",
        "multi-output",
        "aggregate",
    ];

    let devices = super::list_input_devices().ok()?;

    for needle in KNOWN {
        if let Some(found) = devices
            .iter()
            .find(|d| d.name.to_lowercase().contains(needle))
        {
            let name = found.device.name().unwrap_or_else(|_| found.name.clone());
            return Some((found.device.clone(), name));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_handles_channel_counts() {
        assert_eq!(downmix(&[]), 0.0);
        assert_eq!(downmix(&[0.5]), 0.5);
        assert_eq!(downmix(&[1.0, 0.0]), 0.5);
    }

    /// Auxiliary channels must not drag the dialogue down. A 5.1 frame whose
    /// front pair is full-scale should stay full-scale even when the surrounds
    /// and LFE are silent — averaging all six channels would report 0.33 and
    /// quietly attenuate every multi-channel meeting.
    #[test]
    fn downmix_ignores_auxiliary_channels() {
        let five_one = [1.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(downmix(&five_one), 1.0);
    }

    #[test]
    fn interleaved_to_mono_collapses_stereo() {
        let mut out = Vec::new();
        interleaved_to_mono(&[1.0, 0.0, 0.0, 1.0], 2, &mut out);
        assert_eq!(out, vec![0.5, 0.5]);
    }

    #[test]
    fn interleaved_to_mono_passes_mono_through() {
        let mut out = Vec::new();
        interleaved_to_mono(&[0.1, 0.2, 0.3], 1, &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }

    /// A partial trailing frame is dropped rather than emitted half-formed:
    /// `chunks_exact` guarantees every produced sample came from a complete
    /// interleaved frame, so a truncated buffer cannot shift channel alignment
    /// for everything after it.
    #[test]
    fn interleaved_to_mono_ignores_partial_trailing_frame() {
        let mut out = Vec::new();
        interleaved_to_mono(&[1.0, 1.0, 0.5], 2, &mut out);
        assert_eq!(out, vec![1.0]);
    }

    #[test]
    fn errors_render_their_user_facing_message() {
        let e = LoopbackError::Unsupported("no device here".into());
        assert_eq!(e.to_string(), "no device here");
    }

    /// The macOS help text is load-bearing, not decoration: it names the second
    /// permission that otherwise fails silently.
    #[test]
    fn macos_help_names_the_silent_permission() {
        assert!(MACOS_PERMISSION_HELP.contains("System Audio Only"));
    }
}
