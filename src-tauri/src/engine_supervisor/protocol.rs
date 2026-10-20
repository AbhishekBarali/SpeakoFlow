//! Wire protocol between SpeakoFlow and its transcribe.cpp worker process.
//!
//! Strict request/response: every [`Request`] gets exactly one [`Response`],
//! except [`Request::Cancel`], which is answered by the request it cancels
//! ending early. Each message is one frame:
//! `[u32 json_len][json][u32 pcm_len][pcm bytes]` (lengths little-endian, PCM as
//! little-endian f32). JSON rather than a binary format because transcribe.cpp's
//! serde impls round-trip NaN confidences through `null`, which only works with
//! a self-describing format; the audio, which is nearly all of the bytes, never
//! goes through JSON.
//!
//! Ported from upstream Handy's `engine_supervisor/protocol.rs` (#2208), with
//! SpeakoFlow's load target, request ids for cancellation, and structured
//! transcribe.cpp errors added.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use transcribe_cpp::{
    Capabilities, DeviceType, ErrorKind, RunOptions, StreamOptions, StreamText, StreamUpdate,
    Transcript,
};

/// Refuse frames larger than this: a garbage length must not turn into a
/// multi-gigabyte allocation. One hour of 16 kHz f32 PCM is ~230 MB.
const MAX_SECTION_BYTES: usize = 1 << 30;
const PCM_BYTES_PER_MINUTE: usize = 16_000 * 4 * 60;
const F32_BYTES: usize = std::mem::size_of::<f32>();

#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    /// First request to every worker; answered once backend init is done
    /// (with an error if it registered no compute device). With
    /// `list_devices`, the worker also enumerates its compute devices.
    Hello {
        list_devices: bool,
    },
    Load {
        path: PathBuf,
        target: LoadTarget,
    },
    /// Followed by the PCM to transcribe. `id` names the run for
    /// [`Request::Cancel`].
    Run {
        id: u64,
        options: RunOptions,
    },
    StreamBegin {
        id: u64,
        run: RunOptions,
        stream: StreamOptions,
    },
    /// Followed by the PCM frame to feed.
    Feed,
    /// `want_language`: also report the model's detected language (costs a
    /// transcript snapshot, so only asked for when it can change the outcome).
    Finalize {
        want_language: bool,
    },
    StreamReset,
    /// Stop the run or stream `id` at transcribe.cpp's next abort check. Read
    /// by the worker's input thread, never queued behind the work it stops,
    /// and never answered: the cancelled request answers instead, early.
    Cancel {
        id: u64,
    },
}

/// Where a model load should run, decided by the app from the settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoadTarget {
    /// The processor only.
    Cpu,
    /// transcribe.cpp's automatic policy: the first GPU that initializes,
    /// dedicated GPUs before integrated ones, else the processor.
    Auto,
    /// The GPU with this stable key ([`device_key`]), if it is still present;
    /// otherwise the automatic policy. Device handles are process-local, so
    /// the worker resolves the key against its own registry.
    Device(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    /// `devices` is present only when the hello asked for them.
    Hello {
        devices: Option<Vec<DeviceInfo>>,
    },
    Loaded(LoadedInfo),
    Transcript(Transcript),
    Fed {
        update: StreamUpdate,
        /// Present only when the committed or tentative text changed.
        text: Option<StreamText>,
    },
    /// The end of a stream. A finalize that transcribe.cpp failed still
    /// reports the text the stream had committed (`error` says why), so a
    /// stream that worked until its last call never loses the user's words.
    Finalized {
        update: Option<StreamUpdate>,
        text: StreamText,
        language: Option<String>,
        error: Option<String>,
    },
    Ok,
    /// transcribe.cpp refused or failed the request; the worker is fine.
    /// `kind` lets the app tell "too short to decode" and "aborted" apart
    /// from a real failure without matching on message text.
    Failed {
        kind: ErrorKind,
        message: String,
    },
    /// The file loaded, but it is not a speech-to-text model (a diarizer or
    /// language-ID GGUF reports no ASR capabilities).
    NotSpeechModel(String),
    /// The request made no sense in the worker's state (no model, a stream
    /// already active); the worker is fine.
    Error(String),
}

