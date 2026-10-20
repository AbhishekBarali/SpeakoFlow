//! Running other programs from inside an AppImage.

use std::path::Path;
use std::process::Command;

/// `ld_library_path` without the entries under the AppImage mount `appdir`.
/// `None` when nothing is left, so the caller can drop the variable.
fn without_appdir_entries(ld_library_path: &str, appdir: &Path) -> Option<String> {
    // `Path::starts_with` compares whole components, so `/tmp/.mount_a2` is not
    // mistaken for a child of `/tmp/.mount_a`. Empty entries mean "the current
    // directory" to the loader and are never intentional here.
    let kept: Vec<&str> = ld_library_path
        .split(':')
        .filter(|entry| !entry.is_empty() && !Path::new(entry).starts_with(appdir))
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept.join(":"))
    }
}

/// Keep the AppImage's library directories out of a child process's
/// environment.
///
/// The AppImage launcher exports `LD_LIBRARY_PATH=$APPDIR/usr/lib:...` so the
/// app finds its bundled libraries, and every child process inherits it. The
/// loader searches `LD_LIBRARY_PATH` before a binary's own `RUNPATH`, so a
/// program that is not part of the bundle then loads the AppImage's copies of
/// libraries it shares with the app, and can fail to start. The local LLM
/// engine loads the AppImage's `libggml` (the older one Whisper uses) instead
/// of the newer copy shipped beside `llama-server`, so `libllama` fails on an
/// undefined `ggml_*` symbol and the engine exits with status 127. The
/// system's own `pactl`, `wpctl` and `amixer` can likewise pick up a bundled
/// library built for a different version. Entries the user set themselves are
/// kept, for engines pointed to by `HANDY_LLAMA_SERVER`. Outside an AppImage
/// (no `APPDIR`) nothing changes.
pub fn scrub_appimage_library_path(cmd: &mut Command) {
    let Some(appdir) = std::env::var_os("APPDIR").filter(|dir| !dir.is_empty()) else {
        return;
    };
    let Ok(current) = std::env::var("LD_LIBRARY_PATH") else {
        return;
    };
    match without_appdir_entries(&current, Path::new(&appdir)) {
        Some(kept) => cmd.env("LD_LIBRARY_PATH", kept),
        None => cmd.env_remove("LD_LIBRARY_PATH"),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appdir_entries_are_dropped_and_user_entries_kept() {
        let appdir = Path::new("/tmp/.mount_Speako1");
        let path = "/tmp/.mount_Speako1/usr/lib/:/tmp/.mount_Speako1/lib64/:/opt/custom/lib";
        assert_eq!(
            without_appdir_entries(path, appdir).as_deref(),
            Some("/opt/custom/lib")
        );
    }

    #[test]
    fn appdir_prefix_match_respects_path_components() {
        let appdir = Path::new("/tmp/.mount_Speako1");
        assert_eq!(
            without_appdir_entries("/tmp/.mount_Speako12/usr/lib", appdir).as_deref(),
            Some("/tmp/.mount_Speako12/usr/lib")
        );
    }

    #[test]
    fn library_path_made_only_of_appdir_entries_is_removed() {
        let appdir = Path::new("/tmp/.mount_Speako1");
        assert_eq!(
            without_appdir_entries("/tmp/.mount_Speako1/usr/lib/:", appdir),
            None
        );
        assert_eq!(without_appdir_entries("", appdir), None);
    }
}
