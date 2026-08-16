//! Finding the agent CLI, and giving it an environment that actually works.
//!
//! Two failures made this necessary, both observed on a real machine:
//!
//! 1. The handed-off terminal reported `'claude' is not recognized` *and*
//!    `'DOSKEY' is not recognized` — the latter lives in System32, so the child
//!    had no usable `PATH` at all. Depending on whatever `PATH` the app process
//!    happens to have inherited is not something a shipped feature can do.
//! 2. Sessions failed with `Not logged in`, because the user configured their
//!    provider credentials *after* launching SpeakoFlow. On Windows, a running
//!    process never sees later `setx` changes, and closing the window only hides
//!    it to the tray — so the stale environment can outlive several "restarts".
//!
//! Both are fixed the same way: read the truth out of the registry (which is
//! where `setx` writes and where Windows builds every new process environment
//! from) and pass it to the child explicitly.

use std::path::PathBuf;

/// The platform's `PATH` entry separator.
const SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// Environment variables the agent CLI needs that a stale app process is likely
/// to be missing. Deliberately a fixed list rather than "everything": copying
/// the whole user environment into a child is a good way to smuggle in surprises.
const FORWARDED: [&str; 12] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "ANTHROPIC_FOUNDRY_RESOURCE",
    "ANTHROPIC_MODEL",
    "AWS_PROFILE",
    "AWS_REGION",
    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_VERTEX",
];

/// Read one variable as Windows would resolve it for a *new* process: the live
/// process environment first, then the user and machine registry hives.
pub fn resolve_var(name: &str) -> Option<String> {
    if let Ok(value) = std::env::var(name) {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }
    registry_var(name)
}

#[cfg(windows)]
fn registry_var(name: &str) -> Option<String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let user = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .ok()
        .and_then(|key| key.get_value::<String, _>(name).ok());
    if let Some(value) = user.filter(|v| !v.trim().is_empty()) {
        return Some(expand(&value));
    }
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment")
        .ok()
        .and_then(|key| key.get_value::<String, _>(name).ok())
        .filter(|v| !v.trim().is_empty())
        .map(|value| expand(&value))
}

#[cfg(not(windows))]
fn registry_var(_name: &str) -> Option<String> {
    None
}

/// Expand `%VAR%` references, which `REG_EXPAND_SZ` values (notably `PATH`) are
/// full of. Unknown names are left as-is rather than blanked, so a bad expansion
/// is visible instead of silently losing a directory.
#[cfg(windows)]
fn expand(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) => out.push_str(&value),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                out.push_str(after);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// A `PATH` a child process can actually rely on: the live one, plus both
/// registry hives, in that order, deduplicated.
///
/// Joined with the platform's own separator. This matters: the result is handed
/// to a child as its `PATH`, so a semicolon-joined value on Unix would give the
/// child one nonsensical directory instead of many working ones.
pub fn effective_path() -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut push_all = |value: Option<String>| {
        if let Some(value) = value {
            for part in value.split(SEPARATOR) {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let key = part.to_lowercase();
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                parts.push(part.to_string());
            }
        }
    };
    push_all(std::env::var("PATH").ok());
    push_all(registry_var("PATH"));
    // System directories, in case both of the above are somehow unusable — the
    // failure that started all this had no System32 on `PATH`.
    #[cfg(windows)]
    {
        let root = std::env::var("SYSTEMROOT").unwrap_or_else(|_| "C:\\Windows".to_string());
        push_all(Some(format!(
            "{root}\\System32;{root};{root}\\System32\\Wbem"
        )));
    }
    parts.join(&SEPARATOR.to_string())
}

