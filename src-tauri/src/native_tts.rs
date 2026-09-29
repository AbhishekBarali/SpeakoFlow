//! Native local speech: Kokoro on the processor, and the voices that only run
//! here — Kitten (nano, micro, mini), Kyutai's Pocket TTS, and Supertone's
//! Supertonic 3.
//!
//! The default local voice is Kokoro running inside the assistant panel's
//! WebView (kokoro-js on WebGPU). On a working graphics card that is fast, but
//! the WebView is a poor host for everything else: without WebGPU (every Linux
//! WebKitGTK build, many VMs, older GPUs) it falls back to single-threaded
//! WebAssembly, which on a Ryzen 7 took over ten seconds per reply, and some GPU
//! and driver combinations produce garbled audio that throws no error at all
//! (issue #29). This module is the other way to run a local voice: the
//! [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) engine, natively, on
//! every core of the processor. Measured on a Ryzen 7 7700X, Kokoro renders
//! speech about five times faster than it plays (RTF 0.21 at 8 threads) and
//! Kitten nano about fifteen times faster (RTF 0.06).
//!
//! **The processor, not the graphics card, on purpose.** sherpa-onnx publishes
//! GPU builds only for NVIDIA CUDA (480–600 MB on Windows, and they need the
//! CUDA and cuDNN runtimes installed), none for DirectML, Vulkan or Metal. The
//! voices here are small enough that a batch-of-one GPU run gains little: Kyutai
//! measured no speedup for Pocket TTS on a strong CPU. Kokoro keeps its GPU path
//! through the WebView.
//!
//! Four decisions shape it.
//!
//! **The engine is downloaded, not bundled.** sherpa-onnx's shared build is
//! ~22 MB on disk (onnxruntime plus the C API). Most users never need it, so it
//! arrives with the first native voice pack instead of making every installer
//! heavier. Both the runtime archive and the voice packs are pinned by SHA-256.
//!
//! **It is loaded with `libloading`, not linked.** The app already links ONNX
//! Runtime statically through `ort` (VAD, transcription, diarization); linking a
//! second copy would collide at link time. Loading sherpa's own onnxruntime by
//! absolute path keeps the two apart: on Windows a DLL's imports bind by module,
//! on Linux the preloaded SONAME satisfies the C API's `DT_NEEDED`, and macOS
//! uses two-level namespaces. The library reports its version on load and
//! anything but [`SHERPA_VERSION`] is refused, because the `#[repr(C)]` structs
//! below mirror that release's `c-api.h` field for field.
//!
//! **One engine is loaded at a time, and it goes away when idle.** Kokoro costs
//! ~400 MB of RAM while loaded and Kitten ~80 MB. Synthesis is serialized on one
//! lock (a CPU engine gains nothing from running two requests at once), and a
//! watcher releases the engine after [`IDLE_UNLOAD`] without speech.
//!
//! **Kokoro keeps its GPU path.** [`route`] only sends Kokoro here when the
//! user asks for the processor, or when Automatic finds the WebView cannot use
//! the graphics card. Kitten, Pocket TTS and Supertonic always run here.

use log::{debug, info, warn};
use once_cell::sync::{Lazy, OnceCell};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::settings::AppSettings;

/// The sherpa-onnx release whose C API the structs below mirror. Bumping it
/// means re-checking every `#[repr(C)]` struct against that release's
/// `sherpa-onnx/c-api/c-api.h`, then updating the runtime hashes.
pub const SHERPA_VERSION: &str = "1.13.8";

/// How long a loaded engine may sit unused before its memory is given back.
const IDLE_UNLOAD: Duration = Duration::from_secs(180);

/// Longest piece of text sent to the engine in one call. The engine splits on
/// sentence ends itself, but an unpunctuated run past its token window is cut
/// off rather than split, so callers break text below this first.
pub const MAX_CHARS: usize = 400;

// ---------------------------------------------------------------------------
// Voice packs and the engine runtime
// ---------------------------------------------------------------------------

/// Which model family a pack contains; decides which config block is filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceFamily {
    Kokoro,
    Kitten,
    /// Kyutai's Pocket TTS. It has no built-in speakers: every reply is spoken
    /// in the voice of a short reference clip ([`VoiceClip`]).
    Pocket,
    /// Supertone's Supertonic 3: ten built-in speakers, 44.1 kHz output.
    Supertonic,
}

/// Where a pack keeps the files its family's config block names, relative to
/// the pack folder. The names differ between precisions of one family, so they
/// are part of the pack rather than fixed per family.
#[derive(Debug)]
pub enum Layout {
    /// Kokoro and Kitten: one model beside `voices.bin`, `tokens.txt` and
    /// eSpeak NG's data folder.
    Espeak { model: &'static str },
    /// Pocket TTS: five graphs, plus `vocab.json` and `token_scores.json`.
    Pocket {
        lm_flow: &'static str,
        lm_main: &'static str,
        encoder: &'static str,
        decoder: &'static str,
        text_conditioner: &'static str,
    },
    /// Supertonic: four graphs, plus `tts.json`, `unicode_indexer.bin` and the
    /// speakers' styles in `voice.bin`.
    Supertonic {
        duration_predictor: &'static str,
        text_encoder: &'static str,
        vector_estimator: &'static str,
        vocoder: &'static str,
    },
}

const ESPEAK_FILES: [&str; 3] = ["voices.bin", "tokens.txt", "espeak-ng-data"];
const POCKET_FILES: [&str; 2] = ["vocab.json", "token_scores.json"];
const SUPERTONIC_FILES: [&str; 3] = ["tts.json", "unicode_indexer.bin", "voice.bin"];

/// A reference recording a Pocket voice is cloned from, fetched beside the
/// pack's archive into `<pack>/voices/`. Pinned to one commit of
/// [`VOICES_REPO`] and checked against its SHA-256, like the archive itself.
#[derive(Debug)]
pub struct VoiceClip {
    /// The name the app offers, which is Kyutai's own name for the voice.
    pub name: &'static str,
    /// Path inside [`VOICES_REPO`].
    pub source: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

/// Kyutai's voice collection for its TTS models, and the commit the clips
/// below were taken from.
pub const VOICES_REPO: &str = "kyutai/tts-voices";
pub const VOICES_REVISION: &str = "323332d33f997de8394f24a193e1a76df720e01a";

impl VoiceClip {
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/{VOICES_REPO}/resolve/{VOICES_REVISION}/{}",
            self.source
        )
    }

    /// Where the clip lives, relative to its pack's folder.
    pub fn relative_path(&self) -> String {
        format!("voices/{}.wav", self.name.to_ascii_lowercase())
    }
}

/// A downloadable native voice. The model catalog in `managers/model.rs`
/// builds its entries from these, so the URL and hash live in one place.
#[derive(Debug)]
pub struct VoicePack {
    /// Catalog id in the model manager.
    pub model_id: &'static str,
    /// Name and one-line description in the model catalog.
    pub name: &'static str,
    pub description: &'static str,
    /// Directory under `<app data>/models/` the archive extracts to.
    pub dir: &'static str,
    pub archive_url: &'static str,
    pub sha256: &'static str,
    /// Size of the archive, for the UI.
    pub download_bytes: u64,
    pub family: VoiceFamily,
    pub layout: Layout,
    /// Reference recordings downloaded beside the archive (Pocket only).
    pub clips: &'static [VoiceClip],
    /// What the model's own speed 1.0 must be multiplied by to sound like a
    /// normal speaking pace. Measured per pack against Kokoro at 1.0.
    pace: f64,
    /// Most intra-op threads worth giving it (see [`threads_for`]).
    max_threads: usize,
}

impl VoicePack {
    /// Every file and folder the engine will open, relative to the pack folder.
    pub fn required_files(&self) -> Vec<String> {
        let mut files: Vec<String> = match &self.layout {
            Layout::Espeak { model } => std::iter::once(*model)
                .chain(ESPEAK_FILES)
                .map(str::to_string)
                .collect(),
            Layout::Pocket {
                lm_flow,
                lm_main,
                encoder,
                decoder,
                text_conditioner,
            } => [*lm_flow, *lm_main, *encoder, *decoder, *text_conditioner]
                .into_iter()
                .chain(POCKET_FILES)
                .map(str::to_string)
                .collect(),
            Layout::Supertonic {
                duration_predictor,
                text_encoder,
                vector_estimator,
                vocoder,
            } => [
                *duration_predictor,
                *text_encoder,
                *vector_estimator,
                *vocoder,
            ]
            .into_iter()
            .chain(SUPERTONIC_FILES)
            .map(str::to_string)
            .collect(),
        };
        files.extend(self.clips.iter().map(VoiceClip::relative_path));
        files
    }

    /// Archive plus reference clips, for the download size the UI shows.
    pub fn total_download_bytes(&self) -> u64 {
        self.download_bytes + self.clips.iter().map(|clip| clip.bytes).sum::<u64>()
    }
}

/// Kokoro v1.0, full precision. The int8 build is smaller (132 MB) but its
/// `ConvInteger` operators have no fast x86 kernel: it measured slower than
/// real time at every thread count, while fp32 runs at RTF 0.2–0.3.
pub const KOKORO_PACK: VoicePack = VoicePack {
    model_id: "kokoro-82m-native",
    name: "Kokoro (processor)",
    description: "Kokoro running on your processor, for computers where the graphics card can't run the voice.",
    dir: "kokoro-multi-lang-v1_0",
    archive_url:
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_0.tar.bz2",
    sha256: "c5f7e2d2caf082bc1d20fb70334a61d99d20b484500aad32e7cf84c128ea3298",
    download_bytes: 349_906_910,
    family: VoiceFamily::Kokoro,
    layout: Layout::Espeak {
        model: "model.onnx",
    },
    clips: &[],
    pace: 1.0,
    max_threads: 8,
};

/// Kitten TTS nano 0.8, full precision. Like Kokoro, the int8 build is the
/// slower one on a desktop CPU (RTF 0.27 against 0.06–0.11 for fp32).
///
/// Kitten speaks at roughly 95 words a minute at its own speed 1.0 (upstream
/// KittenTTS 0.8 and sherpa-onnx agree: ~11 s for an 18-word sentence that
/// Kokoro says in ~5 s), hence the pace.
pub const KITTEN_PACK: VoicePack = VoicePack {
    model_id: "kitten-nano-0.8",
    name: "Kitten nano",
    description: "The smallest Kitten voice: 15M parameters, the fastest on any processor.",
    dir: "kitten-nano-en-v0_8-fp32",
    archive_url:
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kitten-nano-en-v0_8-fp32.tar.bz2",
    sha256: "16092117bfe591ddcd58d078e1454603b8e1caea46f85653b2c2efae76bd883e",
    download_bytes: 63_815_222,
    family: VoiceFamily::Kitten,
    layout: Layout::Espeak {
        model: "model.fp32.onnx",
    },
    clips: &[],
    pace: 1.6,
    max_threads: 4,
};

/// Kitten TTS micro 0.8 (40M parameters): the same eight speakers as nano,
/// with more of the model's capacity spent on each. It speaks a little faster
/// than nano at the same model speed, so its pace is lower (measured ~210
/// words a minute at 1x for all three sizes, against ~200 for Supertonic).
pub const KITTEN_MICRO_PACK: VoicePack = VoicePack {
    model_id: "kitten-micro-0.8",
    name: "Kitten micro",
    description: "The middle Kitten voice: 40M parameters, clearer than nano.",
    dir: "kitten-micro-en-v0_8",
    archive_url:
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kitten-micro-en-v0_8.tar.bz2",
    sha256: "85faaea7511ca9d1d2f251fed0a4553bdf0d1ee046102fa60ddd8046c751f76f",
    download_bytes: 44_423_643,
    family: VoiceFamily::Kitten,
    layout: Layout::Espeak {
        model: "model.onnx",
    },
    clips: &[],
    pace: 1.5,
    max_threads: 4,
};

/// Kitten TTS mini 0.8 (80M parameters), the most natural Kitten.
pub const KITTEN_MINI_PACK: VoicePack = VoicePack {
    model_id: "kitten-mini-0.8",
    name: "Kitten mini",
    description: "The largest Kitten voice: 80M parameters, the most natural of the three.",
    dir: "kitten-mini-en-v0_8",
    archive_url:
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kitten-mini-en-v0_8.tar.bz2",
    sha256: "518f9b130320f690d5b5476df77bde4215fca67773cda16710318e5081234b9d",
    download_bytes: 67_547_594,
    family: VoiceFamily::Kitten,
    layout: Layout::Espeak {
        model: "model.onnx",
    },
    clips: &[],
    pace: 1.3,
    max_threads: 4,
};

/// Every Kitten size, smallest first. They share one voice list.
pub const KITTEN_PACKS: [&VoicePack; 3] = [&KITTEN_PACK, &KITTEN_MICRO_PACK, &KITTEN_MINI_PACK];

