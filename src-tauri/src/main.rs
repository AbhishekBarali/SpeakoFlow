// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use speakoflow_app_lib::CliArgs;

fn main() {
    let cli_args = CliArgs::parse();

    // `--list-devices` is a headless probe: initialize the transcribe.cpp
    // backends, print the compute devices, and exit WITHOUT launching Tauri.
    // Handled here (before any window/single-instance setup) so it stays a
    // fast, side-effect-free CLI. See CI's packaged-build audit (Session 7).
    if cli_args.list_devices {
        speakoflow_app_lib::list_transcribe_devices();
        return;
    }

    #[cfg(target_os = "linux")]
    {
        // DMABUF renderer causes crashes on various GPU/display server configurations
        // See: https://github.com/tauri-apps/tauri/issues/9394
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    // Make the always-on-top recording overlay actually float above other apps
    // on GNOME/Wayland. Must run before any GTK/GDK initialization.
    prefer_xwayland_for_overlay_on_gnome_wayland();

    speakoflow_app_lib::run(cli_args)
}

/// Prefer XWayland on GNOME/Wayland so the recording overlay can stay above
/// other applications.
///
/// The overlay relies on being kept above every other window. There are only
/// two ways to achieve that on Linux:
///   * `wlr-layer-shell` (used via `gtk-layer-shell`) — supported by wlroots
///     compositors (Sway, Hyprland) and KDE Plasma's KWin, and
///   * X11 "keep above" stacking — honored by every X11 window manager and by
///     XWayland.
///
/// GNOME's Mutter deliberately does **not** implement `wlr-layer-shell`, and
/// Wayland has no client API to raise a window to the top. So under a native
/// GNOME/Wayland session the overlay cannot float above other apps at all —
/// the single most common Linux beta complaint. Running the process under
/// XWayland restores X11 keep-above semantics, which the existing overlay
/// fallback (`force_overlay_keep_above`) already uses, so the overlay works.
///
/// This only forces XWayland on GNOME/Wayland. wlroots and KDE Wayland keep
/// their native Wayland session (where `gtk-layer-shell` works and is better),
/// and X11 sessions are untouched. It is a no-op when:
///   * the user already set `GDK_BACKEND` (their choice wins),
///   * `SPEAKOFLOW_ALLOW_WAYLAND` is set (explicit opt-out), or
///   * the session is not detectably GNOME-on-Wayland (e.g. non-Linux, X11,
///     KDE, wlroots).
///
/// Pure `std::env` so it compiles on every platform and is a harmless no-op
/// off Linux (the XDG/Wayland variables are absent, and `GDK_BACKEND` is
/// meaningless outside GTK).
fn prefer_xwayland_for_overlay_on_gnome_wayland() {
    // Respect an explicit user/backend choice — never override it.
    if std::env::var_os("GDK_BACKEND").is_some() {
        return;
    }
    // Explicit opt-out for users who prefer native Wayland (and accept that the
    // overlay won't float on GNOME).
    if std::env::var_os("SPEAKOFLOW_ALLOW_WAYLAND").is_some() {
        return;
    }

    let is_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false);

    // XDG_CURRENT_DESKTOP can be colon-separated (e.g. "ubuntu:GNOME").
    let is_gnome = std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| {
            v.split(':').any(|part| part.eq_ignore_ascii_case("gnome"))
                || v.to_ascii_lowercase().contains("gnome")
        })
        .unwrap_or(false);

    if is_wayland && is_gnome {
        std::env::set_var("GDK_BACKEND", "x11");
        // Log init hasn't run yet, so print directly. Visible for terminal
        // launches; harmless otherwise.
        eprintln!(
            "SpeakoFlow: GNOME/Wayland detected — running under XWayland so the \
             recording overlay can float above other apps. Set \
             SPEAKOFLOW_ALLOW_WAYLAND=1 to keep native Wayland (the overlay may \
             not stay on top there)."
        );
    }
}