impl Response {
    /// A transcribe.cpp error as it crosses the process boundary.
    pub fn failed(error: &transcribe_cpp::Error) -> Self {
        Response::Failed {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

/// Serializable mirror of `transcribe_cpp::Device` (which holds a
/// process-local handle and so has no serde impl).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub description: String,
    pub kind: String,
    pub device_type: DeviceType,
    pub device_id: Option<String>,
    pub memory_total: u64,
    pub index: Option<usize>,
    /// Stable identity for persisting a device choice; see [`device_key`].
    pub key: String,
}

impl DeviceInfo {
    pub fn from_device(device: &transcribe_cpp::Device) -> Self {
        Self {
            name: device.name.clone(),
            description: device.description.clone(),
            kind: device.kind.clone(),
            device_type: device.device_type,
            device_id: device.device_id.clone(),
            memory_total: device.memory_total,
            index: device.index,
            key: device_key(&device.kind, device.device_id.as_deref(), &device.name),
        }
    }

    /// The human name of a device: the adapter description when there is one.
    pub fn label(&self) -> &str {
        if self.description.is_empty() {
            &self.name
        } else {
            &self.description
        }
    }

    /// Whether the device is a GPU, dedicated or integrated. CPU, host-memory
    /// accelerators (BLAS) and devices a newer runtime reports in a class this
    /// binding does not know cannot host a model on their own.
    pub fn is_gpu(&self) -> bool {
        matches!(self.device_type, DeviceType::Gpu | DeviceType::Igpu)
    }
}

/// Persistent device identity: the backend's stable `device_id` (the PCI bus id
/// on Vulkan) where it reports one, its name otherwise (Metal reports none).
/// The same encoding upstream Handy stores and older SpeakoFlow versions saved
/// as `transcribe_gpu_device`, so it must stay byte-identical.
pub fn device_key(kind: &str, device_id: Option<&str>, name: &str) -> String {
    let (identity_kind, identity) = match device_id {
        Some(device_id) => ("id", device_id),
        None => ("name", name),
    };
    // Serializing a tuple of three strings cannot fail; the fallback only
    // keeps this function total.
    serde_json::to_string(&(kind, identity_kind, identity))
        .unwrap_or_else(|_| format!("{kind}:{identity_kind}:{identity}"))
}

/// What the app needs to know about a loaded model, read once at load. Every
/// decision the app used to make by asking the in-process `Model` is made from
/// this instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadedInfo {
    pub arch: String,
    pub variant: String,
    pub backend: String,
    /// Label of the device the model actually bound to.
    pub device: String,
    /// That device's class, for the log; `None` when transcribe.cpp did not
    /// report the device.
    pub device_type: Option<DeviceType>,
    /// Whether that device is a GPU (a crash there may be the GPU's fault).
    pub on_gpu: bool,
    pub capabilities: Capabilities,
    pub supports_initial_prompt: bool,
    /// Whether the model handles audio longer than its native window in one
    /// run (Whisper today); other models' long audio is chunked by the app.
    pub supports_long_form: bool,
    pub supports_vocabulary: bool,
    /// Whether the model honours an abort request mid-run. Without it a
    /// cancel can only stop the work by stopping the worker.
    pub supports_cancellation: bool,
    /// The model streams through Parakeet's buffered (chunked attention with
    /// right context) extension, which needs silence fed after the last word
    /// before it finalizes.
    pub buffered_stream: bool,
    /// CPU threads the session was opened with; 0 means every logical CPU.
    pub cpu_threads: i32,
}

/// Encode one frame. Refuses a section the reader would refuse, so an
/// oversized request fails cleanly here instead of killing the worker (which
/// would read as a crash and be blamed on the GPU).
pub fn encode_message<T: Serialize>(message: &T, pcm: Option<&[f32]>) -> io::Result<Vec<u8>> {
    let json = serde_json::to_vec(message).map_err(io::Error::other)?;
    check_section_len(json.len())?;
    let pcm = pcm.unwrap_or(&[]);
    let pcm_len = std::mem::size_of_val(pcm);
    if check_section_len(pcm_len).is_err() {
        return Err(io::Error::other(format!(
            "audio too long for one transcription: {} minutes (limit {} minutes)",
            pcm_len / PCM_BYTES_PER_MINUTE,
            MAX_SECTION_BYTES / PCM_BYTES_PER_MINUTE,
        )));
    }
    let mut frame = Vec::with_capacity(8 + json.len() + pcm_len);
    frame.extend_from_slice(&(json.len() as u32).to_le_bytes());
    frame.extend_from_slice(&json);
    frame.extend_from_slice(&(pcm_len as u32).to_le_bytes());
    extend_with_pcm(&mut frame, pcm);
    Ok(frame)
}

pub fn write_message<T: Serialize>(
    w: &mut impl Write,
    message: &T,
    pcm: Option<&[f32]>,
) -> io::Result<()> {
    w.write_all(&encode_message(message, pcm)?)?;
    w.flush()
}

/// Read one frame. `Ok(None)` only on a clean EOF before the first byte of a
/// frame; a frame cut off part-way is an error.
pub fn read_message<T: for<'de> Deserialize<'de>>(
    r: &mut impl Read,
) -> io::Result<Option<(T, Vec<f32>)>> {
    let mut len = [0u8; 4];
    let mut filled = 0;
    while filled < len.len() {
        match r.read(&mut len[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let json = read_section_body(r, len)?;
    let message = serde_json::from_slice(&json).map_err(io::Error::other)?;
    let mut pcm_len = [0u8; 4];
    r.read_exact(&mut pcm_len)?;
    let pcm_len = u32::from_le_bytes(pcm_len) as usize;
    check_section_len(pcm_len)?;
    if !pcm_len.is_multiple_of(F32_BYTES) {
        return Err(io::Error::other(
            "PCM section is not a whole number of f32s",
        ));
    }
    let pcm = read_pcm(r, pcm_len / F32_BYTES)?;
    Ok(Some((message, pcm)))
}

fn read_section_body(r: &mut impl Read, len: [u8; 4]) -> io::Result<Vec<u8>> {
    let len = u32::from_le_bytes(len) as usize;
    check_section_len(len)?;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// Also keeps every section length within the frame's `u32` length prefix.
fn check_section_len(len: usize) -> io::Result<()> {
    if len > MAX_SECTION_BYTES {
        return Err(io::Error::other(format!("frame section too large: {len}")));
    }
    Ok(())
}

/// Append `pcm` as little-endian f32 bytes.
///
/// On a little-endian machine (every platform SpeakoFlow ships on) that is the
/// samples' own memory, copied in one `memcpy`. A per-sample loop costs tens
/// of milliseconds for a long recording in a dev build, where this crate is
/// compiled without optimisation.
fn extend_with_pcm(frame: &mut Vec<u8>, pcm: &[f32]) {
    #[cfg(target_endian = "little")]
    {
        // SAFETY: `f32` has no padding and no invalid bit patterns as bytes,
        // `u8` has alignment 1, and the slice covers exactly `pcm`'s memory.
        let bytes = unsafe {
            std::slice::from_raw_parts(pcm.as_ptr().cast::<u8>(), std::mem::size_of_val(pcm))
        };
        frame.extend_from_slice(bytes);
    }
    #[cfg(not(target_endian = "little"))]
    for sample in pcm {
        frame.extend_from_slice(&sample.to_le_bytes());
    }
}

/// Read `samples` little-endian f32s straight into the vector they end up in.
fn read_pcm(r: &mut impl Read, samples: usize) -> io::Result<Vec<f32>> {
    let mut pcm = vec![0f32; samples];
    #[cfg(target_endian = "little")]
    {
        // SAFETY: every bit pattern is a valid `f32`, so writing arbitrary
        // bytes into the vector's initialized memory is sound; the slice
        // covers exactly that memory.
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(
                pcm.as_mut_ptr().cast::<u8>(),
                std::mem::size_of_val(pcm.as_slice()),
            )
        };
        r.read_exact(bytes)?;
    }
    #[cfg(not(target_endian = "little"))]
    {
        let mut bytes = vec![0u8; samples * F32_BYTES];
        r.read_exact(&mut bytes)?;
        for (sample, b) in pcm.iter_mut().zip(bytes.chunks_exact(F32_BYTES)) {
            *sample = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        }
    }
    Ok(pcm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_request() -> Request {
        Request::Run {
            id: 7,
            options: RunOptions::default(),
        }
    }

    #[test]
    fn round_trips_message_with_pcm() {
        let mut buf = Vec::new();
        let pcm = [0.0f32, -1.0, 0.5, f32::MIN_POSITIVE, f32::MAX, -0.0];
        write_message(&mut buf, &run_request(), Some(&pcm)).unwrap();
        let (req, got): (Request, Vec<f32>) = read_message(&mut buf.as_slice()).unwrap().unwrap();
        assert!(matches!(req, Request::Run { id: 7, .. }));
        assert_eq!(got.len(), pcm.len());
        for (a, b) in got.iter().zip(pcm.iter()) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }

    #[test]
    fn the_wire_format_is_little_endian_whatever_the_host() {
        let frame = encode_message(&Request::Feed, Some(&[1.0f32])).unwrap();
        assert_eq!(&frame[frame.len() - 4..], &1.0f32.to_le_bytes());
        let pcm_len = &frame[frame.len() - 8..frame.len() - 4];
        assert_eq!(pcm_len, &4u32.to_le_bytes());
    }

    #[test]
    fn two_frames_in_a_row_read_back_in_order() {
        let mut buf = Vec::new();
        write_message(&mut buf, &Request::Feed, Some(&[0.25f32; 3])).unwrap();
        write_message(&mut buf, &Request::Cancel { id: 9 }, None).unwrap();
        let mut reader = buf.as_slice();
        let (first, pcm): (Request, _) = read_message(&mut reader).unwrap().unwrap();
        assert!(matches!(first, Request::Feed));
        assert_eq!(pcm, vec![0.25; 3]);
        let (second, pcm): (Request, _) = read_message(&mut reader).unwrap().unwrap();
        assert!(matches!(second, Request::Cancel { id: 9 }));
        assert!(pcm.is_empty());
        assert!(read_message::<Request>(&mut reader).unwrap().is_none());
    }

    #[test]
    fn clean_eof_is_none() {
        let empty: &[u8] = &[];
        assert!(read_message::<Request>(&mut &*empty).unwrap().is_none());
    }

    #[test]
    fn truncated_frame_is_an_error() {
        let mut buf = Vec::new();
        write_message(&mut buf, &run_request(), Some(&[0.5f32; 8])).unwrap();
        let json_len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        // Cut inside the length prefix, the JSON, the PCM length and the PCM.
        for cut in [1, 3, 6, 4 + json_len + 2, buf.len() - 1] {
            let partial = &buf[..cut];
            let err = read_message::<Request>(&mut &*partial).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof, "cut at {cut}");
        }
    }

    #[test]
    fn an_oversized_length_is_refused_before_allocating() {
        // A JSON length one past the limit, followed by nothing: refused on
        // the length alone, not by trying to read a gigabyte.
        let mut frame = ((MAX_SECTION_BYTES + 1) as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(b"{}");
        let err = read_message::<Request>(&mut frame.as_slice()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(err.to_string().contains("too large"), "{err}");

        // Same for the PCM section.
        let json = serde_json::to_vec(&Request::Feed).unwrap();
        let mut frame = (json.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(&json);
        frame.extend_from_slice(&u32::MAX.to_le_bytes());
        let err = read_message::<Request>(&mut frame.as_slice()).unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
    }

    #[test]
    fn a_pcm_section_of_partial_samples_is_refused() {
        let json = serde_json::to_vec(&Request::Feed).unwrap();
        let mut frame = (json.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(&json);
        frame.extend_from_slice(&3u32.to_le_bytes());
        frame.extend_from_slice(&[0, 0, 0]);
        let err = read_message::<Request>(&mut frame.as_slice()).unwrap_err();
        assert!(err.to_string().contains("whole number"), "{err}");
    }

    #[test]
    fn garbage_json_is_an_error_not_a_panic() {
        let mut frame = 3u32.to_le_bytes().to_vec();
        frame.extend_from_slice(b"{x}");
        frame.extend_from_slice(&0u32.to_le_bytes());
        assert!(read_message::<Request>(&mut frame.as_slice()).is_err());
    }

    #[test]
    fn section_limit_is_inclusive_and_fits_the_length_prefix() {
        assert!(check_section_len(MAX_SECTION_BYTES).is_ok());
        assert!(check_section_len(MAX_SECTION_BYTES + 1).is_err());
        assert!(MAX_SECTION_BYTES <= u32::MAX as usize);
    }

    #[test]
    fn a_transcript_with_nan_confidence_round_trips() {
        // A family without confidences reports NaN; JSON has no NaN, so it
        // travels as `null` and must come back as NaN, not fail the frame.
        let transcript = Transcript {
            text: "x".into(),
            tokens: vec![transcribe_cpp::Token {
                p: f32::NAN,
                ..transcribe_cpp::Token::default()
            }],
            ..Transcript::default()
        };
        let mut buf = Vec::new();
        write_message(&mut buf, &Response::Transcript(transcript), None).unwrap();
        let (back, _): (Response, _) = read_message(&mut buf.as_slice()).unwrap().unwrap();
        let Response::Transcript(back) = back else {
            panic!("not a transcript: {back:?}");
        };
        assert_eq!(back.text, "x");
        assert!(back.tokens[0].p.is_nan());
    }

    #[test]
    fn device_key_matches_the_saved_format() {
        assert_eq!(
            device_key("metal", None, "Metal"),
            r#"["metal","name","Metal"]"#
        );
        assert_eq!(
            device_key("vulkan", Some("0000:01:00.0"), "Vulkan0"),
            r#"["vulkan","id","0000:01:00.0"]"#
        );
    }
}