/// Pocket TTS's voices, from [`VOICES_REPO`]. Pocket has no voices of its
/// own: it copies the one in its reference recording, recording quality
/// included, so which recordings ship is the biggest single decision about how
/// it sounds.
///
/// Each one was scored by rendering four sentences and rating them with UTMOS
/// (a predicted 1-5 listener score; Kokoro's af_heart scores 4.47 on the same
/// sentences) and a speaker-embedding similarity to the recording. Five of
/// Kyutai's presets are left out on that evidence:
///
/// - **Marius** and **Javert** are phone recordings from Kyutai's voice
///   donations (the recordings themselves score 2.7 and 3.0). Marius rendered
///   at 1.8 with its pitch doubling, and Javert produced 22 seconds of audio
///   for a six-second sentence.
/// - **Alba**, Kyutai's default, is listed as a woman's voice, but the
///   recording sits at 126 Hz, the range of a man's voice, and the clone
///   copies it faithfully (136 Hz). Offered as "female" it sounds mislabelled.
/// - **Charles** and **Michael** scored 3.5, the lowest of the rest.
///
/// Kyutai's two presets from non-commercial datasets (Cosette, Jean) are not
/// offered either. Every voice here is CC BY 4.0 (VCTK) or CC0 (Voice-Zero).
/// Listed best first within each group, as scored.
pub const POCKET_VOICES: [VoiceClip; 14] = [
    VoiceClip {
        name: "Mary",
        source: "vctk/p333_023_enhanced.wav",
        bytes: 639_084,
        sha256: "a35b0468382218e9f37a9a7494d1e4b74deaf18d7ced22265b4e325bb55c183f",
    },
    VoiceClip {
        name: "Caro",
        source: "voice-zero/caro_davy.wav",
        bytes: 743_528,
        sha256: "40c692c005a0268a7a5b6ebae348077d3dca6a86eb6b12bd36e343bbcd71b5f6",
    },
    VoiceClip {
        name: "Azelma",
        source: "vctk/p303_023_enhanced.wav",
        bytes: 823_852,
        sha256: "60e3d26cdf2efdec5df712152c839928f4d5522821e6554ae11fd96c57ab1026",
    },
    VoiceClip {
        name: "Vera",
        source: "vctk/p229_023_enhanced.wav",
        bytes: 691_416,
        sha256: "309cf91a895830f15842b398f69a4962cb1f7e0bfab10e25dd27838e826c204b",
    },
    VoiceClip {
        name: "Eve",
        source: "vctk/p361_023_enhanced.wav",
        bytes: 671_872,
        sha256: "396e7cbd066b0f3fb6d67fa26e7904076958239d736d4390f15b5fe88feb14cd",
    },
    VoiceClip {
        name: "Anna",
        source: "vctk/p228_023_enhanced.wav",
        bytes: 804_630,
        sha256: "0a6de25cf12bf1540beb85979f306a92be81fecc051c547c5395e7e5237a3856",
    },
    VoiceClip {
        name: "Jane",
        source: "vctk/p339_023_enhanced.wav",
        bytes: 759_340,
        sha256: "2f12e7f155eb3118f55425394f1b049e5b1b67bdc9b3932c8ba4521420aeb84a",
    },
    VoiceClip {
        name: "Fantine",
        source: "vctk/p244_023_enhanced.wav",
        bytes: 674_852,
        sha256: "5f07d4e2a3f20a15572aae885156b43ef3fc12ef3812996fd135680d9956448b",
    },
    VoiceClip {
        name: "Eponine",
        source: "vctk/p262_023_enhanced.wav",
        bytes: 716_330,
        sha256: "a13c27fb47627b05223691a0ef2974358a18c886e6c2f9d2762ff1d02c20926b",
    },
    VoiceClip {
        name: "George",
        source: "vctk/p315_023_enhanced.wav",
        bytes: 642_692,
        sha256: "29a41f93bf5236e5b21501091d7774c255d5f3d4e62fa4f9fdf0a92a793c84ae",
    },
    VoiceClip {
        name: "Bill",
        source: "voice-zero/bill_boerst.wav",
        bytes: 955_496,
        sha256: "be4815e4fb760ba1b78117545a260cce4a4c124c7657bc5c6127a0fef8ba661f",
    },
    VoiceClip {
        name: "Peter",
        source: "voice-zero/peter_yearsley.wav",
        bytes: 524_448,
        sha256: "fbb3920fda7ae26a5a8b317ffcae1d55c0bd5d89d075205f5a52b1e924b83f51",
    },
    VoiceClip {
        name: "Stuart",
        source: "voice-zero/stuart_bell.wav",
        bytes: 745_776,
        sha256: "00c7baeb2fb7a8c1c6198e045b5e853a7ccc04002a51a09b4be3dd7c96994f73",
    },
    VoiceClip {
        name: "Paul",
        source: "vctk/p259_023_enhanced.wav",
        bytes: 717_182,
        sha256: "7aba504fe0b3b16478b69eb27ce6007e3cb42b0c1915b5f1c6a6024ae37d679b",
    },
];

/// The best-scoring voice (UTMOS 4.38 against af_heart's 4.47).
pub const DEFAULT_POCKET_VOICE: &str = "Mary";

/// Pocket TTS (Kyutai, 100M parameters), full precision.
///
/// Not the int8 build, which is 70 MB smaller: across the same voices and
/// sentences it scored 3.48 against fp32's 3.62 and misread half again as many
/// words (8% against 5%). Two other projects reached the same conclusion by
/// ear (k2-fsa/sherpa-onnx#3172; block/buzz switched for it). fp32 costs RTF
/// 0.25 against 0.20 on an 8-core Ryzen, still four times faster than speech,
/// and about 500 MB of memory while loaded.
pub const POCKET_PACK: VoicePack = VoicePack {
    model_id: "pocket-tts",
    name: "Pocket TTS",
    description: "Kyutai's Pocket TTS: a natural English voice built for processors, with 14 speakers.",
    dir: "sherpa-onnx-pocket-tts-2026-01-26",
    archive_url:
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/sherpa-onnx-pocket-tts-2026-01-26.tar.bz2",
    sha256: "61422a478ef09d9b2c067261fb822e11786e0c86842c76d8633b7069968be7b5",
    download_bytes: 168_148_625,
    family: VoiceFamily::Pocket,
    layout: Layout::Pocket {
        lm_flow: "lm_flow.onnx",
        lm_main: "lm_main.onnx",
        encoder: "encoder.onnx",
        decoder: "decoder.onnx",
        text_conditioner: "text_conditioner.onnx",
    },
    clips: &POCKET_VOICES,
    pace: 1.0,
    max_threads: 4,
};

/// Supertonic 3's speakers, in `voice.bin` order: sherpa-onnx packs the style
/// files sorted by name, so F1–F5 are ids 0–4 and M1–M5 ids 5–9.
pub const SUPERTONIC_VOICES: [&str; 10] =
    ["F1", "F2", "F3", "F4", "F5", "M1", "M2", "M3", "M4", "M5"];

pub const DEFAULT_SUPERTONIC_VOICE: &str = "F1";

/// Supertonic 3 (Supertone), int8 as sherpa-onnx publishes it. Its speed 1.0
/// is a deliberate ~160 words a minute, slower than the other local voices;
/// the pace brings the app's 1x to ~190 without costing accuracy (word error
/// rate 0.015 at 1.2 against 0.020 at 1.0 on the quality matrix).
pub const SUPERTONIC_PACK: VoicePack = VoicePack {
    model_id: "supertonic-3-int8",
    name: "Supertonic 3",
    description: "Supertone's Supertonic 3: a crisp, fast voice with ten speakers.",
    dir: "sherpa-onnx-supertonic-3-tts-int8-2026-05-11",
    archive_url:
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/sherpa-onnx-supertonic-3-tts-int8-2026-05-11.tar.bz2",
    sha256: "82fa96f91c4ef8abaae3a14a3f4153facf88bed821d1f7331cec2700f432c427",
    download_bytes: 128_774_318,
    family: VoiceFamily::Supertonic,
    layout: Layout::Supertonic {
        duration_predictor: "duration_predictor.int8.onnx",
        text_encoder: "text_encoder.int8.onnx",
        vector_estimator: "vector_estimator.int8.onnx",
        vocoder: "vocoder.int8.onnx",
    },
    clips: &[],
    pace: 1.2,
    max_threads: 6,
};

pub const PACKS: [&VoicePack; 6] = [
    &KOKORO_PACK,
    &KITTEN_PACK,
    &KITTEN_MICRO_PACK,
    &KITTEN_MINI_PACK,
    &POCKET_PACK,
    &SUPERTONIC_PACK,
];

/// The pack a catalog id refers to.
pub fn pack_for_model_id(model_id: &str) -> Option<&'static VoicePack> {
    PACKS.iter().copied().find(|pack| pack.model_id == model_id)
}

/// Engine ids (`assistant_tts_engine`) that only ever speak through this
/// module. Kokoro is not one of them: it lives in the WebView and only moves
/// here when [`route`] says so.
pub const NATIVE_ENGINES: [&str; 3] = ["kitten", "pocket", "supertonic"];

pub fn is_native_engine(engine: &str) -> bool {
    NATIVE_ENGINES.contains(&engine.trim())
}

/// The name a native engine goes by in messages.
fn engine_label(engine: &str) -> &'static str {
    match engine.trim() {
        "pocket" => "Pocket TTS",
        "supertonic" => "Supertonic",
        _ => "Kitten",
    }
}

/// The Kitten size a model setting names: a catalog id, or a bare size.
/// Anything else is nano, the size every earlier version installed.
pub fn kitten_pack(model: &str) -> &'static VoicePack {
    let model = model.trim().to_ascii_lowercase();
    KITTEN_PACKS
        .iter()
        .copied()
        .find(|pack| pack.model_id == model || pack.model_id == format!("kitten-{model}-0.8"))
        .unwrap_or(&KITTEN_PACK)
}

/// The pack a native engine speaks with, given its model setting.
pub fn pack_for_engine(engine: &str, model: &str) -> Option<&'static VoicePack> {
    match engine.trim() {
        "kitten" => Some(kitten_pack(model)),
        "pocket" => Some(&POCKET_PACK),
        "supertonic" => Some(&SUPERTONIC_PACK),
        _ => None,
    }
}

/// A prebuilt sherpa-onnx shared-library archive for one platform.
#[derive(Debug)]
pub struct RuntimeAsset {
    pub archive: &'static str,
    pub sha256: &'static str,
}

impl RuntimeAsset {
    pub fn url(&self) -> String {
        format!(
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/v{SHERPA_VERSION}/{}",
            self.archive
        )
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const RUNTIME: Option<RuntimeAsset> = Some(RuntimeAsset {
    archive: "sherpa-onnx-v1.13.8-win-x64-shared-MT-Release-lib.tar.bz2",
    sha256: "b8eedf41bd6d3779218887b48367bb7a3ece5aaa7667f01f69ee823a12b0a9e7",
});
#[cfg(all(target_os = "windows", target_arch = "aarch64"))]
const RUNTIME: Option<RuntimeAsset> = Some(RuntimeAsset {
    archive: "sherpa-onnx-v1.13.8-win-arm64-shared-MT-Release-lib.tar.bz2",
    sha256: "22cd2b2b5e35c1132abc74a91ee77f683645437382617af6a4bd5818b9c509f4",
});
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const RUNTIME: Option<RuntimeAsset> = Some(RuntimeAsset {
    archive: "sherpa-onnx-v1.13.8-linux-x64-shared-lib.tar.bz2",
    sha256: "3892d184be41027e18165e67f549cd4e4cdd8dcd73ac5579e97afd55e14e30b6",
});
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const RUNTIME: Option<RuntimeAsset> = Some(RuntimeAsset {
    archive: "sherpa-onnx-v1.13.8-linux-aarch64-shared-cpu-lib.tar.bz2",
    sha256: "fb98b80628a383909bf83626daca2e0d0ffc93bf1f7d222eaaa4282b388f072f",
});
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const RUNTIME: Option<RuntimeAsset> = Some(RuntimeAsset {
    archive: "sherpa-onnx-v1.13.8-osx-arm64-shared-lib.tar.bz2",
    sha256: "ae77050cdae565496059d96f5ab33d77b397a0864282a4e4a26e3b3b3effb948",
});
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const RUNTIME: Option<RuntimeAsset> = Some(RuntimeAsset {
    archive: "sherpa-onnx-v1.13.8-osx-x64-shared-lib.tar.bz2",
    sha256: "0f88371565a06372889c76266e253abd1b415fcc6f98232cabd968933efb0711",
});
#[cfg(not(any(
    all(
        target_os = "windows",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(
        target_os = "macos",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
)))]
const RUNTIME: Option<RuntimeAsset> = None;

/// This platform's runtime archive, or `None` where sherpa-onnx ships none.
pub fn runtime_asset() -> Option<&'static RuntimeAsset> {
    RUNTIME.as_ref()
}

/// Whether native voices can run on this platform at all.
pub fn native_supported() -> bool {
    runtime_asset().is_some()
}

/// Set when the downloaded engine was on disk but refused to load this session,
/// e.g. macOS library validation rejecting a library the app didn't ship with.
/// While set, nothing counts as ready, so Kokoro keeps speaking in the WebView
/// instead of failing every reply on a route that cannot work.
static LOAD_FAILED: AtomicBool = AtomicBool::new(false);

/// The engine is on disk but could not be loaded this session.
pub fn load_failed() -> bool {
    LOAD_FAILED.load(Ordering::Relaxed)
}

/// Called when something inside this module changes where voices speak (the
/// engine failing to load), so the windows can follow. Set once at startup.
static STATUS_LISTENER: OnceCell<Box<dyn Fn() + Send + Sync>> = OnceCell::new();

pub fn set_status_listener(listener: impl Fn() + Send + Sync + 'static) {
    let _ = STATUS_LISTENER.set(Box::new(listener));
}

fn notify_status() {
    if let Some(listener) = STATUS_LISTENER.get() {
        listener();
    }
}

