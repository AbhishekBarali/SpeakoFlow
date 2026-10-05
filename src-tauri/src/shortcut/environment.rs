//! Whether global shortcuts can reach SpeakoFlow on this Linux session, and
//! what to do when they cannot.
//!
//! Neither keyboard engine can hear a key pressed in another app on most Linux
//! desktops today, and both used to fail without a word: the Settings page
//! showed the keys as if they worked.
//!
//! * **Tauri** (the Linux default) registers through X11 (`XGrabKey`). On a
//!   Wayland session that reaches the XWayland server only, which sees a key
//!   while an X11 window is in front and nothing otherwise. GNOME, KDE Plasma,
//!   Hyprland, Sway and COSMIC all run Wayland by default.
//! * **SpeakoFlow keys** (handy-keys) read the keyboard devices directly, which
//!   works on any desktop, but `rdev`'s grab opens every `/dev/input/event*`
//!   (normally `root:input 0660`), clones each through `/dev/uinput` (normally
//!   `root:root 0600`, so the `input` group alone is not enough) and needs an
//!   X display for key names. When any of that is missing the listener thread
//!   gave up with one line on stderr, which no log file captured.
//!
//! What always works is a shortcut the desktop itself owns, set to run
//! `speakoflow --toggle-transcription` and friends. [`shortcut_environment`]
//! says which case this is and lists those commands, so the app can explain
//! it next to the keys.

// The probing runs on Linux only; elsewhere the command answers `Works`, and
// the helpers stay compiled so their tests run on every platform.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use crate::settings::{self, AppSettings, KeyboardImplementation};

/// Whether shortcuts pressed in other apps reach SpeakoFlow.
#[derive(Serialize, Type, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutReach {
    /// They do (or this platform needs nothing explained).
    Works,
    /// Wayland with the Tauri engine: heard only while an X11 window is in
    /// front, so in practice not at all.
    WaylandX11Only,
    /// The SpeakoFlow keys engine cannot open the keyboard devices.
    NoKeyboardAccess,
}

/// The desktop, as far as it changes where custom shortcuts are added.
#[derive(Serialize, Type, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LinuxDesktop {
    Gnome,
    Kde,
    Other,
}

/// What the SpeakoFlow keys engine is missing. Each needs its own fix.
#[derive(Serialize, Type, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardAccessGap {
    /// A `/dev/input/event*` device cannot be opened for reading.
    InputDevices,
    /// `/dev/uinput` cannot be opened for writing.
    Uinput,
    /// No X display (`DISPLAY`), which `rdev` needs for key names.
    Display,
}

/// One action and the command a desktop shortcut runs to trigger it.
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct DesktopShortcutCommand {
    /// The binding id (`transcribe`, `assistant_call`, ...).
    pub id: String,
    /// The full command line, ready to paste.
    pub command: String,
}

/// Everything the Shortcuts card needs to explain a Linux session.
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct ShortcutEnvironment {
    pub reach: ShortcutReach,
    pub desktop: LinuxDesktop,
    /// Filled for `NoKeyboardAccess`.
    pub missing: Vec<KeyboardAccessGap>,
    /// A terminal command that grants what `missing` lists, or empty.
    pub access_command: String,
    /// One command per action that is in use, for desktop shortcuts.
    pub commands: Vec<DesktopShortcutCommand>,
}

impl ShortcutEnvironment {
    fn works() -> Self {
        Self {
            reach: ShortcutReach::Works,
            desktop: LinuxDesktop::Other,
            missing: Vec::new(),
            access_command: String::new(),
            commands: Vec::new(),
        }
    }
}

/// The reach of an engine on a session. Pure, so every case is tested.
pub fn reach_for(
    engine: KeyboardImplementation,
    wayland: bool,
    missing: &[KeyboardAccessGap],
) -> ShortcutReach {
    match engine {
        KeyboardImplementation::HandyKeys if !missing.is_empty() => ShortcutReach::NoKeyboardAccess,
        KeyboardImplementation::HandyKeys => ShortcutReach::Works,
        KeyboardImplementation::Tauri if wayland => ShortcutReach::WaylandX11Only,
        KeyboardImplementation::Tauri => ShortcutReach::Works,
    }
}

/// Which desktop `XDG_CURRENT_DESKTOP` names (a `:`-separated list).
pub fn desktop_from(current_desktop: &str) -> LinuxDesktop {
    let names = current_desktop.to_ascii_lowercase();
    let has = |name: &str| names.split(':').any(|part| part.trim() == name);
    if has("kde") {
        LinuxDesktop::Kde
    } else if has("gnome") || has("ubuntu") || has("unity") {
        LinuxDesktop::Gnome
    } else {
        LinuxDesktop::Other
    }
}

/// The command flag that triggers each binding from outside the app, in the
/// order the Shortcuts card lists them.
const FLAGS: [(&str, &str); 5] = [
    ("transcribe", "--toggle-transcription"),
    ("transcribe_with_post_process", "--toggle-post-process"),
    ("assistant", "--toggle-assistant"),
    ("assistant_call", "--toggle-call"),
    ("cancel", "--cancel"),
];