/// Every directory on the effective `PATH`, as paths.
pub fn path_dirs() -> Vec<PathBuf> {
    effective_path()
        .split(SEPARATOR)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Find an executable by name, the way a shell would, but against the
/// [`effective_path`] rather than whatever the app inherited.
///
/// On Windows the bare stem is tried with each of the extensions that make a
/// file executable, because npm-installed CLIs are usually `.cmd` shims rather
/// than `.exe` binaries.
pub fn resolve_program(stem: &str) -> Option<PathBuf> {
    let stem = stem.trim();
    if stem.is_empty() {
        return None;
    }
    // An absolute path needs no searching.
    let direct = PathBuf::from(stem);
    if direct.is_absolute() {
        return direct.is_file().then_some(direct);
    }

    let mut names: Vec<String> = Vec::new();
    #[cfg(windows)]
    {
        // `.exe` first: when both exist the native binary is the better target,
        // and a `.cmd` shim needs a shell to run.
        for extension in ["exe", "cmd", "bat", "com", ""] {
            names.push(if extension.is_empty() {
                stem.to_string()
            } else {
                format!("{stem}.{extension}")
            });
        }
    }
    #[cfg(not(windows))]
    {
        names.push(stem.to_string());
    }

    let mut roots = well_known_bin_dirs();
    roots.extend(path_dirs());
    for root in roots {
        for name in &names {
            let candidate = root.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Directories that hold user-installed CLIs, checked before `PATH` so a broken
/// `PATH` cannot break the feature.
fn well_known_bin_dirs() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = resolve_var("USERPROFILE").or_else(|| resolve_var("HOME")) {
        let home = PathBuf::from(home);
        roots.push(home.join(".local").join("bin"));
        roots.push(home.join(".claude").join("local"));
        roots.push(home.join(".kiro").join("bin"));
        roots.push(home.join("AppData").join("Roaming").join("npm"));
        roots.push(home.join(".npm-global").join("bin"));
        roots.push(home.join(".bun").join("bin"));
        roots.push(home.join(".cargo").join("bin"));
    }
    if let Some(local) = resolve_var("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        roots.push(local.join("Programs").join("claude"));
        roots.push(local.join("Programs").join("kiro").join("bin"));
        roots.push(local.join("Microsoft").join("WindowsApps"));
    }
    roots.push(PathBuf::from("/usr/local/bin"));
    roots.push(PathBuf::from("/opt/homebrew/bin"));
    roots
}

/// Provider credentials and settings to hand the child, taken from the registry
/// when the live process is missing them.
pub fn forwarded_vars() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for name in FORWARDED {
        // Only fill gaps: an explicitly set live value always wins, so a user
        // launching from a terminal can still override.
        if std::env::var(name).map(|v| !v.is_empty()).unwrap_or(false) {
            continue;
        }
        if let Some(value) = registry_var(name) {
            out.push((name.to_string(), value));
        }
    }
    out
}

/// Absolute path to the Claude Code CLI.
///
/// Resolved rather than trusted to `PATH`, because a GUI app's inherited
/// environment is not dependable, and because the handed-off terminal needs an
/// absolute command anyway.
pub fn resolve_claude() -> Result<PathBuf, String> {
    // An explicit override wins, for unusual installs.
    if let Some(explicit) = resolve_var("SPEAKOFLOW_CLAUDE_PATH") {
        let path = PathBuf::from(explicit.trim());
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "SPEAKOFLOW_CLAUDE_PATH points at {}, which is not a file.",
            path.display()
        ));
    }

    let names: &[&str] = if cfg!(windows) {
        &["claude.exe", "claude.cmd", "claude.bat", "claude"]
    } else {
        &["claude"]
    };

    // Where the official installers put it, checked before walking PATH so a
    // broken PATH cannot break the feature.
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = resolve_var("USERPROFILE").or_else(|| resolve_var("HOME")) {
        let home = PathBuf::from(home);
        roots.push(home.join(".local").join("bin"));
        roots.push(home.join(".claude").join("local"));
        roots.push(home.join("AppData").join("Roaming").join("npm"));
        roots.push(home.join(".npm-global").join("bin"));
        roots.push(home.join(".bun").join("bin"));
    }
    if let Some(local) = resolve_var("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Programs").join("claude"));
    }
    roots.push(PathBuf::from("/usr/local/bin"));
    roots.push(PathBuf::from("/opt/homebrew/bin"));

    roots.extend(path_dirs());

    for root in roots {
        for name in names {
            let candidate = root.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    Err("Couldn't find the Claude Code CLI. Install it, or set SPEAKOFLOW_CLAUDE_PATH to the full path of claude.exe.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_path_is_deduplicated_and_non_empty() {
        let path = effective_path();
        assert!(!path.is_empty());
        let parts: Vec<String> = path
            .split(';')
            .filter(|p| !p.trim().is_empty())
            .map(|p| p.to_lowercase())
            .collect();
        let mut unique = parts.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(parts.len(), unique.len(), "PATH should not repeat entries");
    }

    #[cfg(windows)]
    #[test]
    fn effective_path_always_includes_system32() {
        assert!(effective_path().to_lowercase().contains("system32"));
    }

    #[test]
    fn live_environment_wins_over_the_registry() {
        // Uses a name nothing else will set, so the assertion is about
        // precedence rather than about this machine's configuration.
        std::env::set_var("SPEAKOFLOW_TEST_PRECEDENCE", "live");
        assert_eq!(
            resolve_var("SPEAKOFLOW_TEST_PRECEDENCE").as_deref(),
            Some("live")
        );
        std::env::remove_var("SPEAKOFLOW_TEST_PRECEDENCE");
        assert!(resolve_var("SPEAKOFLOW_TEST_PRECEDENCE").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn expansion_keeps_unknown_names_visible() {
        std::env::set_var("SPEAKOFLOW_TEST_EXPAND", "value");
        assert_eq!(expand("a%SPEAKOFLOW_TEST_EXPAND%b"), "avalueb");
        assert_eq!(expand("%NO_SUCH_VAR_HERE%"), "%NO_SUCH_VAR_HERE%");
        assert_eq!(expand("plain"), "plain");
        assert_eq!(expand("50%"), "50%");
        std::env::remove_var("SPEAKOFLOW_TEST_EXPAND");
    }
}