/// Packs whose engine refused to start this session (the library loaded, but
/// creating the voice from these files failed). Like [`LOAD_FAILED`], a pack
/// listed here reads as not ready, so Kokoro falls back to the WebView instead
/// of failing every reply. Cleared when the pack is removed or reinstalled.
static PACK_FAILED: Lazy<Mutex<Vec<&'static str>>> = Lazy::new(|| Mutex::new(Vec::new()));

fn pack_failed(pack: &VoicePack) -> bool {
    PACK_FAILED
        .lock()
        .map(|failed| failed.contains(&pack.model_id))
        .unwrap_or(false)
}

/// Remember that `pack` cannot start; `true` the first time.
fn set_pack_failed(pack: &'static VoicePack) -> bool {
    PACK_FAILED.lock().is_ok_and(|mut failed| {
        if failed.contains(&pack.model_id) {
            false
        } else {
            failed.push(pack.model_id);
            true
        }
    })
}

fn forget_pack_failure(model_id: &str) {
    if let Ok(mut failed) = PACK_FAILED.lock() {
        failed.retain(|id| *id != model_id);
    }
}

/// A voice pack finished installing: give it a fresh start, and check in the
/// background that the engine library loads here, so a machine that refuses it
/// (macOS library validation) says so in Settings now rather than on the first
/// reply. Loading the library maps it into the process; no voice is loaded.
pub fn after_install(model_id: &str) {
    forget_pack_failure(model_id);
    let _ = std::thread::Builder::new()
        .name("native-voice-verify".into())
        .spawn(|| {
            if let Err(e) = runtime() {
                debug!("Native voice engine check after install failed: {e}");
            }
        });
}

#[cfg(target_os = "windows")]
const ORT_LIB: &str = "onnxruntime.dll";
#[cfg(target_os = "windows")]
const C_API_LIB: &str = "sherpa-onnx-c-api.dll";
#[cfg(target_os = "macos")]
const ORT_LIB: &str = "libonnxruntime.dylib";
#[cfg(target_os = "macos")]
const C_API_LIB: &str = "libsherpa-onnx-c-api.dylib";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const ORT_LIB: &str = "libonnxruntime.so";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const C_API_LIB: &str = "libsherpa-onnx-c-api.so";

/// Files copied out of the runtime archive. The Windows provider bridge is only
/// needed by non-CPU execution providers, so it is copied when present but not
/// required.
const RUNTIME_OPTIONAL: &[&str] = &["onnxruntime_providers_shared.dll"];

/// `<models>/tts-runtime/sherpa-onnx-<version>/`.
pub fn runtime_dir_in(models_dir: &Path) -> PathBuf {
    models_dir
        .join("tts-runtime")
        .join(format!("sherpa-onnx-{SHERPA_VERSION}"))
}

/// Whether this version's runtime libraries are on disk.
pub fn runtime_installed_in(models_dir: &Path) -> bool {
    let dir = runtime_dir_in(models_dir);
    dir.join(ORT_LIB).is_file() && dir.join(C_API_LIB).is_file()
}

/// Whether a voice pack's files are on disk (not whether it is loaded),
/// including any reference clips downloaded beside its archive.
pub fn pack_installed_in(models_dir: &Path, pack: &VoicePack) -> bool {
    let dir = models_dir.join(pack.dir);
    pack.required_files()
        .iter()
        .all(|file| dir.join(file).exists())
}

/// The reference clips of `pack` that are not on disk yet, with the path each
/// belongs at. The model manager fetches these after unpacking the archive.
pub fn missing_clips(
    models_dir: &Path,
    pack: &'static VoicePack,
) -> Vec<(&'static VoiceClip, PathBuf)> {
    let dir = models_dir.join(pack.dir);
    pack.clips
        .iter()
        .map(|clip| (clip, dir.join(clip.relative_path())))
        .filter(|(_, path)| !path.is_file())
        .collect()
}

/// Unpack a verified runtime archive into [`runtime_dir_in`]. Only the two
/// libraries (plus the optional provider bridge) are kept; headers and import
/// libraries are discarded, as is the archive itself.
pub fn install_runtime_archive(archive: &Path, models_dir: &Path) -> Result<(), String> {
    let base = models_dir.join("tts-runtime");
    let staging = base.join(format!(".extracting-{SHERPA_VERSION}"));
    let installing = base.join(format!(".installing-{SHERPA_VERSION}"));
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&installing);
    std::fs::create_dir_all(&staging)
        .and_then(|_| std::fs::create_dir_all(&installing))
        .map_err(|e| format!("Couldn't prepare the voice engine folder: {e}"))?;

    let result = (|| -> Result<(), String> {
        let file = std::fs::File::open(archive)
            .map_err(|e| format!("Couldn't open the voice engine download: {e}"))?;
        let decoder = bzip2::read::MultiBzDecoder::new(std::io::BufReader::new(file));
        tar::Archive::new(decoder)
            .unpack(&staging)
            .map_err(|e| format!("Couldn't unpack the voice engine: {e}"))?;

        let mut wanted: Vec<(&str, bool)> = vec![(ORT_LIB, true), (C_API_LIB, true)];
        wanted.extend(RUNTIME_OPTIONAL.iter().map(|name| (*name, false)));
        for (name, required) in wanted {
            match find_file(&staging, name) {
                Some(source) => {
                    std::fs::copy(&source, installing.join(name))
                        .map_err(|e| format!("Couldn't install {name}: {e}"))?;
                }
                None if required => {
                    return Err(format!("The voice engine download is missing {name}"));
                }
                None => {}
            }
        }

        let final_dir = runtime_dir_in(models_dir);
        if final_dir.exists() {
            std::fs::remove_dir_all(&final_dir)
                .map_err(|e| format!("Couldn't replace the old voice engine: {e}"))?;
        }
        std::fs::rename(&installing, &final_dir)
            .map_err(|e| format!("Couldn't install the voice engine: {e}"))
    })();

    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&installing);
    if result.is_ok() {
        let _ = std::fs::remove_file(archive);
        // New files: let the next voice try loading them again.
        LOAD_FAILED.store(false, Ordering::Relaxed);
        info!("Native voice engine installed (sherpa-onnx {SHERPA_VERSION})");
    }
    result
}

/// Depth-first search for a file by name under `dir` (the archives nest their
/// libraries one or two folders down).
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
                return Some(path);
            }
        }
    }
    None
}

/// Where the model manager keeps its files. Set once at startup so the speech
/// paths, which only have settings in hand, can find the packs.
static MODELS_DIR: OnceCell<PathBuf> = OnceCell::new();

/// Record the models directory, and tidy a runtime nobody needs any more: one
/// left behind by an older version, or one whose last voice pack was removed.
pub fn init(models_dir: PathBuf) {
    let tts_runtime = models_dir.join("tts-runtime");
    if let Ok(entries) = std::fs::read_dir(&tts_runtime) {
        let current = runtime_dir_in(&models_dir);
        for entry in entries.flatten() {
            let path = entry.path();
            if path != current {
                debug!("Removing stale native voice files at {}", path.display());
                let _ = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
            }
        }
    }
    let any_pack = PACKS.iter().any(|pack| models_dir.join(pack.dir).is_dir());
    if !any_pack && tts_runtime.exists() {
        debug!("No native voice pack installed; removing the unused voice engine");
        let _ = std::fs::remove_dir_all(&tts_runtime);
    }
    let _ = MODELS_DIR.set(models_dir);
}

fn models_dir() -> Option<&'static Path> {
    MODELS_DIR.get().map(PathBuf::as_path)
}

/// Whether a pack and the runtime it needs are both installed.
pub fn pack_ready(pack: &VoicePack) -> bool {
    !load_failed()
        && !pack_failed(pack)
        && models_dir().is_some_and(|dir| {
            native_supported() && runtime_installed_in(dir) && pack_installed_in(dir, pack)
        })
}

/// Remove the runtime once no voice pack is left to use it. Skipped while the
/// libraries are loaded: Windows cannot delete a mapped DLL, and the next
/// launch's [`init`] cleans it up instead.
pub fn remove_runtime_if_unused(models_dir: &Path) {
    let any_pack = PACKS.iter().any(|pack| models_dir.join(pack.dir).is_dir());
    if any_pack || RUNTIME_LIB.lock().map(|r| r.is_some()).unwrap_or(true) {
        return;
    }
    let dir = models_dir.join("tts-runtime");
    if dir.exists() {
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => info!("Removed the native voice engine (no voice pack uses it)"),
            Err(e) => warn!("Couldn't remove the native voice engine: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Where speech is produced
// ---------------------------------------------------------------------------

/// Where the selected engine's speech is produced right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum VoiceRoute {
    /// Kokoro inside the assistant panel (kokoro-js, WebGPU or WebAssembly).
    Webview,
    /// This module (sherpa-onnx on the processor).
    Native,
    /// A cloud or self-hosted engine, spoken by `tts.rs`.
    Remote,
}

/// What the WebView reported about WebGPU on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WebGpuState {
    /// Nothing reported yet this session.
    Unknown,
    /// An adapter exists and has not been caught producing broken audio.
    Usable,
    /// No adapter, it failed to start, or its audio was garbled.
    Unusable,
}

static WEBGPU: AtomicU8 = AtomicU8::new(0);

pub fn webgpu_state() -> WebGpuState {
    match WEBGPU.load(Ordering::SeqCst) {
        1 => WebGpuState::Usable,
        2 => WebGpuState::Unusable,
        _ => WebGpuState::Unknown,
    }
}

/// Record the WebView's report. Returns whether anything changed.
pub fn report_webgpu(usable: bool) -> bool {
    let value = if usable { 1 } else { 2 };
    WEBGPU.swap(value, Ordering::SeqCst) != value
}

/// Where Kokoro should run, as the user asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KokoroDevice {
    Auto,
    Gpu,
    Cpu,
}

impl KokoroDevice {
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "gpu" => Self::Gpu,
            "cpu" => Self::Cpu,
            _ => Self::Auto,
        }
    }
}

/// The routing rule for Kokoro, separated from global state so every branch is
/// testable.
///
/// Automatic prefers the graphics card whenever the WebView can use it — on a
/// working GPU that is the fastest path and the one the owner's machines run
/// instantly — and only moves to the processor when the WebView reported no
/// usable GPU. In every case where the processor voice is wanted but not yet
/// downloaded, the answer is the WebView, which still speaks (on its slower
/// WebAssembly fallback) rather than going silent.
pub fn resolve_kokoro_route(
    device: KokoroDevice,
    webgpu: WebGpuState,
    native_ready: bool,
) -> VoiceRoute {
    let want_native = match device {
        KokoroDevice::Gpu => false,
        KokoroDevice::Cpu => true,
        KokoroDevice::Auto => webgpu == WebGpuState::Unusable,
    };
    if want_native && native_ready {
        VoiceRoute::Native
    } else {
        VoiceRoute::Webview
    }
}

/// Where the configured engine speaks.
pub fn route(settings: &AppSettings) -> VoiceRoute {
    let engine = settings.assistant_tts_engine.trim();
    if is_native_engine(engine) {
        return VoiceRoute::Native;
    }
    match engine {
        "" | "kokoro" => resolve_kokoro_route(
            KokoroDevice::parse(&settings.assistant_tts_kokoro_device),
            webgpu_state(),
            pack_ready(&KOKORO_PACK),
        ),
        _ => VoiceRoute::Remote,
    }
}

/// Whether speech is produced by the panel's WebView.
pub fn uses_webview(settings: &AppSettings) -> bool {
    route(settings) == VoiceRoute::Webview
}

/// The pack that speaks for these settings, when the route is native.
pub fn active_pack(settings: &AppSettings) -> Option<&'static VoicePack> {
    if route(settings) != VoiceRoute::Native {
        return None;
    }
    pack_for_engine(
        &settings.assistant_tts_engine,
        &settings.assistant_tts_model,
    )
    .or(Some(&KOKORO_PACK))
}

/// Why the configured native voice cannot speak, or `None` when it can. Only
/// the native-only engines can be blocked: Kokoro falls back to the WebView
/// when its processor pack is missing.
pub fn blocker(settings: &AppSettings) -> Option<String> {
    let engine = settings.assistant_tts_engine.trim();
    let pack = pack_for_engine(engine, &settings.assistant_tts_model)?;
    let name = engine_label(engine);
    if !native_supported() {
        return Some(format!(
            "{name} isn't available on this computer yet. Switch the voice to Kokoro in Models → Voice."
        ));
    }
    if load_failed() {
        return Some(format!(
            "{name}'s voice engine couldn't start on this computer. Switch the voice to Kokoro in Models → Voice."
        ));
    }
    if pack_failed(pack) {
        return Some(format!(
            "{name} couldn't start on this computer. Remove it in Models → Voice and download it again, or switch the voice to Kokoro."
        ));
    }
    if !pack_ready(pack) {
        return Some(format!(
            "The assistant's voice is set to {name}, which isn't downloaded yet. Download it in Models → Voice, or switch the voice to Kokoro."
        ));
    }
    None
}

/// Everything the Voice settings need to explain the current state.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct LocalVoiceStatus {
    pub route: VoiceRoute,
    pub webgpu: WebGpuState,
    /// This platform has a native engine build.
    pub native_supported: bool,
    /// The processor voice is downloaded but couldn't start this session: the
    /// engine library refused to load, or the Kokoro pack refused to start.
    pub native_load_failed: bool,
    pub kokoro_native_ready: bool,
    /// The Kitten size currently chosen is ready.
    pub kitten_ready: bool,
}

