//! Registering models the user already has on disk.
//!
//! Two entry points, both landing in the same place:
//!
//! 1. **Pick a file.** The user chooses a `.gguf` or a Whisper `.bin` anywhere
//!    on the filesystem and it is registered as a catalog entry that points at
//!    that path. Nothing is copied and nothing is downloaded.
//! 2. **Link a folder.** The user points at one or more directories they already
//!    keep models in (an LM Studio / Ollama-adjacent tree, a fine-tuning output
//!    dir, an external drive). Each is scanned recursively and every model found
//!    is registered. Re-scanning is idempotent, so files that come and go in a
//!    linked folder appear and disappear from the catalog on their own.
//!
//! The hard part is not the filesystem walk, it's **classification**: a bare
//! `.gguf` on disk doesn't say whether it's a speech-to-text model, a chat
//! model, or a vision projector that isn't a standalone model at all. Guessing
//! from the filename fails constantly (a fine-tune named `my-whisper-v3.gguf`
//! could be either, and `mmproj` is only a convention). So we read the GGUF
//! header's `general.architecture` — the same value transcribe-cpp and
//! llama.cpp themselves dispatch on — and route from that. See
//! [`classify_model_file`].
//!
//! Nothing here mutates the filesystem. Registering a local model never copies,
//! moves, or deletes the user's file, and unregistering one only forgets the
//! path. That is a deliberate invariant: these files are not ours.

use log::{debug, warn};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::gguf_meta::{self, GgufError};
use super::model::EngineType;
use super::model_capabilities::KNOWN_ARCHES;

/// GGML file magic (`"ggml"` as a little-endian u32), the first four bytes of a
/// legacy Whisper `ggml-*.bin`. Used to reject a `.bin` that is some unrelated
/// binary before it can reach whisper.cpp and fail there instead.
const GGML_MAGIC: u32 = 0x6767_6d6c;

/// How deep a linked folder is walked. Real-world model trees are shallow but
/// not flat — the common publisher/repo/file.gguf layout is already three
/// levels, and people nest that under a category dir — so 5 covers the
/// realistic cases without turning "I linked my home directory by accident"
/// into a filesystem-wide crawl.
const MAX_SCAN_DEPTH: usize = 5;

/// Upper bound on directory entries examined per linked folder. A safety valve
/// for a mistakenly linked huge tree: scanning stops rather than blocking
/// startup. Generous enough that a real model collection is never truncated.
const MAX_SCAN_ENTRIES: usize = 20_000;

/// Only these two extensions are recognized. Everything else in a linked folder
/// is ignored silently — model directories are full of tokenizers, configs, and
/// READMEs, and warning about each would be noise.
const MODEL_EXTENSIONS: &[&str] = &["gguf", "bin"];

/// Enough of a GGUF header to reach `general.architecture`. The key/value block
/// sits at the very front of the file, but tokenizer metadata that precedes our
/// key can be large, so we grow and retry on [`GgufError::Truncated`] rather
/// than betting on one read size.
const GGUF_PROBE_INITIAL: usize = 256 * 1024;
const GGUF_PROBE_MAX: usize = 16 * 1024 * 1024;

/// The GGUF key that decides everything. transcribe-cpp and llama.cpp both
/// dispatch on this, so classifying on it means we agree with whichever engine
/// will actually load the file.
const KEY_ARCH: &str = "general.architecture";

/// Architectures belonging to a multimodal *projector* — the companion file a
/// vision model needs (`--mmproj`), which is not loadable on its own.
const PROJECTOR_ARCHES: &[&str] = &["clip", "mmproj", "llava", "qwen2vl_vision"];

/// What a file on disk turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalModelKind {
    /// A speech-to-text model. Carries the engine that should load it, which is
    /// [`EngineType::TranscribeCpp`] for GGUF and [`EngineType::Whisper`] for a
    /// legacy ggml `.bin`.
    Transcription {
        engine: EngineType,
        /// `general.architecture`, when we could read one. Displayed to the user
        /// so an unrecognized fine-tune is at least identifiable.
        architecture: Option<String>,
    },
    /// A chat / instruct model for the built-in llama.cpp sidecar.
    Llm { architecture: Option<String> },
    /// A vision projector. Registered only as a *companion* to an LLM in the
    /// same directory, never as a model in its own right.
    Projector,
}

