//! Where agents are allowed to work, and telling the assistant about the machine.
//!
//! ## The bug this fixes
//!
//! Asked to "make a folder on the Desktop and start a coding session there", the
//! assistant asked the user for their Windows username. It was not the model's
//! fault. It had no way to know the home directory, there was no tool that could
//! create a folder, and starting a session required the folder to already exist.
//! Three gaps, all ours.
//!
//! ## The safety posture
//!
//! Voice is a lossy input. "Delete the old build folder" and "delete the build
//! folder" sound alike, transcription drops words, and there is no undo for a
//! spoken command. So this module is deliberately one-directional:
//!
//! * **Constructive only.** Create a directory, create a new file. No delete, no
//!   move, no overwrite, no rename. Those exist in the agent's own toolset, where
//!   they go through the approval policy and a human.
//! * **Allow-listed roots.** Desktop, Documents, and the workspace root. Nowhere
//!   else, so a misheard path cannot reach `C:\Windows` or `~/.ssh`.
//! * **Checked on resolved paths, twice.** Once before creating and once after,
//!   because a path is only really knowable after it exists. Comparing strings
//!   instead of resolved paths is the mistake behind CVE-2025-53109 in
//!   Anthropic's own filesystem MCP server, where a symlink escaped the
//!   allow-list.

use std::path::{Path, PathBuf};

use super::policy::within;
use super::{env, registry};

/// Folder name used for the default workspace root.
const WORKSPACE_FOLDER: &str = "SpeakoFlow Projects";