pub fn status(settings: &AppSettings) -> LocalVoiceStatus {
    let kitten_model = if settings.assistant_tts_engine.trim() == "kitten" {
        settings.assistant_tts_model.clone()
    } else {
        settings
            .assistant_tts_models
            .get("kitten")
            .cloned()
            .unwrap_or_default()
    };
    LocalVoiceStatus {
        route: route(settings),
        webgpu: webgpu_state(),
        native_supported: native_supported(),
        native_load_failed: load_failed() || pack_failed(&KOKORO_PACK),
        kokoro_native_ready: pack_ready(&KOKORO_PACK),
        kitten_ready: pack_ready(kitten_pack(&kitten_model)),
    }
}

// ---------------------------------------------------------------------------
// Voices, speed, threads
// ---------------------------------------------------------------------------

/// Kokoro v1.0's English speakers, in the order of the model's `voices.bin`
/// (the model's own `id2speaker` metadata; ids 28+ are other languages).
const KOKORO_VOICES: [&str; 28] = [
    "af_alloy",
    "af_aoede",
    "af_bella",
    "af_heart",
    "af_jessica",
    "af_kore",
    "af_nicole",
    "af_nova",
    "af_river",
    "af_sarah",
    "af_sky",
    "am_adam",
    "am_echo",
    "am_eric",
    "am_fenrir",
    "am_liam",
    "am_michael",
    "am_onyx",
    "am_puck",
    "am_santa",
    "bf_alice",
    "bf_emma",
    "bf_isabella",
    "bf_lily",
    "bm_daniel",
    "bm_fable",
    "bm_george",
    "bm_lewis",
];

pub const DEFAULT_KOKORO_VOICE: &str = "af_heart";

/// Kitten 0.8's voices by the names its authors give them, in `voices.bin`
/// order (`expr-voice-2-m`, `-2-f`, `-3-m`, … as sherpa-onnx packs them).
pub const KITTEN_VOICES: [(&str, &str); 8] = [
    ("Jasper", "expr-voice-2-m"),
    ("Bella", "expr-voice-2-f"),
    ("Bruno", "expr-voice-3-m"),
    ("Luna", "expr-voice-3-f"),
    ("Hugo", "expr-voice-4-m"),
    ("Rosie", "expr-voice-4-f"),
    ("Leo", "expr-voice-5-m"),
    ("Kiki", "expr-voice-5-f"),
];

pub const DEFAULT_KITTEN_VOICE: &str = "Bella";

/// Speaker id for a Kokoro voice name, falling back to Heart for anything this
/// model does not know (a hand-edited setting, a voice from a newer build).
fn kokoro_sid(voice: &str) -> i32 {
    let voice = voice.trim();
    KOKORO_VOICES
        .iter()
        .position(|name| *name == voice)
        .or_else(|| {
            KOKORO_VOICES
                .iter()
                .position(|n| *n == DEFAULT_KOKORO_VOICE)
        })
        .unwrap_or(0) as i32
}

/// British voices are phonemized with eSpeak's British English (`en`); every
/// other English voice uses the model's default `en-us`.
fn kokoro_is_british(voice: &str) -> bool {
    let voice = voice.trim();
    voice.starts_with("bf_") || voice.starts_with("bm_")
}

/// Speaker id for a Kitten voice, by friendly name or `expr-voice-*` id,
/// case-insensitively. Unknown names get Bella.
fn kitten_sid(voice: &str) -> i32 {
    let voice = voice.trim();
    let find = |v: &str| {
        KITTEN_VOICES
            .iter()
            .position(|(name, id)| name.eq_ignore_ascii_case(v) || id.eq_ignore_ascii_case(v))
    };
    find(voice)
        .or_else(|| find(DEFAULT_KITTEN_VOICE))
        .unwrap_or(0) as i32
}

/// Speaker id for a Supertonic voice (`F1`…`M5`, case-insensitive). Unknown
/// names get F1.
fn supertonic_sid(voice: &str) -> i32 {
    let voice = voice.trim();
    SUPERTONIC_VOICES
        .iter()
        .position(|name| name.eq_ignore_ascii_case(voice))
        .unwrap_or(0) as i32
}

/// The reference clip a Pocket voice name refers to (case-insensitive).
/// Unknown names, including voices earlier builds offered and no longer do,
/// get the default.
fn pocket_clip(voice: &str) -> &'static VoiceClip {
    let voice = voice.trim();
    POCKET_VOICES
        .iter()
        .find(|clip| clip.name.eq_ignore_ascii_case(voice))
        .or_else(|| {
            POCKET_VOICES
                .iter()
                .find(|clip| clip.name == DEFAULT_POCKET_VOICE)
        })
        .unwrap_or(&POCKET_VOICES[0])
}

/// The voice names a native engine offers, for the settings picker.
pub fn voice_names(engine: &str) -> Vec<&'static str> {
    match engine.trim() {
        "kitten" => KITTEN_VOICES.iter().map(|(name, _)| *name).collect(),
        "pocket" => POCKET_VOICES.iter().map(|clip| clip.name).collect(),
        "supertonic" => SUPERTONIC_VOICES.to_vec(),
        _ => Vec::new(),
    }
}

/// Flow-matching steps. Pocket's is Kyutai's own default: on the quality
/// matrix, 1 step at temperature 0.3 had the lowest word error rate and never
/// dropped a sentence, while 5 steps (sherpa-onnx's default) sometimes ended a
/// clip after one word. Supertonic's is Supertone's default.
const POCKET_STEPS: i32 = 1;
const SUPERTONIC_STEPS: i32 = 5;

/// Longest stretch of a reference clip Pocket listens to. Its own default; a
/// longer clip is trimmed rather than refused.
const POCKET_MAX_REFERENCE_SECS: f32 = 10.0;

/// Pocket's sampling temperature. sherpa-onnx defaults to 0.7; Kyutai's own
/// runtime uses 0.3, and at 0.7 the model skipped or repeated words far more
/// often (see the quality matrix test).
const POCKET_TEMPERATURE: f32 = 0.3;

fn pocket_temperature() -> f32 {
    #[cfg(test)]
    if let Some(n) = std::env::var("SPEAKOFLOW_TTS_TEMP")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return n;
    }
    POCKET_TEMPERATURE
}

/// The speed the model is asked for, from the app's 0.25–4x setting.
fn model_speed(pack: &VoicePack, user_speed: f64) -> f32 {
    let speed = if user_speed.is_finite() {
        user_speed
    } else {
        1.0
    }
    .clamp(0.25, 4.0);
    (speed * pack.pace).clamp(0.25, 4.0) as f32
}

/// Intra-op threads for a pack on a machine with `logical` CPUs.
///
/// Half the logical CPUs, i.e. the physical cores on an SMT machine: on a
/// 16-thread Ryzen, Kokoro measured RTF 0.27 at 4 threads, 0.20 at 8, and got
/// slower beyond. Each pack caps it where more threads stopped helping.
fn threads_for(pack: &VoicePack, logical: usize) -> i32 {
    let physical = (logical / 2).max(1);
    physical.min(pack.max_threads) as i32
}

/// Read a reference clip as mono samples at its own rate. The engine
/// resamples to 24 kHz itself.
fn read_clip(path: &Path) -> Result<(Vec<f32>, i32), String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| format!("Couldn't read the voice recording: {e}"))?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels.max(1));
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| format!("Couldn't read the voice recording: {e}"))?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1_i64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .map_err(|e| format!("Couldn't read the voice recording: {e}"))?
        }
    };
    let mono = if channels == 1 {
        interleaved
    } else {
        interleaved
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    };
    if mono.is_empty() || spec.sample_rate == 0 {
        return Err("The voice recording is empty".to_string());
    }
    Ok((mono, spec.sample_rate as i32))
}

fn logical_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// Text as the engine should see it: control characters and runs of whitespace
/// collapsed to single spaces. The speech pipeline has already removed Markdown
/// and emoji; this only guards the C boundary (an interior NUL would truncate).
fn engine_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() || c.is_control() {
            if !out.is_empty() {
                space = true;
            }
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// FFI: mirrors sherpa-onnx v1.13.8 `c-api.h`, field for field
// ---------------------------------------------------------------------------

#[repr(C)]
struct VitsModelConfig {
    model: *const c_char,
    lexicon: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    noise_scale: f32,
    noise_scale_w: f32,
    length_scale: f32,
    dict_dir: *const c_char,
}

#[repr(C)]
struct MatchaModelConfig {
    acoustic_model: *const c_char,
    vocoder: *const c_char,
    lexicon: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    noise_scale: f32,
    length_scale: f32,
    dict_dir: *const c_char,
}

#[repr(C)]
struct KokoroModelConfig {
    model: *const c_char,
    voices: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    length_scale: f32,
    dict_dir: *const c_char,
    lexicon: *const c_char,
    lang: *const c_char,
}

#[repr(C)]
struct KittenModelConfig {
    model: *const c_char,
    voices: *const c_char,
    tokens: *const c_char,
    data_dir: *const c_char,
    length_scale: f32,
}

#[repr(C)]
struct ZipvoiceModelConfig {
    tokens: *const c_char,
    encoder: *const c_char,
    decoder: *const c_char,
    vocoder: *const c_char,
    data_dir: *const c_char,
    lexicon: *const c_char,
    feat_scale: f32,
    t_shift: f32,
    target_rms: f32,
    guidance_scale: f32,
}

#[repr(C)]
struct PocketModelConfig {
    lm_flow: *const c_char,
    lm_main: *const c_char,
    encoder: *const c_char,
    decoder: *const c_char,
    text_conditioner: *const c_char,
    vocab_json: *const c_char,
    token_scores_json: *const c_char,
    voice_embedding_cache_capacity: i32,
}

#[repr(C)]
struct SupertonicModelConfig {
    duration_predictor: *const c_char,
    text_encoder: *const c_char,
    vector_estimator: *const c_char,
    vocoder: *const c_char,
    tts_json: *const c_char,
    unicode_indexer: *const c_char,
    voice_style: *const c_char,
}

#[repr(C)]
struct TtsModelConfig {
    vits: VitsModelConfig,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
    matcha: MatchaModelConfig,
    kokoro: KokoroModelConfig,
    kitten: KittenModelConfig,
    zipvoice: ZipvoiceModelConfig,
    pocket: PocketModelConfig,
    supertonic: SupertonicModelConfig,
}

#[repr(C)]
struct TtsConfig {
    model: TtsModelConfig,
    rule_fsts: *const c_char,
    max_num_sentences: i32,
    rule_fars: *const c_char,
    silence_scale: f32,
}

#[repr(C)]
struct GenerationConfig {
    silence_scale: f32,
    speed: f32,
    sid: i32,
    reference_audio: *const f32,
    reference_audio_len: i32,
    reference_sample_rate: i32,
    reference_text: *const c_char,
    num_steps: i32,
    extra: *const c_char,
}

#[repr(C)]
struct GeneratedAudio {
    samples: *const f32,
    n: i32,
    sample_rate: i32,
}

type ProgressCallback = unsafe extern "C" fn(*const f32, i32, f32, *mut c_void) -> i32;

const CPU_PROVIDER: &CStr = c"cpu";
const KOKORO_LANG: &CStr = c"en-us";

struct Api {
    version: unsafe extern "C" fn() -> *const c_char,
    create: unsafe extern "C" fn(*const TtsConfig) -> *const c_void,
    destroy: unsafe extern "C" fn(*const c_void),
    sample_rate: unsafe extern "C" fn(*const c_void) -> i32,
    generate: unsafe extern "C" fn(
        *const c_void,
        *const c_char,
        *const GenerationConfig,
        Option<ProgressCallback>,
        *mut c_void,
    ) -> *const GeneratedAudio,
    destroy_audio: unsafe extern "C" fn(*const GeneratedAudio),
}

/// The loaded libraries. Kept for the life of the process once loaded: ONNX
/// Runtime owns thread pools and thread-local state, and unloading it under a
/// live process is not something it supports.
struct Runtime {
    api: Api,
    _ort: libloading::Library,
    _c_api: libloading::Library,
}

static RUNTIME_LIB: Lazy<Mutex<Option<Arc<Runtime>>>> = Lazy::new(|| Mutex::new(None));

#[cfg(target_os = "windows")]
unsafe fn open_library(path: &Path) -> Result<libloading::Library, libloading::Error> {
    use libloading::os::windows::{Library, LOAD_WITH_ALTERED_SEARCH_PATH};
    // Resolve the library's own imports from its folder first, so
    // sherpa-onnx-c-api.dll binds to the onnxruntime.dll beside it and never to
    // the unrelated copy Windows ships in System32.
    Library::load_with_flags(path, LOAD_WITH_ALTERED_SEARCH_PATH).map(Into::into)
}

#[cfg(unix)]
unsafe fn open_library(path: &Path) -> Result<libloading::Library, libloading::Error> {
    use libloading::os::unix::{Library, RTLD_LOCAL, RTLD_NOW};
    Library::open(Some(path), RTLD_NOW | RTLD_LOCAL).map(Into::into)
}

/// Load the runtime once; later calls return the same handle.
fn runtime() -> Result<Arc<Runtime>, String> {
    let mut guard = RUNTIME_LIB
        .lock()
        .map_err(|_| "The voice engine lock was poisoned".to_string())?;
    if let Some(runtime) = guard.as_ref() {
        return Ok(runtime.clone());
    }
    let dir = models_dir()
        .map(runtime_dir_in)
        .ok_or_else(|| "The voice engine isn't set up yet".to_string())?;
    if !dir.join(ORT_LIB).is_file() || !dir.join(C_API_LIB).is_file() {
        return Err(
            "The voice engine isn't downloaded. Download the voice again in Models → Voice."
                .to_string(),
        );
    }
    // SAFETY: both libraries come from the pinned, hash-verified sherpa-onnx
    // release; their initializers only set up ONNX Runtime's own state. The
    // symbol types below are the declarations from that release's c-api.h.
    let loaded = unsafe { load_runtime_from(&dir) };
    let runtime = match loaded {
        Ok(runtime) => runtime,
        Err(e) => {
            // Files present but unusable: stop routing voices here until the
            // next launch (or a fresh download), so replies fall back instead
            // of failing one after another.
            if !LOAD_FAILED.swap(true, Ordering::Relaxed) {
                warn!("Native voice engine failed to load; using the in-app voice instead: {e}");
                drop(guard);
                notify_status();
            }
            return Err(e);
        }
    };
    info!("Native voice engine loaded (sherpa-onnx {SHERPA_VERSION})");
    let runtime = Arc::new(runtime);
    *guard = Some(runtime.clone());
    drop(guard);
    if LOAD_FAILED.swap(false, Ordering::Relaxed) {
        notify_status();
    }
    Ok(runtime)
}

/// Open both libraries in `dir`, resolve the C API, and check its version.
///
/// # Safety
/// `dir` must hold the pinned sherpa-onnx release (see [`runtime`]).
unsafe fn load_runtime_from(dir: &Path) -> Result<Runtime, String> {
    let ort = open_library(&dir.join(ORT_LIB))
        .map_err(|e| format!("Couldn't load the voice engine ({ORT_LIB}): {e}"))?;
    let c_api = open_library(&dir.join(C_API_LIB))
        .map_err(|e| format!("Couldn't load the voice engine ({C_API_LIB}): {e}"))?;
    macro_rules! symbol {
        ($name:literal) => {
            *c_api
                .get($name)
                .map_err(|e| format!("The voice engine is missing a function: {e}"))?
        };
    }
    let api = Api {
        version: symbol!(b"SherpaOnnxGetVersionStr\0"),
        create: symbol!(b"SherpaOnnxCreateOfflineTts\0"),
        destroy: symbol!(b"SherpaOnnxDestroyOfflineTts\0"),
        sample_rate: symbol!(b"SherpaOnnxOfflineTtsSampleRate\0"),
        generate: symbol!(b"SherpaOnnxOfflineTtsGenerateWithConfig\0"),
        destroy_audio: symbol!(b"SherpaOnnxDestroyOfflineTtsGeneratedAudio\0"),
    };
    let version_ptr = (api.version)();
    let version = if version_ptr.is_null() {
        String::new()
    } else {
        CStr::from_ptr(version_ptr).to_string_lossy().into_owned()
    };
    if version != SHERPA_VERSION {
        return Err(format!(
            "The voice engine on disk is version {version:?}, but this app needs {SHERPA_VERSION}. Download the voice again in Models → Voice."
        ));
    }
    Ok(Runtime {
        api,
        _ort: ort,
        _c_api: c_api,
    })
}

/// A path the engine can open. The C API takes narrow strings, which the
/// Windows build opens in the ANSI code page, so a profile folder with a
/// non-ASCII name would fail to load. The 8.3 short name is ASCII and opens the
/// same file.
fn engine_path(path: &Path) -> Result<CString, String> {
    let text = path.to_string_lossy().into_owned();
    #[cfg(target_os = "windows")]
    if !text.is_ascii() {
        if let Some(short) = short_path(path) {
            return CString::new(short).map_err(|_| "Invalid path".to_string());
        }
        warn!("Native voice path is not ASCII and has no short name: {text}");
    }
    CString::new(text).map_err(|_| "Invalid path".to_string())
}

#[cfg(target_os = "windows")]
fn short_path(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long_path: *const u16, short_path: *mut u16, len: u32) -> u32;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buffer = vec![0u16; 1024];
    // SAFETY: `wide` is NUL-terminated and `buffer` is writable for its length.
    let len = unsafe { GetShortPathNameW(wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) };
    if len == 0 || len as usize >= buffer.len() {
        return None;
    }
    let short = String::from_utf16_lossy(&buffer[..len as usize]);
    short.is_ascii().then_some(short)
}