/// Why a file could not be registered. These strings reach the user directly,
/// so each says what was wrong and what to do instead.
#[derive(Debug)]
pub enum ClassifyError {
    /// Extension is neither `.gguf` nor `.bin`.
    UnsupportedExtension(String),
    /// A `.gguf` whose magic/version didn't parse, or a `.bin` that isn't ggml.
    NotAModel(String),
    /// The file could not be read at all.
    Unreadable(String),
}

impl std::fmt::Display for ClassifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClassifyError::UnsupportedExtension(ext) => {
                if ext.is_empty() {
                    write!(
                        f,
                        "This file has no extension. Pick a .gguf model or a Whisper .bin model."
                    )
                } else {
                    write!(
                        f,
                        "'.{ext}' files aren't supported. Pick a .gguf model or a Whisper .bin model."
                    )
                }
            }
            ClassifyError::NotAModel(why) => {
                write!(f, "This doesn't look like a model file: {why}")
            }
            ClassifyError::Unreadable(why) => write!(f, "Couldn't read this file: {why}"),
        }
    }
}

impl std::error::Error for ClassifyError {}

/// A model discovered on disk, ready to be turned into a catalog entry.
#[derive(Debug, Clone)]
pub struct DiscoveredModel {
    pub path: PathBuf,
    pub kind: LocalModelKind,
    pub size_bytes: u64,
    /// Companion vision projector found next to an LLM, enabling screen vision
    /// for it. Always `None` for transcription models.
    pub mmproj_path: Option<PathBuf>,
}

impl DiscoveredModel {
    /// The engine that will load this model.
    pub fn engine_type(&self) -> EngineType {
        match &self.kind {
            LocalModelKind::Transcription { engine, .. } => engine.clone(),
            LocalModelKind::Llm { .. } => EngineType::LlamaCpp,
            // Never surfaced as a model; mapped for exhaustiveness only.
            LocalModelKind::Projector => EngineType::LlamaCpp,
        }
    }

    /// `general.architecture`, when the header carried one.
    pub fn architecture(&self) -> Option<&str> {
        match &self.kind {
            LocalModelKind::Transcription { architecture, .. }
            | LocalModelKind::Llm { architecture } => architecture.as_deref(),
            LocalModelKind::Projector => None,
        }
    }
}

/// Lowercased extension of `path`, or an empty string when there is none.
fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether `path` has an extension we know how to classify. Cheap pre-filter so
/// a directory walk doesn't open every file it passes.
pub fn has_model_extension(path: &Path) -> bool {
    MODEL_EXTENSIONS.contains(&extension_of(path).as_str())
}

/// An absolute, user-presentable form of `path`.
///
/// [`fs::canonicalize`] is the reliable way to make a path absolute and resolve
/// `..` and symlinks, but on Windows it returns an *extended-length* path
/// (`\\?\C:\models\x.gguf`). That form must not escape this function: it is
/// stored in settings, shown in the Models tab, put in error messages, and
/// handed to the llama.cpp sidecar on the command line. So the `\\?\` prefix is
/// stripped back to the path the user actually recognizes (`\\?\UNC\srv\share`
/// back to `\\srv\share`). A path that cannot be canonicalized — a drive that
/// isn't mounted right now — is returned unchanged rather than lost.
///
/// Because the same function normalizes both what we store and what we compare,
/// identity checks stay consistent.
pub fn absolute_path(path: &Path) -> PathBuf {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    strip_extended_length_prefix(&canonical)
}