/// The commands for the actions in use: cleanup's own row only when it has a
/// shortcut of its own, the assistant's two only while it is on.
pub fn desktop_commands(settings: &AppSettings, program: &str) -> Vec<DesktopShortcutCommand> {
    FLAGS
        .iter()
        .filter(|(id, _)| match *id {
            settings::CLEANUP_BINDING_ID => settings::cleanup_binding_active(settings),
            "assistant" | "assistant_call" => settings.assistant_enabled,
            _ => true,
        })
        .map(|(id, flag)| DesktopShortcutCommand {
            id: (*id).to_string(),
            command: format!("{program} {flag}"),
        })
        .collect()
}

/// A path as one shell word: bare when it is safe, single-quoted otherwise.
pub fn shell_word(path: &str) -> String {
    let safe = !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c));
    if safe {
        path.to_string()
    } else {
        format!("'{}'", path.replace('\'', r"'\''"))
    }
}

/// How a desktop shortcut should name this program. An AppImage runs from a
/// temporary mount, so its own file (`$APPIMAGE`) is the stable path; an
/// installed binary on the usual PATH is named bare.
pub fn program_name(appimage: Option<&str>, current_exe: Option<&str>) -> String {
    if let Some(path) = appimage.filter(|p| !p.is_empty()) {
        return shell_word(path);
    }
    match current_exe {
        Some(path) => {
            let in_path_dir = ["/usr/bin/", "/usr/local/bin/", "/bin/"].iter().any(|dir| {
                path.strip_prefix(dir)
                    .is_some_and(|rest| !rest.contains('/'))
            });
            if in_path_dir {
                shell_word(path.rsplit('/').next().unwrap_or(path))
            } else {
                shell_word(path)
            }
        }
        None => "speakoflow".to_string(),
    }
}

/// The terminal command that grants what is missing. The input group gives
/// read access to the keyboards; a udev rule makes `/dev/uinput` writable by
/// that group, which no distribution does by default. Both need a fresh login
/// to apply to the running session. A missing display has no command.
pub fn access_command_for(missing: &[KeyboardAccessGap]) -> String {
    let mut steps: Vec<&str> = Vec::new();
    if missing.contains(&KeyboardAccessGap::InputDevices)
        || missing.contains(&KeyboardAccessGap::Uinput)
    {
        // The group is what the uinput rule grants to, so it is needed either way.
        steps.push("sudo usermod -aG input \"$USER\"");
    }
    if missing.contains(&KeyboardAccessGap::Uinput) {
        steps.push(
            "echo 'KERNEL==\"uinput\", GROUP=\"input\", MODE=\"0660\"' | sudo tee /etc/udev/rules.d/70-speakoflow-uinput.rules",
        );
        steps.push("sudo udevadm control --reload");
        steps.push("sudo udevadm trigger /dev/uinput");
    }
    steps.join(" && ")
}

/// What the SpeakoFlow keys engine would be missing on this machine right now.
#[cfg(target_os = "linux")]
pub fn keyboard_access_gaps() -> Vec<KeyboardAccessGap> {
    use std::os::unix::fs::FileTypeExt;

    let mut missing = Vec::new();
    // The same set `rdev` opens: every character device in /dev/input except
    // the legacy mouse nodes it skips. One it cannot open fails the whole grab.
    let devices: Vec<std::path::PathBuf> = std::fs::read_dir("/dev/input")
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|t| t.is_char_device()))
                .map(|entry| entry.path())
                .filter(|path| {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    name != "mice" && !name.starts_with("mouse") && !name.starts_with("js")
                })
                .collect()
        })
        .unwrap_or_default();
    if devices.is_empty()
        || devices
            .iter()
            .any(|path| std::fs::File::open(path).is_err())
    {
        missing.push(KeyboardAccessGap::InputDevices);
    }
    if std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_err()
    {
        missing.push(KeyboardAccessGap::Uinput);
    }
    if std::env::var_os("DISPLAY").is_none_or(|d| d.is_empty()) {
        missing.push(KeyboardAccessGap::Display);
    }
    missing
}

/// Whether shortcuts reach the app on this session, and the commands for
/// desktop shortcuts when they do not. Off Linux there is nothing to explain.
#[tauri::command]
#[specta::specta]
pub fn get_shortcut_environment(app: AppHandle) -> ShortcutEnvironment {
    #[cfg(target_os = "linux")]
    {
        let settings = settings::get_settings(&app);
        let engine = settings.keyboard_implementation;
        let missing = match engine {
            KeyboardImplementation::HandyKeys => keyboard_access_gaps(),
            KeyboardImplementation::Tauri => Vec::new(),
        };
        let reach = reach_for(engine, crate::utils::is_wayland(), &missing);
        if reach == ShortcutReach::Works {
            return ShortcutEnvironment::works();
        }
        let appimage = std::env::var("APPIMAGE").ok();
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(str::to_string));
        let program = program_name(appimage.as_deref(), exe.as_deref());
        ShortcutEnvironment {
            reach,
            desktop: desktop_from(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()),
            access_command: access_command_for(&missing),
            missing,
            commands: desktop_commands(&settings, &program),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        ShortcutEnvironment::works()
    }
}