// ---------------------------------------------------------------------------
// The loaded engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EngineKey {
    pack: &'static str,
    threads: i32,
}

struct Engine {
    handle: *const c_void,
    key: EngineKey,
    runtime: Arc<Runtime>,
}

// SAFETY: a sherpa-onnx OfflineTts handle may be used from any thread; this
// module only ever uses it under the `ENGINE` mutex, so never concurrently.
unsafe impl Send for Engine {}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: `handle` came from this runtime's create and is destroyed once.
        unsafe { (self.runtime.api.destroy)(self.handle) };
        debug!("Native voice released ({})", self.key.pack);
    }
}

struct Loaded {
    engine: Engine,
    last_used: Instant,
    /// Pocket's last reference clip, decoded, so a reply does not re-read the
    /// WAV for every sentence.
    reference: Option<(&'static str, Arc<(Vec<f32>, i32)>)>,
}

static ENGINE: Lazy<Mutex<Option<Loaded>>> = Lazy::new(|| Mutex::new(None));
static WATCHER_STARTED: AtomicBool = AtomicBool::new(false);
/// A [`release`] that arrived while a synthesis held the engine. It cannot wait
/// (its callers run on the main thread), so whoever holds the engine next
/// drops it: the synthesis on its way out, or the idle watcher.
static RELEASE_PENDING: AtomicBool = AtomicBool::new(false);

/// Release the engine after [`IDLE_UNLOAD`] without use. `try_lock` so the
/// watcher never waits behind a synthesis in progress, which is use anyway.
fn ensure_idle_watcher() {
    if WATCHER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("native-voice-idle".into())
        .spawn(|| loop {
            std::thread::sleep(Duration::from_secs(15));
            if let Ok(mut guard) = ENGINE.try_lock() {
                let pending = RELEASE_PENDING.swap(false, Ordering::SeqCst);
                let idle = guard
                    .as_ref()
                    .is_some_and(|loaded| loaded.last_used.elapsed() >= IDLE_UNLOAD);
                if guard.is_some() && (pending || idle) {
                    info!(
                        "Native voice {}; releasing its memory",
                        if idle { "idle" } else { "no longer used" }
                    );
                    *guard = None;
                }
            }
        });
    if spawned.is_err() {
        WATCHER_STARTED.store(false, Ordering::SeqCst);
    }
}

fn create_engine(
    runtime: &Arc<Runtime>,
    pack: &'static VoicePack,
    threads: i32,
) -> Result<Engine, String> {
    let dir = models_dir()
        .map(|models| models.join(pack.dir))
        .ok_or_else(|| "The voice isn't set up yet".to_string())?;
    // Checked here rather than left to the engine: some of its missing-file
    // paths end the whole process instead of returning an error.
    if !pack
        .required_files()
        .iter()
        .all(|file| dir.join(file).exists())
    {
        return Err(
            "This voice's files are missing. Download it again in Models → Voice.".to_string(),
        );
    }
    // Every path the config points at, kept alive until the engine has read
    // them. Pushing moves the CString, not its heap buffer, so the pointers
    // taken below stay valid.
    let mut strings: Vec<CString> = Vec::new();
    let mut path = |name: &str| -> Result<*const c_char, String> {
        let c = engine_path(&dir.join(name))?;
        let ptr = c.as_ptr();
        strings.push(c);
        Ok(ptr)
    };

    // SAFETY: a zeroed config is the documented starting point (every pointer
    // null, every number zero); only the chosen family's block is filled in.
    let mut config: TtsConfig = unsafe { std::mem::zeroed() };
    match &pack.layout {
        Layout::Espeak { model } => {
            let model = path(model)?;
            let voices = path("voices.bin")?;
            let tokens = path("tokens.txt")?;
            let data_dir = path("espeak-ng-data")?;
            if pack.family == VoiceFamily::Kokoro {
                config.model.kokoro.model = model;
                config.model.kokoro.voices = voices;
                config.model.kokoro.tokens = tokens;
                config.model.kokoro.data_dir = data_dir;
                config.model.kokoro.length_scale = 1.0;
                // Kokoro v1.0 is multilingual and needs a lexicon or a
                // language. English goes through eSpeak whenever a language is
                // set, so the 6 MB lexicon adds nothing but load time
                // (measured: 1.5 s → 1.1 s, 42 MB less memory).
                config.model.kokoro.lang = KOKORO_LANG.as_ptr();
            } else {
                config.model.kitten.model = model;
                config.model.kitten.voices = voices;
                config.model.kitten.tokens = tokens;
                config.model.kitten.data_dir = data_dir;
                config.model.kitten.length_scale = 1.0;
            }
        }
        Layout::Pocket {
            lm_flow,
            lm_main,
            encoder,
            decoder,
            text_conditioner,
        } => {
            config.model.pocket.lm_flow = path(lm_flow)?;
            config.model.pocket.lm_main = path(lm_main)?;
            config.model.pocket.encoder = path(encoder)?;
            config.model.pocket.decoder = path(decoder)?;
            config.model.pocket.text_conditioner = path(text_conditioner)?;
            config.model.pocket.vocab_json = path("vocab.json")?;
            config.model.pocket.token_scores_json = path("token_scores.json")?;
            // The engine turns a reference clip into a voice embedding and
            // keeps the last few, so switching between voices is instant after
            // the first reply in each.
            config.model.pocket.voice_embedding_cache_capacity = 4;
        }
        Layout::Supertonic {
            duration_predictor,
            text_encoder,
            vector_estimator,
            vocoder,
        } => {
            config.model.supertonic.duration_predictor = path(duration_predictor)?;
            config.model.supertonic.text_encoder = path(text_encoder)?;
            config.model.supertonic.vector_estimator = path(vector_estimator)?;
            config.model.supertonic.vocoder = path(vocoder)?;
            config.model.supertonic.tts_json = path("tts.json")?;
            config.model.supertonic.unicode_indexer = path("unicode_indexer.bin")?;
            config.model.supertonic.voice_style = path("voice.bin")?;
        }
    }
    config.model.num_threads = threads;
    config.model.provider = CPU_PROVIDER.as_ptr();
    config.max_num_sentences = 1;
    config.silence_scale = 0.2;

    let started = Instant::now();
    // SAFETY: every pointer in `config` is either null, a static C string, or
    // a CString in `strings`, all alive until after this call; the engine
    // copies what it keeps.
    let handle = unsafe { (runtime.api.create)(&config) };
    drop(strings);
    if handle.is_null() {
        // The library loaded but refused these files. Retrying on every reply
        // cannot help, so the pack reads as not ready until it is reinstalled
        // and Kokoro falls back to the WebView.
        if set_pack_failed(pack) {
            warn!(
                "Native voice {} failed to start; not using it this session",
                pack.model_id
            );
        }
        return Err(
            "The local voice couldn't start. Try downloading it again in Models → Voice."
                .to_string(),
        );
    }
    info!(
        "Native voice loaded: {} with {} threads in {:?}",
        pack.model_id,
        threads,
        started.elapsed()
    );
    Ok(Engine {
        handle,
        key: EngineKey {
            pack: pack.model_id,
            threads,
        },
        runtime: runtime.clone(),
    })
}

/// Lets a Stop end synthesis at the next sentence instead of after the whole
/// piece: the engine calls this after every sentence and stops on 0.
unsafe extern "C" fn keep_going(
    _samples: *const f32,
    _n: i32,
    _progress: f32,
    arg: *mut c_void,
) -> i32 {
    // SAFETY: `arg` points at the `u64` epoch owned by `synthesize_pcm`'s frame,
    // which outlives the generate call that invokes this.
    let epoch = unsafe { *(arg as *const u64) };
    i32::from(crate::tts::current_epoch() == epoch)
}

/// One request for the native engine.
#[derive(Debug, Clone)]
pub struct NativeRequest {
    pub pack: &'static VoicePack,
    pub voice: String,
    pub speed: f64,
}

impl NativeRequest {
    /// The request these settings describe, when their route is native.
    pub fn from_settings(settings: &AppSettings) -> Option<Self> {
        let pack = active_pack(settings)?;
        let voice = match pack.family {
            VoiceFamily::Kokoro => settings.assistant_tts_voice.clone(),
            // Every native-only engine keeps its voice in the per-engine slot.
            _ => settings.assistant_tts_remote_voice.clone(),
        };
        Some(Self {
            pack,
            voice,
            speed: settings.assistant_tts_speed,
        })
    }
}

/// Threads for this pack here. Tests can pin it to measure a curve.
fn engine_threads(pack: &VoicePack) -> i32 {
    #[cfg(test)]
    if let Some(n) = std::env::var("SPEAKOFLOW_TTS_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return n;
    }
    threads_for(pack, logical_cpus())
}

/// Flow-matching steps for a family, which tests can override.
fn flow_steps(default: i32) -> i32 {
    #[cfg(test)]
    if let Some(n) = std::env::var("SPEAKOFLOW_TTS_STEPS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return n;
    }
    default
}

/// How much of each pause between phrases is kept. Tests can override it.
///
/// sherpa-onnx shortens every interior pause of 0.2 s or more by this factor.
/// For the voices trained on read speech that tightens delivery; Pocket's
/// pauses come from its reference recording, and Kyutai's runtime leaves them
/// alone, so it keeps them whole.
fn silence_scale(family: VoiceFamily) -> f32 {
    #[cfg(test)]
    if let Some(n) = std::env::var("SPEAKOFLOW_TTS_SILENCE")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return n;
    }
    match family {
        VoiceFamily::Pocket => 1.0,
        _ => 0.2,
    }
}