/// The user's home directory, resolved the way a new process would see it.
fn home() -> Option<PathBuf> {
    env::resolve_var("USERPROFILE")
        .or_else(|| env::resolve_var("HOME"))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// The Desktop, if there is one.
///
/// OneDrive-redirected Desktops are the common case on Windows and are not at
/// `%USERPROFILE%\Desktop`, so the redirected location is checked first —
/// otherwise "put it on my desktop" creates a folder the user cannot see.
fn desktop() -> Option<PathBuf> {
    if let Some(onedrive) =
        env::resolve_var("OneDrive").or_else(|| env::resolve_var("OneDriveConsumer"))
    {
        let candidate = PathBuf::from(onedrive).join("Desktop");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    let candidate = home()?.join("Desktop");
    candidate.is_dir().then_some(candidate)
}

fn documents() -> Option<PathBuf> {
    if let Some(onedrive) = env::resolve_var("OneDrive") {
        let candidate = PathBuf::from(onedrive).join("Documents");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    let candidate = home()?.join("Documents");
    candidate.is_dir().then_some(candidate)
}

/// Where new projects go when the user does not say.
///
/// A dedicated folder rather than the Desktop root: it keeps generated projects
/// together, it is obvious what created them, and it gives "make me a new
/// project" a legal answer that needs no follow-up question.
pub fn workspace_root() -> PathBuf {
    if let Some(configured) = env::resolve_var("SPEAKOFLOW_WORKSPACE_ROOT") {
        let path = PathBuf::from(configured.trim());
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    let base = desktop().or_else(home).unwrap_or_else(std::env::temp_dir);
    base.join(WORKSPACE_FOLDER)
}

/// Every root a voice command may create things inside.
pub fn allowed_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    roots.push(workspace_root());
    if let Some(desktop) = desktop() {
        roots.push(desktop);
    }
    if let Some(documents) = documents() {
        roots.push(documents);
    }
    // Extra roots the user has opted into, separated the way PATH is.
    if let Some(extra) = env::resolve_var("SPEAKOFLOW_EXTRA_PROJECT_ROOTS") {
        for part in extra.split(if cfg!(windows) { ';' } else { ':' }) {
            let part = part.trim();
            if !part.is_empty() {
                roots.push(PathBuf::from(part));
            }
        }
    }
    roots
}

/// Whether `path` is somewhere a voice command may create things.
fn is_allowed(path: &Path) -> bool {
    allowed_roots().iter().any(|root| {
        // The root itself counts, as does anything inside it.
        path == root || within(root, path)
    })
}

/// Turn what the user said into an absolute path.
///
/// A bare name ("my new project") lands in the workspace root. A path starting
/// with `~`, `desktop`, or `documents` is resolved against that folder. An
/// absolute path is taken as given, and then checked like everything else.
fn interpret(raw: &str) -> Result<PathBuf, String> {
    let text = raw.trim().trim_matches('"').trim();
    if text.is_empty() {
        return Err("What should the folder be called?".to_string());
    }
    // Reject the shapes that only appear when something has gone wrong.
    if text.contains('\0') {
        return Err("That folder name isn't valid.".to_string());
    }

    let normalised = text.replace('/', std::path::MAIN_SEPARATOR_STR);
    let lowered = normalised.to_lowercase();

    if let Some(rest) = normalised.strip_prefix('~') {
        let rest = rest.trim_start_matches(std::path::MAIN_SEPARATOR);
        return Ok(home()
            .ok_or_else(|| "I couldn't work out your home folder.".to_string())?
            .join(rest));
    }
    for (prefix, base) in [
        ("desktop", desktop()),
        ("documents", documents()),
        ("my documents", documents()),
    ] {
        if lowered == prefix
            || lowered.starts_with(&format!("{prefix}{}", std::path::MAIN_SEPARATOR))
        {
            let base = base.ok_or_else(|| format!("I couldn't find your {prefix} folder."))?;
            let rest = normalised[prefix.len()..].trim_start_matches(std::path::MAIN_SEPARATOR);
            return Ok(if rest.is_empty() {
                base
            } else {
                base.join(rest)
            });
        }
    }

    let candidate = PathBuf::from(&normalised);
    if candidate.is_absolute() {
        return Ok(candidate);
    }
    Ok(workspace_root().join(normalised))
}

/// Create a folder for an agent to work in, and return where it went.
///
/// Idempotent: an existing allowed folder is returned as-is, because "make a
/// folder called notes" said twice should not fail the second time.
pub fn create_folder(raw: &str) -> Result<PathBuf, String> {
    let target = interpret(raw)?;

    if target.is_file() {
        return Err(format!(
            "{} is a file, so I can't use it as a project folder.",
            target.display()
        ));
    }

    // Checked before creating, so a disallowed path never gets made at all.
    if !is_allowed(&target) {
        return Err(format!(
            "I can only create project folders under {}. {} is outside that.",
            allowed_roots()
                .iter()
                .map(|r| r.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            target.display()
        ));
    }

    if !target.is_dir() {
        std::fs::create_dir_all(&target)
            .map_err(|e| format!("Couldn't create {}: {}", target.display(), e))?;
    }

    // Checked again now that it exists, because only a real path can be
    // canonicalised — and canonicalising is what resolves a symlink that was
    // pointing somewhere else all along.
    let resolved = std::fs::canonicalize(&target)
        .map_err(|e| format!("Created {} but couldn't verify it: {}", target.display(), e))?;
    let resolved = strip_verbatim(resolved);
    if !is_allowed(&resolved) {
        // Only remove it if it is empty, so this can never delete user data.
        let _ = std::fs::remove_dir(&resolved);
        return Err(format!(
            "{} resolves to {}, which is outside the folders I'm allowed to use.",
            target.display(),
            resolved.display()
        ));
    }
    Ok(resolved)
}

/// Create a new file with the given contents.
///
/// Refuses to touch an existing file. Overwriting by voice is not a feature: the
/// agent has editing tools for that, and they go through the approval policy.
pub fn create_file(raw: &str, contents: &str) -> Result<PathBuf, String> {
    let target = interpret(raw)?;
    if target.exists() {
        return Err(format!(
            "{} already exists. I only create new files — ask the agent to edit it instead.",
            target.display()
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| "That path has no folder to put a file in.".to_string())?;
    if !is_allowed(parent) {
        return Err(format!(
            "I can only create files under {}.",
            allowed_roots()
                .iter()
                .map(|r| r.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !parent.is_dir() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Couldn't create {}: {}", parent.display(), e))?;
    }
    // Re-check the parent once it is real, then confirm the file lands inside it.
    let resolved_parent = strip_verbatim(
        std::fs::canonicalize(parent)
            .map_err(|e| format!("Couldn't verify {}: {}", parent.display(), e))?,
    );
    if !is_allowed(&resolved_parent) {
        return Err(format!(
            "{} resolves outside the folders I'm allowed to use.",
            parent.display()
        ));
    }
    let name = target
        .file_name()
        .ok_or_else(|| "That path has no file name.".to_string())?;
    let final_path = resolved_parent.join(name);

    // `create_new` is the whole guarantee: it fails rather than truncating, and
    // it does so atomically, so nothing races between the check and the write.
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&final_path)
        .map_err(|e| format!("Couldn't create {}: {}", final_path.display(), e))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| format!("Couldn't write {}: {}", final_path.display(), e))?;
    Ok(final_path)
}

/// Folder a handoff brief is written into, inside the project.
const HANDOFF_DIR: &str = ".speakoflow";

/// Write a takeover brief inside a session's own working directory.
///
/// Needed because most agents keep their protocol sessions and their terminal
/// sessions in different stores — verified for Kiro: an ACP session id never
/// appears in `kiro-cli chat --list-sessions`, so there is nothing for
/// `--resume-id` to find. The transcript genuinely cannot be handed over, so the
/// *situation* is handed over instead: what the task was, what changed, where it
/// got to. A terminal that opens knowing that is a handover; one that opens empty
/// is just a terminal.
///
/// Scoped deliberately narrower than [`create_folder`]: the only writable target
/// is `<cwd>/.speakoflow/`, checked against the canonicalised `cwd`, so a session
/// working outside the allow-listed project roots still gets a brief without this
/// becoming a general "write anywhere" path. Overwriting is allowed here — unlike
/// [`create_file`] — because the file is ours and a stale brief is worse than none.
pub fn write_handoff_brief(cwd: &str, name: &str, body: &str) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(cwd)
        .map(strip_verbatim)
        .map_err(|e| format!("Couldn't resolve {}: {}", cwd, e))?;
    if !root.is_dir() {
        return Err(format!("{} is not a folder.", root.display()));
    }
    // No path separators from the caller: the name becomes exactly one file.
    let file_name = format!(
        "{}.md",
        name.chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            })
            .collect::<String>()
    );
    let dir = root.join(HANDOFF_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Couldn't create {}: {}", dir.display(), e))?;
    let resolved_dir = std::fs::canonicalize(&dir)
        .map(strip_verbatim)
        .map_err(|e| format!("Couldn't verify {}: {}", dir.display(), e))?;
    // Checked after creation, so a symlinked `.speakoflow` cannot redirect the
    // write somewhere else entirely.
    if !within(&root, &resolved_dir) {
        return Err(format!(
            "{} resolves outside {}.",
            resolved_dir.display(),
            root.display()
        ));
    }
    let target = resolved_dir.join(file_name);
    std::fs::write(&target, body)
        .map_err(|e| format!("Couldn't write {}: {}", target.display(), e))?;
    Ok(target)
}

/// Drop Windows' `\\?\` verbatim prefix so paths read normally to a human.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => path,
    }
}

/// Existing project folders, newest first, for offering choices out loud.
fn known_projects(limit: usize) -> Vec<String> {
    let mut found: Vec<(std::time::SystemTime, String)> = Vec::new();
    for root in [workspace_root()] {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let when = metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            found.push((when, name));
        }
    }
    found.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
    found
        .into_iter()
        .take(limit)
        .map(|(_, name)| name)
        .collect()
}

/// Facts about this machine, for the assistant's system prompt.
///
/// Short on purpose. It sits in every turn's prompt, so it earns its tokens by
/// removing whole classes of question the assistant would otherwise have to ask
/// — starting with "what is your username?", which is what prompted all of this.
pub fn machine_context() -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "Operating system: {}",
        match std::env::consts::OS {
            "windows" => "Windows",
            "macos" => "macOS",
            "linux" => "Linux",
            other => other,
        }
    ));
    if let Some(home) = home() {
        lines.push(format!("Home folder: {}", home.display()));
    }
    if let Some(desktop) = desktop() {
        lines.push(format!("Desktop: {}", desktop.display()));
    }
    if let Some(documents) = documents() {
        lines.push(format!("Documents: {}", documents.display()));
    }
    lines.push(format!(
        "Default folder for new projects: {}",
        workspace_root().display()
    ));

    let agents = registry::installed();
    if agents.is_empty() {
        lines.push(
            "Coding agents installed: none found. Say so if asked to start a coding session."
                .to_string(),
        );
    } else {
        lines.push(format!(
            "Coding agents installed: {}",
            agents
                .iter()
                .map(|a| a.label())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let projects = known_projects(8);
    if !projects.is_empty() {
        lines.push(format!("Recent project folders: {}", projects.join(", ")));
    }

    format!(
        "--- THIS MACHINE ---\n{}\nYou already know these paths. Never ask the user for their \
username, home folder, or where their desktop is — build the path yourself.\n--- END THIS MACHINE ---",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Points the allow-list at a temporary folder for the duration of a test.
    ///
    /// Environment variables are process-global, so these tests would otherwise
    /// clobber each other when the harness runs them in parallel — which it does
    /// by default. The lock makes them take turns instead of being marked
    /// `#[ignore]` or requiring `--test-threads=1`, either of which would mean
    /// the safety checks stop being verified in a normal test run.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Sandbox {
        /// Held for the test's lifetime; released on drop.
        _guard: std::sync::MutexGuard<'static, ()>,
        _dir: tempfile::TempDir,
        root: PathBuf,
    }

    impl Sandbox {
        fn new() -> Self {
            // A panicking test poisons the lock. Recovering rather than
            // propagating means one real failure does not cascade into several
            // misleading ones.
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::tempdir().expect("temp dir");
            let root = std::fs::canonicalize(dir.path()).expect("canonicalize");
            let root = strip_verbatim(root);
            std::env::set_var("SPEAKOFLOW_WORKSPACE_ROOT", &root);
            std::env::remove_var("SPEAKOFLOW_EXTRA_PROJECT_ROOTS");
            Self {
                _guard: guard,
                _dir: dir,
                root,
            }
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            std::env::remove_var("SPEAKOFLOW_WORKSPACE_ROOT");
        }
    }

    #[test]
    fn a_bare_name_lands_in_the_workspace_root() {
        let sandbox = Sandbox::new();
        let made = create_folder("my new project").expect("created");
        assert_eq!(made, sandbox.root.join("my new project"));
        assert!(made.is_dir());
    }

    #[test]
    fn creating_the_same_folder_twice_succeeds() {
        let _sandbox = Sandbox::new();
        let first = create_folder("notes").expect("first");
        let second = create_folder("notes").expect("second");
        assert_eq!(first, second);
    }

    #[test]
    fn a_path_outside_the_allowed_roots_is_refused() {
        let _sandbox = Sandbox::new();
        let outside = tempfile::tempdir().expect("outside");
        let target = outside.path().join("should-not-exist");
        let error = create_folder(&target.to_string_lossy()).expect_err("must refuse");
        assert!(error.contains("outside"), "{error}");
        assert!(!target.exists(), "nothing may be created when refused");
    }

    #[test]
    fn traversal_out_of_the_workspace_is_refused() {
        let _sandbox = Sandbox::new();
        let error = create_folder("../escaped").expect_err("must refuse");
        assert!(error.contains("outside"), "{error}");
    }

    #[test]
    fn nested_folders_are_created_in_one_go() {
        let sandbox = Sandbox::new();
        let made = create_folder("client/site/src").expect("created");
        assert!(made.is_dir());
        assert!(made.starts_with(&sandbox.root));
    }

    #[test]
    fn an_empty_name_is_a_question_not_a_folder() {
        let _sandbox = Sandbox::new();
        assert!(create_folder("   ").is_err());
    }

    #[test]
    fn files_are_created_but_never_overwritten() {
        let _sandbox = Sandbox::new();
        let path = create_file("notes/hello.txt", "hello").expect("created");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "hello");

        let error = create_file("notes/hello.txt", "different").expect_err("must refuse");
        assert!(error.contains("already exists"), "{error}");
        // The original is untouched, which is the point.
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "hello");
    }

    #[test]
    fn files_outside_the_allowed_roots_are_refused() {
        let _sandbox = Sandbox::new();
        let outside = tempfile::tempdir().expect("outside");
        let target = outside.path().join("stray.txt");
        assert!(create_file(&target.to_string_lossy(), "x").is_err());
        assert!(!target.exists());
    }

    #[test]
    fn the_machine_context_answers_the_question_that_started_this() {
        let context = machine_context();
        // The failure was the assistant asking for a username. The block must
        // contain a real home path and say not to ask.
        assert!(context.contains("Home folder:") || context.contains("Desktop:"));
        assert!(context.contains("Never ask the user for their"));
        assert!(context.contains("Default folder for new projects:"));
    }
}