/// Write the reason to the log when shortcuts cannot work, once at startup, so
/// a "my shortcuts do nothing" report carries it.
#[cfg(target_os = "linux")]
pub fn log_startup_environment(app: &AppHandle) {
    let environment = get_shortcut_environment(app.clone());
    match environment.reach {
        ShortcutReach::Works => {}
        ShortcutReach::WaylandX11Only => log::warn!(
            "Global shortcuts: this is a Wayland session and the keyboard engine is Tauri, \
             which only hears keys while an X11 window is focused. Desktop shortcuts that run \
             the CLI flags work instead (see the Shortcuts card)."
        ),
        ShortcutReach::NoKeyboardAccess => log::warn!(
            "Global shortcuts: the SpeakoFlow keys engine cannot read the keyboard ({:?}); \
             no shortcut will fire",
            environment.missing
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tauri_on_wayland_does_not_reach_other_apps() {
        assert_eq!(
            reach_for(KeyboardImplementation::Tauri, true, &[]),
            ShortcutReach::WaylandX11Only
        );
        assert_eq!(
            reach_for(KeyboardImplementation::Tauri, false, &[]),
            ShortcutReach::Works
        );
    }

    #[test]
    fn speakoflow_keys_work_anywhere_with_access_and_nowhere_without() {
        for wayland in [true, false] {
            assert_eq!(
                reach_for(KeyboardImplementation::HandyKeys, wayland, &[]),
                ShortcutReach::Works
            );
            assert_eq!(
                reach_for(
                    KeyboardImplementation::HandyKeys,
                    wayland,
                    &[KeyboardAccessGap::Uinput]
                ),
                ShortcutReach::NoKeyboardAccess
            );
        }
    }

    #[test]
    fn desktops_are_read_from_the_whole_list() {
        assert_eq!(desktop_from("ubuntu:GNOME"), LinuxDesktop::Gnome);
        assert_eq!(desktop_from("GNOME"), LinuxDesktop::Gnome);
        assert_eq!(desktop_from("KDE"), LinuxDesktop::Kde);
        assert_eq!(desktop_from("Hyprland"), LinuxDesktop::Other);
        assert_eq!(desktop_from("sway"), LinuxDesktop::Other);
        assert_eq!(desktop_from(""), LinuxDesktop::Other);
    }

    #[test]
    fn an_appimage_is_named_by_its_own_file() {
        assert_eq!(
            program_name(
                Some("/home/me/Apps/SpeakoFlow_1.5.0_amd64.AppImage"),
                Some("/tmp/.mount_SpeakXYZ/usr/bin/speakoflow"),
            ),
            "/home/me/Apps/SpeakoFlow_1.5.0_amd64.AppImage"
        );
        assert_eq!(
            program_name(Some("/home/me/My Apps/SpeakoFlow.AppImage"), None),
            "'/home/me/My Apps/SpeakoFlow.AppImage'"
        );
    }

    #[test]
    fn an_installed_binary_is_named_bare() {
        assert_eq!(
            program_name(None, Some("/usr/bin/speakoflow")),
            "speakoflow"
        );
        assert_eq!(
            program_name(None, Some("/opt/speakoflow/speakoflow")),
            "/opt/speakoflow/speakoflow"
        );
        assert_eq!(program_name(None, None), "speakoflow");
    }

    #[test]
    fn shell_words_survive_quotes() {
        assert_eq!(shell_word("/a/it's"), r"'/a/it'\''s'");
    }

    #[test]
    fn commands_follow_the_actions_in_use() {
        let mut settings = settings::get_default_settings();
        settings.assistant_enabled = true;
        settings.post_process_enabled = false;
        let ids: Vec<String> = desktop_commands(&settings, "speakoflow")
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, ["transcribe", "assistant", "assistant_call", "cancel"]);

        settings.assistant_enabled = false;
        let commands = desktop_commands(&settings, "speakoflow");
        assert_eq!(
            commands
                .iter()
                .map(|c| c.command.as_str())
                .collect::<Vec<_>>(),
            ["speakoflow --toggle-transcription", "speakoflow --cancel"]
        );
    }

    #[test]
    fn the_access_command_grants_only_what_is_missing() {
        assert_eq!(access_command_for(&[]), "");
        assert_eq!(
            access_command_for(&[KeyboardAccessGap::InputDevices]),
            "sudo usermod -aG input \"$USER\""
        );
        let both =
            access_command_for(&[KeyboardAccessGap::InputDevices, KeyboardAccessGap::Uinput]);
        assert!(both.starts_with("sudo usermod -aG input \"$USER\" && echo 'KERNEL==\"uinput\""));
        assert!(both.ends_with("sudo udevadm trigger /dev/uinput"));
        assert_eq!(access_command_for(&[KeyboardAccessGap::Display]), "");
    }
}