/// Pocket stops when its end-of-speech signal fires, and on short or odd text
/// it sometimes never does: the engine then generates up to its 500-frame
/// ceiling, 40 seconds of breathing and repeated words. Measured on the app's
/// own test lines and short replies ("Sure.", "Got it"), that happened on most
/// of them. Kyutai pads short prompts with spaces to avoid it; sherpa-onnx
/// drops the spaces, so the padding changes nothing here (identical output).
///
/// What does work is a ceiling scaled to the text. Mimi produces 12.5 frames a
/// second and the slowest clean voice speaks ~14.5 characters a second, 0.86
/// frames a character, so 1.4 frames a character plus a second of slack never
/// cuts real speech short. With it the longest of 80 short clips was 4.3 s.
const POCKET_FRAMES_BASE: f32 = 12.0;
const POCKET_FRAMES_PER_CHAR: f32 = 1.4;
/// sherpa-onnx's own ceiling.
const POCKET_MAX_FRAMES: i32 = 500;
/// sherpa-onnx joins sentences shorter than this before generating them
/// (`min_char_in_sentence`); pieces are cut the same way so that each call is
/// exactly one of its chunks and the ceiling fits it.
const POCKET_MIN_PIECE_CHARS: usize = 30;

/// Frame ceiling for one piece of text.
fn pocket_max_frames(piece: &str) -> i32 {
    let chars = piece.chars().count() as f32;
    ((POCKET_FRAMES_BASE + POCKET_FRAMES_PER_CHAR * chars).ceil() as i32).min(POCKET_MAX_FRAMES)
}

/// Split text into the pieces Pocket speaks one call at a time: sentences,
/// with short ones joined to the next, each starting with a capital and ending
/// with punctuation as Kyutai's own text preparation does (an unterminated
/// sentence is one of the inputs whose end signal never fires).
///
/// A sentence ends at `.`, `!`, `?` or `…` followed by a space or the end, so
/// "2.5" stays whole; an abbreviation that does end a "sentence" ("Dr.") is
/// too short to stand alone and is joined back to what follows.
fn pocket_pieces(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();
    for (i, c) in chars.iter().enumerate() {
        current.push(*c);
        let ends = matches!(c, '.' | '!' | '?' | '…')
            && chars.get(i + 1).is_none_or(|next| next.is_whitespace());
        if ends {
            sentences.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        sentences.push(current.trim().to_string());
    }

    let mut pieces: Vec<String> = Vec::new();
    for sentence in sentences.into_iter().filter(|s| !s.is_empty()) {
        match pieces.last_mut() {
            Some(last) if last.chars().count() < POCKET_MIN_PIECE_CHARS => {
                last.push(' ');
                last.push_str(&sentence);
            }
            _ => pieces.push(sentence),
        }
    }
    // A short last sentence joins the one before it rather than standing alone.
    if pieces.len() > 1
        && pieces
            .last()
            .is_some_and(|last| last.chars().count() < POCKET_MIN_PIECE_CHARS)
    {
        let tail = pieces.pop().expect("checked above");
        let last = pieces.last_mut().expect("checked above");
        last.push(' ');
        last.push_str(&tail);
    }
    pieces.into_iter().map(|piece| tidy_piece(&piece)).collect()
}

/// Capitalize the first letter and make sure the piece ends like a sentence.
fn tidy_piece(piece: &str) -> String {
    let mut out = String::with_capacity(piece.len() + 1);
    let mut capitalized = false;
    for c in piece.chars() {
        if !capitalized && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            capitalized = true;
        } else {
            out.push(c);
        }
    }
    let ends_sentence = out
        .trim_end_matches(['"', '\'', ')', ']', '\u{201d}', '\u{2019}'])
        .ends_with(['.', '!', '?', '…']);
    if !ends_sentence {
        out.push('.');
    }
    out
}

/// Synthesize `text` into mono samples. Blocking; call off the async runtime.
/// Returns `Ok(None)` when a Stop superseded the request mid-way.
fn synthesize_pcm(request: &NativeRequest, text: &str) -> Result<Option<(Vec<f32>, u32)>, String> {
    let text = engine_text(text);
    if text.is_empty() {
        return Err("There was nothing to say".to_string());
    }
    let epoch = crate::tts::current_epoch();
    let runtime = runtime()?;
    let failed_before = pack_failed(request.pack);
    let result = {
        let mut guard = ENGINE
            .lock()
            .map_err(|_| "The voice engine lock was poisoned".to_string())?;
        let result = synthesize_locked(&mut guard, &runtime, request, &text, epoch);
        // A release asked for while this held the engine (a voice switch, or
        // Runs on changing) could not wait on its caller's thread; honour it
        // now that the work is done.
        if RELEASE_PENDING.swap(false, Ordering::SeqCst) {
            *guard = None;
        }
        result
    };
    if !failed_before && pack_failed(request.pack) {
        // This pack just refused to start: tell the windows, so the next reply
        // is routed back to the WebView.
        notify_status();
    }
    result
}

/// The part of [`synthesize_pcm`] that runs with the engine slot locked.
fn synthesize_locked(
    slot: &mut Option<Loaded>,
    runtime: &Arc<Runtime>,
    request: &NativeRequest,
    text: &str,
    epoch: u64,
) -> Result<Option<(Vec<f32>, u32)>, String> {
    let family = request.pack.family;
    let key = EngineKey {
        pack: request.pack.model_id,
        threads: engine_threads(request.pack),
    };
    if slot.as_ref().is_none_or(|loaded| loaded.engine.key != key) {
        // Drop the old engine before creating the next, so two are never
        // resident at once.
        *slot = None;
        let engine = create_engine(runtime, request.pack, key.threads)?;
        *slot = Some(Loaded {
            engine,
            last_used: Instant::now(),
            reference: None,
        });
        ensure_idle_watcher();
    }
    let loaded = slot.as_mut().expect("engine loaded above");

    // SAFETY: zeroed is the documented default; the fields set are plain values
    // and pointers that outlive the calls below.
    let mut generation: GenerationConfig = unsafe { std::mem::zeroed() };
    generation.silence_scale = silence_scale(family);
    generation.speed = model_speed(request.pack, request.speed);
    // Kept alive until after the last generate call.
    let mut extra: Option<CString> = None;
    let mut reference: Option<Arc<(Vec<f32>, i32)>> = None;
    match family {
        VoiceFamily::Kokoro => {
            generation.sid = kokoro_sid(&request.voice);
            if kokoro_is_british(&request.voice) {
                extra = Some(CString::from(c"{\"lang\":\"en\"}"));
            }
        }
        VoiceFamily::Kitten => generation.sid = kitten_sid(&request.voice),
        VoiceFamily::Supertonic => {
            generation.sid = supertonic_sid(&request.voice);
            generation.num_steps = flow_steps(SUPERTONIC_STEPS);
            extra = Some(CString::from(c"{\"lang\":\"en\"}"));
        }
        VoiceFamily::Pocket => {
            let clip = pocket_clip(&request.voice);
            let cached = loaded
                .reference
                .as_ref()
                .filter(|(name, _)| *name == clip.name)
                .map(|(_, samples)| samples.clone());
            let samples = match cached {
                Some(samples) => samples,
                None => {
                    let path = models_dir()
                        .map(|models| models.join(request.pack.dir).join(clip.relative_path()))
                        .ok_or_else(|| "The voice isn't set up yet".to_string())?;
                    let samples = Arc::new(read_clip(&path)?);
                    loaded.reference = Some((clip.name, samples.clone()));
                    samples
                }
            };
            generation.reference_audio = samples.0.as_ptr();
            generation.reference_audio_len = samples.0.len() as i32;
            generation.reference_sample_rate = samples.1;
            generation.num_steps = flow_steps(POCKET_STEPS);
            reference = Some(samples);
        }
    }

    // Pocket is called once per piece so each gets a ceiling that fits it;
    // everything else takes the text whole.
    let calls: Vec<(String, Option<CString>)> = if family == VoiceFamily::Pocket {
        let temperature = pocket_temperature();
        pocket_pieces(text)
            .into_iter()
            .map(|piece| {
                let json = format!(
                    "{{\"max_reference_audio_len\": {POCKET_MAX_REFERENCE_SECS}, \"temperature\": {temperature}, \"max_frames\": {}}}",
                    pocket_max_frames(&piece)
                );
                let json = CString::new(json).expect("no NUL in formatted JSON");
                (piece, Some(json))
            })
            .collect()
    } else {
        vec![(text.to_string(), extra.take())]
    };

    let started = Instant::now();
    let mut samples: Vec<f32> = Vec::new();
    let mut rate = 0_i32;
    for (piece, piece_extra) in &calls {
        generation.extra = piece_extra
            .as_ref()
            .map_or(std::ptr::null(), |e| e.as_ptr());
        match generate_once(loaded, piece, &generation, epoch)? {
            None => return Ok(None),
            Some((piece_samples, piece_rate)) => {
                samples.extend_from_slice(&piece_samples);
                rate = piece_rate;
            }
        }
    }
    drop(reference);
    let rate = if rate > 0 {
        rate as u32
    } else {
        // SAFETY: live handle under the lock.
        unsafe { (loaded.engine.runtime.api.sample_rate)(loaded.engine.handle) }.max(1) as u32
    };
    debug!(
        "Native voice: {:.2}s of audio in {:?}",
        samples.len() as f64 / rate as f64,
        started.elapsed()
    );
    if samples.is_empty() {
        return Err("The local voice produced no audio".to_string());
    }
    Ok(Some((samples, rate)))
}

/// One call into the engine. `Ok(None)` when a Stop superseded it.
fn generate_once(
    loaded: &mut Loaded,
    text: &str,
    generation: &GenerationConfig,
    epoch: u64,
) -> Result<Option<(Vec<f32>, i32)>, String> {
    let c_text = CString::new(text).map_err(|_| "Invalid text".to_string())?;
    let mut epoch_arg: u64 = epoch;
    // SAFETY: the handle is live under the caller's lock; `c_text`, the
    // config (and every pointer in it, which the caller keeps alive) and
    // `epoch_arg` outlive the call; `keep_going` matches the callback type.
    let audio = unsafe {
        (loaded.engine.runtime.api.generate)(
            loaded.engine.handle,
            c_text.as_ptr(),
            generation,
            Some(keep_going),
            &mut epoch_arg as *mut u64 as *mut c_void,
        )
    };
    loaded.last_used = Instant::now();
    if crate::tts::current_epoch() != epoch {
        if !audio.is_null() {
            // SAFETY: returned by generate, freed once.
            unsafe { (loaded.engine.runtime.api.destroy_audio)(audio) };
        }
        return Ok(None);
    }
    if audio.is_null() {
        return Err("The local voice couldn't read that text aloud".to_string());
    }
    // SAFETY: a non-null result owns `n` samples until destroy_audio.
    let result = unsafe {
        let result = &*audio;
        let samples = if result.samples.is_null() || result.n <= 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(result.samples, result.n as usize).to_vec()
        };
        let rate = result.sample_rate;
        (loaded.engine.runtime.api.destroy_audio)(audio);
        (samples, rate)
    };
    Ok(Some(result))
}

/// Encode mono float samples as a 16-bit PCM WAV file.
fn samples_to_wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        pcm.extend_from_slice(&value.to_le_bytes());
    }
    crate::tts::pcm_to_wav(&pcm, rate, 1, 16)
}

/// Synthesize `text` to WAV bytes. Blocking. `Ok(empty)` means a Stop won.
pub fn synthesize_wav(request: &NativeRequest, text: &str) -> Result<Vec<u8>, String> {
    Ok(synthesize_pcm(request, text)?
        .map(|(samples, rate)| samples_to_wav(&samples, rate))
        .unwrap_or_default())
}

/// Load the engine for these settings in the background, so the first reply of
/// a call does not wait for it. A two-word synthesis also pays ONNX Runtime's
/// first-run cost (~1 s on Kokoro) before anyone is listening.
pub fn prewarm(settings: &AppSettings) {
    let Some(request) = NativeRequest::from_settings(settings) else {
        return;
    };
    if !pack_ready(request.pack) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("native-voice-prewarm".into())
        .spawn(move || {
            let started = Instant::now();
            match synthesize_pcm(&request, "Hi.") {
                Ok(_) => debug!("Native voice warmed in {:?}", started.elapsed()),
                Err(e) => warn!("Native voice warm-up failed: {e}"),
            }
        });
}

/// Release the engine now if it belongs to `model_id`, or unconditionally when
/// `model_id` is `None`. Never waits: callers run on the main thread, and a
/// synthesis can hold the engine for a second or more (loading it cannot be
/// interrupted), so a busy engine is dropped by that synthesis when it ends.
pub fn release(model_id: Option<&str>) {
    match ENGINE.try_lock() {
        Ok(mut guard) => {
            if guard
                .as_ref()
                .is_some_and(|loaded| model_id.is_none_or(|id| loaded.engine.key.pack == id))
            {
                *guard = None;
            }
        }
        Err(std::sync::TryLockError::WouldBlock) => {
            RELEASE_PENDING.store(true, Ordering::SeqCst);
        }
        Err(std::sync::TryLockError::Poisoned(poisoned)) => {
            *poisoned.into_inner() = None;
        }
    }
}