/// Strip Windows' `\\?\` extended-length prefix. A no-op everywhere else, and a
/// no-op on Windows for a path that doesn't carry it.
fn strip_extended_length_prefix(path: &Path) -> PathBuf {
    if !cfg!(windows) {
        return path.to_path_buf();
    }
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        // \\?\UNC\server\share -> \\server\share
        return PathBuf::from(format!(r"\\{}", rest));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        // Only a plain drive path is safe to shorten; anything else (a volume
        // GUID path, say) is left alone because the prefix is load-bearing there.
        let is_drive_path = {
            let mut chars = rest.chars();
            matches!(
                (chars.next(), chars.next(), chars.next()),
                (Some(letter), Some(':'), Some('\\' | '/')) if letter.is_ascii_alphabetic()
            )
        };
        if is_drive_path {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

/// Read the leading `len` bytes of a file, returning fewer if it is shorter.
/// Used for both header probes so a truncated or tiny file surfaces as a parse
/// failure rather than an I/O error.
fn read_prefix(path: &Path, len: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;

    let file = fs::File::open(path)?;
    let mut buf = Vec::new();
    file.take(len as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Read `general.architecture` from a GGUF file's header.
///
/// Returns `Ok(None)` for a well-formed GGUF that simply omits the key — that
/// is a real thing in hand-converted files, and it means "unknown", not
/// "invalid". Grows the read window on truncation because the key can sit
/// behind a large tokenizer array.
fn read_gguf_architecture(path: &Path) -> Result<Option<String>, ClassifyError> {
    let file_len = fs::metadata(path)
        .map_err(|e| ClassifyError::Unreadable(e.to_string()))?
        .len() as usize;

    let mut probe = GGUF_PROBE_INITIAL.min(file_len.max(1));
    loop {
        let bytes =
            read_prefix(path, probe).map_err(|e| ClassifyError::Unreadable(e.to_string()))?;

        match gguf_meta::parse_header(&bytes, &[KEY_ARCH]) {
            Ok(meta) => return Ok(meta.get_str(KEY_ARCH).map(|s| s.to_ascii_lowercase())),
            Err(GgufError::Truncated { needed }) => {
                // Already read the whole file, or hit the cap: stop asking.
                if probe >= file_len || probe >= GGUF_PROBE_MAX {
                    debug!(
                        "GGUF header for {} still truncated at {} bytes; treating architecture as unknown",
                        path.display(),
                        probe
                    );
                    return Ok(None);
                }
                // Grow geometrically, but respect the parser's lower bound.
                probe = (probe * 4).max(needed).min(GGUF_PROBE_MAX).min(file_len);
            }
            Err(e @ (GgufError::NotGguf | GgufError::UnsupportedVersion(_))) => {
                return Err(ClassifyError::NotAModel(e.to_string()));
            }
            Err(GgufError::Malformed(why)) => {
                return Err(ClassifyError::NotAModel(format!("malformed GGUF ({why})")));
            }
        }
    }
}

/// Whether a filename looks like a vision projector. Only a fallback for a GGUF
/// whose header omits `general.architecture`; the header is always preferred.
fn filename_suggests_projector(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_ascii_lowercase().contains("mmproj"))
        .unwrap_or(false)
}

/// Decide what a file on disk is, from its contents rather than its name.
///
/// For `.gguf` this reads `general.architecture` and routes on it:
/// a projector arch (or an `mmproj` filename when the header is silent) is a
/// [`LocalModelKind::Projector`]; an architecture transcribe-cpp ships (see
/// [`KNOWN_ARCHES`]) is speech-to-text; anything else is treated as an LLM.
/// That last branch is the right default rather than a rejection: the set of
/// chat architectures llama.cpp supports grows every release, so an
/// architecture we don't recognize is far more likely to be a new chat model
/// than a broken file, and llama.cpp gives a clear error if it truly can't load
/// it.
///
/// For `.bin` the ggml magic is checked and the file is treated as Whisper —
/// the only `.bin` format the app has ever loaded.
pub fn classify_model_file(path: &Path) -> Result<LocalModelKind, ClassifyError> {
    let ext = extension_of(path);
    match ext.as_str() {
        "gguf" => {
            let arch = read_gguf_architecture(path)?;

            match arch.as_deref() {
                Some(arch) if PROJECTOR_ARCHES.contains(&arch) => Ok(LocalModelKind::Projector),
                Some(arch) if KNOWN_ARCHES.contains(&arch) => Ok(LocalModelKind::Transcription {
                    engine: EngineType::TranscribeCpp,
                    architecture: Some(arch.to_string()),
                }),
                Some(arch) => Ok(LocalModelKind::Llm {
                    architecture: Some(arch.to_string()),
                }),
                // No architecture key: fall back to the filename convention for
                // projectors, otherwise assume a chat model.
                None if filename_suggests_projector(path) => Ok(LocalModelKind::Projector),
                None => Ok(LocalModelKind::Llm { architecture: None }),
            }
        }
        "bin" => {
            let bytes =
                read_prefix(path, 4).map_err(|e| ClassifyError::Unreadable(e.to_string()))?;
            if bytes.len() < 4 {
                return Err(ClassifyError::NotAModel("file is too small".to_string()));
            }
            let magic = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            if magic != GGML_MAGIC {
                return Err(ClassifyError::NotAModel(
                    "not a Whisper ggml .bin file".to_string(),
                ));
            }
            Ok(LocalModelKind::Transcription {
                engine: EngineType::Whisper,
                architecture: Some("whisper".to_string()),
            })
        }
        other => Err(ClassifyError::UnsupportedExtension(other.to_string())),
    }
}

/// Classify a single user-picked file and describe it as a [`DiscoveredModel`].
///
/// A projector is rejected here on purpose: on its own it is not a model, and
/// silently accepting one would put a catalog entry in front of the user that
/// can never load. Projectors are only ever picked up as a companion during a
/// folder scan.
pub fn describe_model_file(path: &Path) -> Result<DiscoveredModel, ClassifyError> {
    if !path.is_file() {
        return Err(ClassifyError::Unreadable("path is not a file".to_string()));
    }

    let kind = classify_model_file(path)?;
    if kind == LocalModelKind::Projector {
        return Err(ClassifyError::NotAModel(
            "this is a vision projector (mmproj), which supports another model rather than \
             running on its own. Add the model file instead — if the projector sits next to it, \
             it's picked up automatically."
                .to_string(),
        ));
    }

    let size_bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    // A projector sitting beside a single picked LLM should still enable vision,
    // matching how the same pair is handled during a folder scan.
    let mmproj_path = if matches!(kind, LocalModelKind::Llm { .. }) {
        path.parent().and_then(find_projector_in_dir)
    } else {
        None
    };

    Ok(DiscoveredModel {
        path: path.to_path_buf(),
        kind,
        size_bytes,
        mmproj_path,
    })
}

/// Find a vision projector directly inside `dir`, if there is one.
///
/// Filename-only, and deliberately so: this runs for every directory in a scan,
/// and opening every `.gguf` twice to confirm by header would double the I/O of
/// a scan for a file whose naming convention is universal in practice. A
/// mis-detected projector costs a failed engine start with a clear llama.cpp
/// message, not a crash.
fn find_projector_in_dir(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && extension_of(p) == "gguf" && filename_suggests_projector(p))
        .collect();
    // Deterministic pick when a repo ships several projector precisions.
    candidates.sort();
    candidates.into_iter().next()
}

/// Recursively scan `root` for model files.
///
/// `skip_dirs` names directories to leave alone — used to keep the app's own
/// models directory out of a scan, since everything in there is already in the
/// catalog and would otherwise show up twice.
///
/// Errors on individual entries are logged and skipped rather than aborting: one
/// unreadable file or a permission-denied subdirectory on an external drive
/// should not lose the user the rest of their collection. A file whose header
/// won't classify is skipped the same way, because a linked folder legitimately
/// contains non-model `.bin` files.
pub fn scan_folder(root: &Path, skip_dirs: &HashSet<PathBuf>) -> Vec<DiscoveredModel> {
    let mut found = Vec::new();
    let mut examined = 0usize;

    if !root.is_dir() {
        warn!(
            "Linked model folder is missing or not a directory: {}",
            root.display()
        );
        return found;
    }

    // Explicit stack instead of recursion: depth is bounded either way, but this
    // keeps the entry cap and the skip set in one place.
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    // Guards against a symlink loop turning the walk into an infinite one.
    let mut visited: HashSet<PathBuf> = HashSet::new();

    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_SCAN_DEPTH {
            continue;
        }
        // Canonicalize only for loop detection and the skip set; the original
        // path is what we record, so the user sees the path they linked.
        let canonical = absolute_path(&dir);
        if skip_dirs.contains(&canonical) || !visited.insert(canonical) {
            continue;
        }

        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                debug!("Skipping unreadable directory {}: {}", dir.display(), e);
                continue;
            }
        };

        for entry in entries {
            if examined >= MAX_SCAN_ENTRIES {
                warn!(
                    "Stopped scanning {} after {} entries; link a more specific folder to see the rest",
                    root.display(),
                    MAX_SCAN_ENTRIES
                );
                return found;
            }
            examined += 1;

            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    debug!("Skipping unreadable entry in {}: {}", dir.display(), e);
                    continue;
                }
            };
            let path = entry.path();

            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Hidden entries, and caches that are never a user's model library.
            if name.starts_with('.') {
                continue;
            }

            if path.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }

            if !path.is_file() || !has_model_extension(&path) {
                continue;
            }

            match classify_model_file(&path) {
                // Projectors are attached to their model below, not listed.
                Ok(LocalModelKind::Projector) => {}
                Ok(kind) => {
                    let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let mmproj_path = if matches!(kind, LocalModelKind::Llm { .. }) {
                        path.parent().and_then(find_projector_in_dir)
                    } else {
                        None
                    };
                    found.push(DiscoveredModel {
                        path,
                        kind,
                        size_bytes,
                        mmproj_path,
                    });
                }
                Err(e) => {
                    debug!("Skipping {} during scan: {}", path.display(), e);
                }
            }
        }
    }

    // Stable order so ids and list positions don't shuffle between scans.
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    /// Build a minimal but valid GGUF v3 header carrying a single
    /// `general.architecture` string, so classification can be tested without
    /// shipping model fixtures.
    fn gguf_with_arch(arch: Option<&str>) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&GGUF_MAGIC_BYTES);
        out.extend_from_slice(&3u32.to_le_bytes()); // version
        out.extend_from_slice(&0u64.to_le_bytes()); // tensor count
        match arch {
            Some(arch) => {
                out.extend_from_slice(&1u64.to_le_bytes()); // kv count
                push_str(&mut out, KEY_ARCH);
                out.extend_from_slice(&8u32.to_le_bytes()); // T_STRING
                push_str(&mut out, arch);
            }
            None => {
                // A well-formed header with one unrelated key.
                out.extend_from_slice(&1u64.to_le_bytes());
                push_str(&mut out, "general.name");
                out.extend_from_slice(&8u32.to_le_bytes());
                push_str(&mut out, "something");
            }
        }
        out
    }

    const GGUF_MAGIC_BYTES: [u8; 4] = *b"GGUF";

    fn push_str(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u64).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }

    fn write(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(bytes).unwrap();
        path
    }

    fn whisper_bin() -> Vec<u8> {
        let mut out = GGML_MAGIC.to_le_bytes().to_vec();
        out.extend_from_slice(&[0u8; 64]);
        out
    }

    #[test]
    fn asr_architecture_classifies_as_transcription() {
        let dir = TempDir::new().unwrap();
        // Every arch transcribe-cpp ships must route to the STT engine, so a
        // future addition to KNOWN_ARCHES can't silently become an "LLM".
        for arch in KNOWN_ARCHES {
            let path = write(
                &dir,
                &format!("{arch}-model.gguf"),
                &gguf_with_arch(Some(arch)),
            );
            assert_eq!(
                classify_model_file(&path).unwrap(),
                LocalModelKind::Transcription {
                    engine: EngineType::TranscribeCpp,
                    architecture: Some((*arch).to_string()),
                },
                "arch {arch} should be transcription"
            );
        }
    }

    #[test]
    fn chat_architecture_classifies_as_llm() {
        let dir = TempDir::new().unwrap();
        for arch in ["llama", "qwen3", "gemma3", "phi3", "some-brand-new-arch"] {
            let path = write(&dir, &format!("{arch}.gguf"), &gguf_with_arch(Some(arch)));
            assert_eq!(
                classify_model_file(&path).unwrap(),
                LocalModelKind::Llm {
                    architecture: Some(arch.to_string())
                },
                "arch {arch} should be an LLM"
            );
        }
    }

    #[test]
    fn projector_detected_by_arch_and_by_filename() {
        let dir = TempDir::new().unwrap();
        let by_arch = write(&dir, "vision.gguf", &gguf_with_arch(Some("clip")));
        assert_eq!(
            classify_model_file(&by_arch).unwrap(),
            LocalModelKind::Projector
        );

        // Header omits the arch key, so the filename convention decides.
        let by_name = write(&dir, "mmproj-model-f16.gguf", &gguf_with_arch(None));
        assert_eq!(
            classify_model_file(&by_name).unwrap(),
            LocalModelKind::Projector
        );
    }

    #[test]
    fn gguf_without_architecture_is_treated_as_llm() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "mystery.gguf", &gguf_with_arch(None));
        assert_eq!(
            classify_model_file(&path).unwrap(),
            LocalModelKind::Llm { architecture: None }
        );
    }

    #[test]
    fn whisper_bin_is_recognized_and_junk_bin_is_rejected() {
        let dir = TempDir::new().unwrap();
        let good = write(&dir, "ggml-tiny.bin", &whisper_bin());
        assert_eq!(
            classify_model_file(&good).unwrap(),
            LocalModelKind::Transcription {
                engine: EngineType::Whisper,
                architecture: Some("whisper".to_string()),
            }
        );

        let junk = write(&dir, "notes.bin", b"this is not a model at all");
        assert!(matches!(
            classify_model_file(&junk),
            Err(ClassifyError::NotAModel(_))
        ));
    }

    #[test]
    fn non_gguf_and_wrong_extension_are_rejected() {
        let dir = TempDir::new().unwrap();
        let fake = write(&dir, "fake.gguf", b"NOT A GGUF FILE AT ALL, REALLY");
        assert!(matches!(
            classify_model_file(&fake),
            Err(ClassifyError::NotAModel(_))
        ));

        let wrong = write(&dir, "weights.safetensors", b"\x00\x01\x02\x03");
        assert!(matches!(
            classify_model_file(&wrong),
            Err(ClassifyError::UnsupportedExtension(ext)) if ext == "safetensors"
        ));
    }

    #[test]
    fn describe_rejects_a_bare_projector() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "mmproj-f16.gguf", &gguf_with_arch(Some("clip")));
        let err = describe_model_file(&path).unwrap_err();
        assert!(
            err.to_string().contains("vision projector"),
            "message should explain what a projector is, got: {err}"
        );
    }

    #[test]
    fn describe_pairs_a_projector_next_to_a_picked_llm() {
        let dir = TempDir::new().unwrap();
        write(&dir, "mmproj-model-f16.gguf", &gguf_with_arch(Some("clip")));
        let model = write(&dir, "chat-Q4_K_M.gguf", &gguf_with_arch(Some("gemma3")));

        let described = describe_model_file(&model).unwrap();
        assert_eq!(described.engine_type(), EngineType::LlamaCpp);
        assert!(
            described.mmproj_path.is_some(),
            "a projector beside the model should enable vision"
        );
    }

    #[test]
    fn scan_finds_models_recursively_and_ignores_the_rest() {
        let dir = TempDir::new().unwrap();
        write(&dir, "top.gguf", &gguf_with_arch(Some("llama")));
        write(
            &dir,
            "publisher/repo/deep-Q4_K_M.gguf",
            &gguf_with_arch(Some("qwen3")),
        );
        write(
            &dir,
            "asr/my-finetune.gguf",
            &gguf_with_arch(Some("whisper")),
        );
        write(&dir, "asr/ggml-custom.bin", &whisper_bin());
        // Noise that must not become catalog entries.
        write(&dir, "README.md", b"# models");
        write(&dir, "tokenizer.json", b"{}");
        write(
            &dir,
            "publisher/repo/mmproj-f16.gguf",
            &gguf_with_arch(Some("clip")),
        );
        write(&dir, "corrupt.gguf", b"garbage");
        write(&dir, ".hidden/secret.gguf", &gguf_with_arch(Some("llama")));

        let found = scan_folder(dir.path(), &HashSet::new());
        let names: HashSet<String> = found
            .iter()
            .map(|m| m.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        let expected: HashSet<String> = [
            "top.gguf",
            "deep-Q4_K_M.gguf",
            "my-finetune.gguf",
            "ggml-custom.bin",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        assert_eq!(names, expected, "unexpected scan result");

        // The whisper-arch GGUF must land on the STT engine, and the .bin on the
        // legacy Whisper engine — a folder can legitimately hold both.
        let finetune = found
            .iter()
            .find(|m| m.path.ends_with("my-finetune.gguf"))
            .unwrap();
        assert_eq!(finetune.engine_type(), EngineType::TranscribeCpp);
        let legacy = found
            .iter()
            .find(|m| m.path.ends_with("ggml-custom.bin"))
            .unwrap();
        assert_eq!(legacy.engine_type(), EngineType::Whisper);
    }

    #[test]
    fn scan_result_is_sorted_and_skips_requested_directories() {
        let dir = TempDir::new().unwrap();
        write(&dir, "b.gguf", &gguf_with_arch(Some("llama")));
        write(&dir, "a.gguf", &gguf_with_arch(Some("llama")));
        let skipped_dir = dir.path().join("appdata");
        fs::create_dir_all(&skipped_dir).unwrap();
        write(&dir, "appdata/managed.gguf", &gguf_with_arch(Some("llama")));

        let mut skip = HashSet::new();
        // Must be normalized the same way `scan_folder` normalizes what it
        // visits, otherwise the skip never matches on Windows.
        skip.insert(absolute_path(&skipped_dir));

        let found = scan_folder(dir.path(), &skip);
        let names: Vec<String> = found
            .iter()
            .map(|m| m.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert_eq!(names, vec!["a.gguf", "b.gguf"]);
    }

    #[test]
    fn scan_attaches_a_projector_to_a_model_in_the_same_directory() {
        let dir = TempDir::new().unwrap();
        write(&dir, "vlm/mmproj-f16.gguf", &gguf_with_arch(Some("clip")));
        write(&dir, "vlm/vlm-Q4_K_M.gguf", &gguf_with_arch(Some("gemma3")));
        write(
            &dir,
            "text/text-Q4_K_M.gguf",
            &gguf_with_arch(Some("llama")),
        );

        let found = scan_folder(dir.path(), &HashSet::new());
        let vlm = found
            .iter()
            .find(|m| m.path.ends_with("vlm-Q4_K_M.gguf"))
            .expect("vision model should be found");
        assert!(vlm.mmproj_path.is_some());

        let text = found
            .iter()
            .find(|m| m.path.ends_with("text-Q4_K_M.gguf"))
            .expect("text model should be found");
        assert!(
            text.mmproj_path.is_none(),
            "a model with no projector beside it must not claim vision"
        );
    }

    #[test]
    fn missing_folder_scans_to_nothing_instead_of_failing() {
        let dir = TempDir::new().unwrap();
        let gone = dir.path().join("not-here");
        assert!(scan_folder(&gone, &HashSet::new()).is_empty());
    }

    /// `\\?\` must never reach the user or the llama.cpp command line.
    #[cfg(windows)]
    #[test]
    fn windows_extended_length_prefix_is_stripped() {
        assert_eq!(
            strip_extended_length_prefix(Path::new(r"\\?\C:\models\a.gguf")),
            PathBuf::from(r"C:\models\a.gguf")
        );
        assert_eq!(
            strip_extended_length_prefix(Path::new(r"\\?\UNC\server\share\a.gguf")),
            PathBuf::from(r"\\server\share\a.gguf")
        );
        // Already plain: unchanged.
        assert_eq!(
            strip_extended_length_prefix(Path::new(r"D:\models\a.gguf")),
            PathBuf::from(r"D:\models\a.gguf")
        );
        // A volume GUID path needs its prefix, so it is left alone.
        let guid = r"\\?\Volume{11111111-2222-3333-4444-555555555555}\a.gguf";
        assert_eq!(
            strip_extended_length_prefix(Path::new(guid)),
            PathBuf::from(guid)
        );
    }

    /// A real canonicalization must come back in plain form, which is the
    /// property the rest of the app depends on.
    #[test]
    fn absolute_path_returns_a_plain_usable_path() {
        let dir = TempDir::new().unwrap();
        let file = write(&dir, "model.gguf", &gguf_with_arch(Some("llama")));

        let resolved = absolute_path(&file);
        assert!(
            !resolved.to_string_lossy().starts_with(r"\\?\"),
            "extended-length prefix leaked: {}",
            resolved.display()
        );
        assert!(
            resolved.is_file(),
            "the normalized path must still open: {}",
            resolved.display()
        );
    }
}