/// Run `remove` (deleting `model_id`'s files) with the engine slot held and that
/// pack unloaded, so no synthesis can start loading it from files that are
/// going away. Waits for a synthesis in progress (one sentence or one engine
/// load at most), so call it off the main thread.
pub fn with_pack_unloaded<T>(model_id: &str, remove: impl FnOnce() -> T) -> T {
    let mut guard = ENGINE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard
        .as_ref()
        .is_some_and(|loaded| loaded.engine.key.pack == model_id)
    {
        *guard = None;
    }
    let result = remove();
    forget_pack_failure(model_id);
    drop(guard);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A library that cannot be opened is an error that names it, never a
    /// crash. This is the path macOS library validation takes, and it is what
    /// arms the fallback in [`runtime`].
    #[test]
    fn a_runtime_that_cannot_be_opened_is_a_named_error() {
        let dir = std::env::temp_dir().join(format!(
            "speakoflow-native-tts-empty-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: nothing in the folder can be loaded, so no foreign code runs.
        let error = match unsafe { load_runtime_from(&dir) } {
            Ok(_) => panic!("an empty folder must not load"),
            Err(error) => error,
        };
        assert!(error.contains(ORT_LIB), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A pack whose engine refused to start stays out of the route until it is
    /// reinstalled or removed, and is reported once.
    #[test]
    fn a_pack_that_failed_to_start_is_not_ready_until_reinstalled() {
        assert!(!pack_failed(&KOKORO_PACK));
        assert!(set_pack_failed(&KOKORO_PACK), "first failure is new");
        assert!(!set_pack_failed(&KOKORO_PACK), "and only reported once");
        assert!(pack_failed(&KOKORO_PACK));
        assert!(!pack_ready(&KOKORO_PACK));
        assert!(!pack_failed(&KITTEN_PACK), "other packs are unaffected");
        forget_pack_failure(KOKORO_PACK.model_id);
        assert!(!pack_failed(&KOKORO_PACK));
    }

    /// The whole point of Automatic: the graphics card is used whenever the
    /// WebView can use it, the processor only when it cannot, and a missing
    /// processor pack never leaves the voice silent.
    #[test]
    fn automatic_prefers_the_gpu_and_falls_back_to_the_processor() {
        use KokoroDevice::*;
        use WebGpuState::*;
        assert_eq!(
            resolve_kokoro_route(Auto, Usable, true),
            VoiceRoute::Webview
        );
        assert_eq!(
            resolve_kokoro_route(Auto, Unknown, true),
            VoiceRoute::Webview
        );
        assert_eq!(
            resolve_kokoro_route(Auto, Unusable, true),
            VoiceRoute::Native
        );
        assert_eq!(
            resolve_kokoro_route(Auto, Unusable, false),
            VoiceRoute::Webview,
            "without the processor pack the WebView still speaks"
        );
    }

    #[test]
    fn an_explicit_choice_wins_over_detection() {
        use KokoroDevice::*;
        use WebGpuState::*;
        assert_eq!(
            resolve_kokoro_route(Gpu, Unusable, true),
            VoiceRoute::Webview
        );
        assert_eq!(resolve_kokoro_route(Cpu, Usable, true), VoiceRoute::Native);
        assert_eq!(
            resolve_kokoro_route(Cpu, Usable, false),
            VoiceRoute::Webview
        );
    }

    #[test]
    fn device_setting_parses_leniently() {
        assert_eq!(KokoroDevice::parse("gpu"), KokoroDevice::Gpu);
        assert_eq!(KokoroDevice::parse(" cpu "), KokoroDevice::Cpu);
        assert_eq!(KokoroDevice::parse("auto"), KokoroDevice::Auto);
        assert_eq!(KokoroDevice::parse("something-else"), KokoroDevice::Auto);
    }

    #[test]
    fn engine_ids_route_to_the_right_place() {
        let mut settings = crate::settings::get_default_settings();
        settings.assistant_tts_engine = "kitten".into();
        assert_eq!(route(&settings), VoiceRoute::Native);
        settings.assistant_tts_engine = "elevenlabs".into();
        assert_eq!(route(&settings), VoiceRoute::Remote);
        settings.assistant_tts_engine = "kokoro".into();
        settings.assistant_tts_kokoro_device = "gpu".into();
        assert_eq!(route(&settings), VoiceRoute::Webview);
    }

    /// Speaker ids are the model's own order; getting one wrong silently speaks
    /// with someone else's voice, so the voices the app offers are pinned.
    #[test]
    fn kokoro_voices_map_to_the_models_speaker_ids() {
        assert_eq!(kokoro_sid("af_heart"), 3);
        assert_eq!(kokoro_sid("af_bella"), 2);
        assert_eq!(kokoro_sid("af_nicole"), 6);
        assert_eq!(kokoro_sid("af_sky"), 10);
        assert_eq!(kokoro_sid("am_adam"), 11);
        assert_eq!(kokoro_sid("am_michael"), 16);
        assert_eq!(kokoro_sid("bf_emma"), 21);
        assert_eq!(kokoro_sid("bm_george"), 26);
        assert_eq!(kokoro_sid("not-a-voice"), 3, "unknown voices get Heart");
    }

    #[test]
    fn british_voices_use_british_phonemes() {
        assert!(kokoro_is_british("bf_emma"));
        assert!(kokoro_is_british("bm_george"));
        assert!(!kokoro_is_british("af_heart"));
        assert!(!kokoro_is_british("am_adam"));
    }

    #[test]
    fn kitten_voices_accept_names_and_ids() {
        assert_eq!(kitten_sid("Jasper"), 0);
        assert_eq!(kitten_sid("Bella"), 1);
        assert_eq!(kitten_sid("bella"), 1);
        assert_eq!(kitten_sid("expr-voice-5-f"), 7);
        assert_eq!(kitten_sid("Kiki"), 7);
        assert_eq!(kitten_sid(""), 1, "empty means the default, Bella");
        assert_eq!(
            kitten_sid("af_heart"),
            1,
            "a Kokoro voice is not a Kitten voice"
        );
    }

    #[test]
    fn speed_is_clamped_and_kitten_is_paced() {
        assert_eq!(model_speed(&KOKORO_PACK, 1.0), 1.0);
        assert_eq!(model_speed(&KOKORO_PACK, 10.0), 4.0);
        assert_eq!(model_speed(&KOKORO_PACK, f64::NAN), 1.0);
        for pack in KITTEN_PACKS {
            assert!((model_speed(pack, 1.0) - pack.pace as f32).abs() < 1e-6);
            assert_eq!(model_speed(pack, 4.0), 4.0);
        }
        assert!((model_speed(&KITTEN_PACK, 1.0) - 1.6).abs() < 1e-6);
    }

    #[test]
    fn threads_follow_physical_cores_within_a_cap() {
        assert_eq!(threads_for(&KOKORO_PACK, 16), 8);
        assert_eq!(threads_for(&KOKORO_PACK, 32), 8);
        assert_eq!(threads_for(&KOKORO_PACK, 8), 4);
        assert_eq!(threads_for(&KOKORO_PACK, 2), 1);
        assert_eq!(threads_for(&KOKORO_PACK, 1), 1);
        assert_eq!(threads_for(&KITTEN_PACK, 16), 4);
    }

    #[test]
    fn kitten_sizes_resolve_from_the_model_setting() {
        assert_eq!(kitten_pack("").model_id, "kitten-nano-0.8");
        assert_eq!(kitten_pack("kitten-nano-0.8").model_id, "kitten-nano-0.8");
        assert_eq!(kitten_pack("kitten-micro-0.8").model_id, "kitten-micro-0.8");
        assert_eq!(kitten_pack("mini").model_id, "kitten-mini-0.8");
        assert_eq!(kitten_pack(" Micro ").model_id, "kitten-micro-0.8");
        assert_eq!(
            kitten_pack("gpt-4o-mini-tts").model_id,
            "kitten-nano-0.8",
            "another engine's model is not a Kitten size"
        );
    }

    #[test]
    fn every_native_engine_routes_natively_and_names_its_pack() {
        let mut settings = crate::settings::get_default_settings();
        for engine in NATIVE_ENGINES {
            settings.assistant_tts_engine = engine.into();
            assert_eq!(route(&settings), VoiceRoute::Native, "{engine}");
            let pack = active_pack(&settings).expect("a native engine has a pack");
            assert_eq!(
                Some(pack.model_id),
                pack_for_engine(engine, "").map(|p| p.model_id)
            );
            assert!(!voice_names(engine).is_empty(), "{engine} offers voices");
        }
        settings.assistant_tts_engine = "kitten".into();
        settings.assistant_tts_model = "kitten-mini-0.8".into();
        assert_eq!(
            active_pack(&settings).map(|p| p.model_id),
            Some("kitten-mini-0.8")
        );
    }

    /// A native-only engine that is not downloaded blocks a call, and says
    /// which engine it is.
    #[test]
    fn an_undownloaded_native_engine_names_itself_in_its_blocker() {
        let mut settings = crate::settings::get_default_settings();
        for (engine, name) in [
            ("kitten", "Kitten"),
            ("pocket", "Pocket TTS"),
            ("supertonic", "Supertonic"),
        ] {
            settings.assistant_tts_engine = engine.into();
            let message = blocker(&settings).expect("nothing is downloaded in a test");
            assert!(message.contains(name), "{engine}: {message}");
        }
        settings.assistant_tts_engine = "kokoro".into();
        assert!(blocker(&settings).is_none());
    }

    #[test]
    fn supertonic_and_pocket_voices_map_to_the_right_speaker() {
        assert_eq!(supertonic_sid("F1"), 0);
        assert_eq!(supertonic_sid("f5"), 4);
        assert_eq!(supertonic_sid("M1"), 5);
        assert_eq!(supertonic_sid("M5"), 9);
        assert_eq!(supertonic_sid("Bella"), 0, "unknown voices get F1");
        assert_eq!(pocket_clip("mary").name, "Mary");
        assert_eq!(pocket_clip("GEORGE").name, "George");
        assert_eq!(pocket_clip("").name, DEFAULT_POCKET_VOICE);
        assert_eq!(pocket_clip("af_heart").name, DEFAULT_POCKET_VOICE);
        // Voices dropped for their recordings fall back rather than failing.
        for dropped in ["Alba", "Marius", "Javert", "Charles", "Michael"] {
            assert_eq!(pocket_clip(dropped).name, DEFAULT_POCKET_VOICE, "{dropped}");
        }
    }

    #[test]
    fn pocket_speaks_whole_sentences_with_short_ones_joined() {
        assert_eq!(pocket_pieces("Sure."), ["Sure."]);
        assert_eq!(pocket_pieces("  got it  "), ["Got it."]);
        // Short openers and a short last sentence join their neighbours, as
        // sherpa-onnx itself would, so each call is one of its chunks.
        assert_eq!(
            pocket_pieces("Sure! I've added that to your list. Anything else?"),
            ["Sure! I've added that to your list. Anything else?"]
        );
        // A decimal is not a sentence end, and "Dr." rejoins its sentence.
        assert_eq!(
            pocket_pieces("Handy 2.5 is out. Dr. Patel will see you now, in room twelve."),
            ["Handy 2.5 is out. Dr. Patel will see you now, in room twelve."]
        );
        let long = "The capital of France is Paris, as it has been for centuries. \
                    It is the country's political and cultural center today. \
                    and its museums draw millions of visitors every year";
        assert_eq!(
            pocket_pieces(long),
            [
                "The capital of France is Paris, as it has been for centuries.",
                "It is the country's political and cultural center today.",
                "And its museums draw millions of visitors every year.",
            ]
        );
        assert_eq!(tidy_piece("\"quoted.\""), "\"Quoted.\"");
        assert!(pocket_pieces("   ").is_empty());
    }

    /// The ceiling has to stop a runaway without ever cutting real speech: the
    /// slowest clean voice needs ~0.86 frames a character.
    #[test]
    fn pocket_frame_ceiling_fits_the_text() {
        assert_eq!(pocket_max_frames("Sure."), 19);
        let sentence = "Your meeting with Doctor Patel starts at three thirty, in room twelve.";
        let needed = (sentence.chars().count() as f32 * 0.86).ceil() as i32 + 3;
        assert!(pocket_max_frames(sentence) > needed);
        assert!(pocket_max_frames(sentence) < POCKET_MAX_FRAMES);
        assert_eq!(pocket_max_frames(&"x".repeat(1000)), POCKET_MAX_FRAMES);
    }

    /// Each pack's files, and its reference clips, are what readiness checks.
    /// Clips are named by voice, so two voices can never share a file.
    #[test]
    fn pack_files_cover_every_config_path_and_clip() {
        for pack in PACKS {
            let files = pack.required_files();
            let unique: std::collections::HashSet<_> = files.iter().collect();
            assert_eq!(
                unique.len(),
                files.len(),
                "{} lists a file twice",
                pack.model_id
            );
            assert!(pack.total_download_bytes() >= pack.download_bytes);
        }
        assert_eq!(POCKET_PACK.required_files().len(), 7 + POCKET_VOICES.len());
        assert_eq!(SUPERTONIC_PACK.required_files().len(), 7);
        assert!(POCKET_PACK
            .required_files()
            .contains(&"voices/mary.wav".to_string()));
        let dirs: std::collections::HashSet<_> = PACKS.iter().map(|p| p.dir).collect();
        assert_eq!(dirs.len(), PACKS.len(), "every pack has its own folder");
    }

    #[test]
    fn clip_urls_are_pinned_to_a_commit() {
        let url = POCKET_VOICES[0].url();
        assert!(url.starts_with("https://huggingface.co/kyutai/tts-voices/resolve/"));
        assert!(url.contains(VOICES_REVISION));
        assert!(url.ends_with("vctk/p333_023_enhanced.wav"));
        for clip in &POCKET_VOICES {
            assert_eq!(clip.sha256.len(), 64, "{}", clip.name);
            assert!(clip.bytes > 0);
        }
    }

    #[test]
    fn clips_decode_to_mono_at_their_own_rate() {
        let dir = std::env::temp_dir().join(format!("native-voice-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stereo.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..100 {
            writer.write_sample(16_384_i16).unwrap();
            writer.write_sample(0_i16).unwrap();
        }
        writer.finalize().unwrap();
        let (samples, rate) = read_clip(&path).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(samples.len(), 100);
        assert!((samples[0] - 0.25).abs() < 1e-3, "{}", samples[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn engine_text_collapses_whitespace_and_controls() {
        assert_eq!(engine_text("  Hello,\n\nworld\t!  "), "Hello, world !");
        assert_eq!(engine_text("a\u{0}b"), "a b");
        assert_eq!(engine_text("   "), "");
    }

    #[test]
    fn wav_encoding_is_16_bit_mono_and_clamped() {
        let wav = samples_to_wav(&[0.0, 1.0, -1.0, 2.0], 24_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(wav.len(), 44 + 8);
        let sample = |i: usize| i16::from_le_bytes([wav[44 + i * 2], wav[45 + i * 2]]);
        assert_eq!(sample(0), 0);
        assert_eq!(sample(1), i16::MAX);
        assert_eq!(sample(2), -i16::MAX);
        assert_eq!(sample(3), i16::MAX, "out-of-range input is clamped");
    }

    #[test]
    fn packs_are_found_by_catalog_id() {
        assert_eq!(
            pack_for_model_id("kokoro-82m-native").map(|p| p.dir),
            Some("kokoro-multi-lang-v1_0")
        );
        assert_eq!(
            pack_for_model_id("kitten-nano-0.8").map(|p| p.dir),
            Some("kitten-nano-en-v0_8-fp32")
        );
        assert!(pack_for_model_id("kokoro-82m").is_none());
    }

    #[test]
    fn pack_readiness_needs_every_file() {
        let dir = std::env::temp_dir().join(format!("native-voice-pack-{}", std::process::id()));
        let pack_dir = dir.join(KITTEN_PACK.dir);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(pack_dir.join("espeak-ng-data")).unwrap();
        assert!(!pack_installed_in(&dir, &KITTEN_PACK));
        for file in ["model.fp32.onnx", "voices.bin", "tokens.txt"] {
            std::fs::write(pack_dir.join(file), b"x").unwrap();
        }
        assert!(pack_installed_in(&dir, &KITTEN_PACK));

        // Pocket is not installed until every reference clip is there too.
        let pocket_dir = dir.join(POCKET_PACK.dir);
        std::fs::create_dir_all(pocket_dir.join("voices")).unwrap();
        for file in POCKET_PACK.required_files() {
            if !file.starts_with("voices/") {
                std::fs::write(pocket_dir.join(file), b"x").unwrap();
            }
        }
        assert!(!pack_installed_in(&dir, &POCKET_PACK));
        assert_eq!(missing_clips(&dir, &POCKET_PACK).len(), POCKET_VOICES.len());
        for (_, path) in missing_clips(&dir, &POCKET_PACK) {
            std::fs::write(path, b"x").unwrap();
        }
        assert!(missing_clips(&dir, &POCKET_PACK).is_empty());
        assert!(pack_installed_in(&dir, &POCKET_PACK));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn webgpu_reports_are_tracked() {
        report_webgpu(true);
        assert_eq!(webgpu_state(), WebGpuState::Usable);
        assert!(report_webgpu(false), "a change is reported");
        assert!(
            !report_webgpu(false),
            "the same report again is not a change"
        );
        assert_eq!(webgpu_state(), WebGpuState::Unusable);
        WEBGPU.store(0, Ordering::SeqCst);
    }

    /// Rough median pitch of voiced frames, by autocorrelation. Only good for
    /// telling a low voice from a high one, which is what the live test needs:
    /// a speaker id off by one silently swaps a woman's voice for a man's.
    fn median_pitch(samples: &[f32], rate: u32) -> f32 {
        let frame = (rate as usize * 40) / 1000;
        let (min_lag, max_lag) = (rate as usize / 400, rate as usize / 70);
        let mut pitches = Vec::new();
        for chunk in samples.chunks(frame) {
            if chunk.len() < frame || chunk.len() <= max_lag {
                continue;
            }
            let energy: f32 = chunk.iter().map(|s| s * s).sum();
            if energy / (frame as f32) < 1e-3 {
                continue;
            }
            let mut best = (0.0f32, 0usize);
            for lag in min_lag..max_lag {
                let corr: f32 = chunk[..frame - lag]
                    .iter()
                    .zip(&chunk[lag..])
                    .map(|(a, b)| a * b)
                    .sum();
                if corr > best.0 {
                    best = (corr, lag);
                }
            }
            if best.1 > 0 && best.0 > 0.3 * energy {
                pitches.push(rate as f32 / best.1 as f32);
            }
        }
        pitches.sort_by(|a, b| a.total_cmp(b));
        pitches.get(pitches.len() / 2).copied().unwrap_or(0.0)
    }

    /// Pocket's int8 build, kept only so the live tests can measure it against
    /// the fp32 build the app ships.
    const POCKET_INT8_PACK: VoicePack = VoicePack {
        model_id: "pocket-tts-int8",
        dir: "sherpa-onnx-pocket-tts-int8-2026-01-26",
        layout: Layout::Pocket {
            lm_flow: "lm_flow.int8.onnx",
            lm_main: "lm_main.int8.onnx",
            encoder: "encoder.onnx",
            decoder: "decoder.int8.onnx",
            text_conditioner: "text_conditioner.onnx",
        },
        ..POCKET_PACK
    };

    /// Sentences for the quality check: a plain one, numbers and a name, and
    /// a longer run with a clause, which is where early-stopping models drop
    /// or repeat words.
    const QUALITY_TEXTS: [&str; 3] = [
        "The capital of France is Paris. It has been the country's political and cultural center for centuries.",
        "Your meeting with Doctor Patel starts at three thirty, in room twelve.",
        "If you want, I can summarize the article, draft a short reply, and remind you about it tomorrow morning.",
    ];

    /// Writes one WAV per pack, voice and sentence for an outside speech
    /// recognizer to check word for word (the Pocket settings were chosen by
    /// transcribing these with SenseVoice and scoring word error rate), and
    /// prints each one's speed. `SPEAKOFLOW_TTS_ONLY` picks packs;
    /// `SPEAKOFLOW_TTS_VOICES` (comma separated) picks voices.
    #[test]
    #[ignore]
    fn live_quality_matrix() {
        let dir = PathBuf::from(
            std::env::var_os("SPEAKOFLOW_NATIVE_TTS_DIR").expect("SPEAKOFLOW_NATIVE_TTS_DIR"),
        );
        let out = PathBuf::from(
            std::env::var_os("SPEAKOFLOW_TTS_WAV_DIR").expect("SPEAKOFLOW_TTS_WAV_DIR"),
        );
        init(dir);
        let only = std::env::var("SPEAKOFLOW_TTS_ONLY").unwrap_or_default();
        let voices: Vec<String> = std::env::var("SPEAKOFLOW_TTS_VOICES")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect();
        let packs: [&'static VoicePack; 6] = [
            &KITTEN_PACK,
            &KITTEN_MICRO_PACK,
            &KITTEN_MINI_PACK,
            &POCKET_PACK,
            &POCKET_INT8_PACK,
            &SUPERTONIC_PACK,
        ];
        for pack in packs {
            if !pack.model_id.contains(&only) || !pack_ready(pack) {
                continue;
            }
            let engine = match pack.family {
                VoiceFamily::Pocket => "pocket",
                VoiceFamily::Supertonic => "supertonic",
                _ => "kitten",
            };
            let names: Vec<String> = if voices.is_empty() {
                voice_names(engine)
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            } else {
                voices.clone()
            };
            for voice in names {
                let request = NativeRequest {
                    pack,
                    voice: voice.clone(),
                    speed: std::env::var("SPEAKOFLOW_TTS_SPEED")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1.0),
                };
                let (mut audio, mut spent, mut words) = (0.0, 0.0, 0usize);
                for (i, text) in QUALITY_TEXTS.iter().enumerate() {
                    let started = Instant::now();
                    let (samples, rate) = synthesize_pcm(&request, text).unwrap().unwrap();
                    spent += started.elapsed().as_secs_f64();
                    audio += samples.len() as f64 / rate as f64;
                    words += text.split_whitespace().count();
                    let path = out.join(format!("{}-{voice}-{i}.wav", pack.model_id));
                    std::fs::write(path, samples_to_wav(&samples, rate)).unwrap();
                }
                eprintln!(
                    "{:<18} {voice:<8} {:4.0} wpm  RTF {:.3}",
                    pack.model_id,
                    words as f64 / audio * 60.0,
                    spent / audio
                );
            }
        }
        release(None);
    }

    /// Real synthesis through the installed runtime and packs, with timings.
    /// Needs the app's models folder with the packs downloaded (or
    /// `SPEAKOFLOW_NATIVE_TTS_DIR` pointing at a folder laid out the same way);
    /// packs that aren't there are skipped. `SPEAKOFLOW_TTS_ONLY` filters by
    /// catalog id, and `SPEAKOFLOW_TTS_THREADS` / `SPEAKOFLOW_TTS_STEPS` pin
    /// those settings to measure a curve:
    /// `cargo test --lib native_tts::tests::live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_native_synthesis() {
        let dir = std::env::var_os("SPEAKOFLOW_NATIVE_TTS_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("APPDATA").map(|appdata| {
                    PathBuf::from(appdata)
                        .join("com.abhishekbarali.speakoflow")
                        .join("models")
                })
            })
            .expect("a models directory");
        init(dir.clone());
        let only = std::env::var("SPEAKOFLOW_TTS_ONLY").unwrap_or_default();
        let cases: [(&'static VoicePack, &str, u32); 12] = [
            (&KOKORO_PACK, "af_heart", 24_000),
            (&KOKORO_PACK, "bf_emma", 24_000),
            (&KITTEN_PACK, "Bella", 24_000),
            (&KITTEN_MICRO_PACK, "Bella", 24_000),
            (&KITTEN_MINI_PACK, "Bella", 24_000),
            (&KITTEN_MINI_PACK, "Jasper", 24_000),
            (&POCKET_PACK, "Mary", 24_000),
            (&POCKET_PACK, "George", 24_000),
            (&POCKET_INT8_PACK, "Mary", 24_000),
            (&SUPERTONIC_PACK, "F1", 44_100),
            (&SUPERTONIC_PACK, "M1", 44_100),
            (&SUPERTONIC_PACK, "F3", 44_100),
        ];
        let text = "The capital of France is Paris. It has been the country's political and cultural center for centuries.";
        let words = text.split_whitespace().count() as f64;
        for (pack, voice, expected_rate) in cases {
            if !only.is_empty() && !pack.model_id.contains(&only) {
                continue;
            }
            if !pack_ready(pack) {
                eprintln!(
                    "skipping {}: not installed in {}",
                    pack.model_id,
                    dir.display()
                );
                continue;
            }
            let request = NativeRequest {
                pack,
                voice: voice.to_string(),
                speed: 1.0,
            };
            let started = Instant::now();
            let (samples, rate) = synthesize_pcm(&request, text).unwrap().unwrap();
            let cold = started.elapsed().as_secs_f64();
            let seconds = samples.len() as f64 / rate as f64;
            assert_eq!(rate, expected_rate, "{} sample rate", pack.model_id);
            assert!(
                seconds > 2.0 && seconds < 15.0,
                "{} {voice}: implausible length {seconds}",
                pack.model_id
            );
            // Warm runs, best of three, which is what every reply after the
            // first pays.
            let mut warm = f64::MAX;
            for _ in 0..3 {
                let started = Instant::now();
                synthesize_pcm(&request, text).unwrap().unwrap();
                warm = warm.min(started.elapsed().as_secs_f64());
            }
            let first = Instant::now();
            synthesize_pcm(&request, "Sure, here it is.")
                .unwrap()
                .unwrap();
            let short = first.elapsed().as_secs_f64();
            if pack.family == VoiceFamily::Pocket {
                // Without a ceiling these ran to 40 s of breathing.
                for reply in ["Sure.", "Got it", "okay, done", "Wagwan brother."] {
                    let (s, r) = synthesize_pcm(&request, reply).unwrap().unwrap();
                    let length = s.len() as f64 / r as f64;
                    assert!(
                        length < 4.0,
                        "{} {voice}: {reply:?} ran to {length:.1}s",
                        pack.model_id
                    );
                }
            }
            eprintln!(
                "{:<20} {voice:<7} threads {} | {seconds:5.2}s audio, {:4.0} wpm, pitch {:3.0} Hz | cold {cold:5.2}s | warm RTF {:.3} | short reply {:.2}s",
                pack.model_id,
                engine_threads(pack),
                words / seconds * 60.0,
                median_pitch(&samples, rate),
                warm / seconds,
                short,
            );
            if let Ok(out) = std::env::var("SPEAKOFLOW_TTS_WAV_DIR") {
                let path = PathBuf::from(out).join(format!("{}-{voice}.wav", pack.model_id));
                std::fs::write(path, samples_to_wav(&samples, rate)).unwrap();
            }
        }
        release(None);
    }
}
