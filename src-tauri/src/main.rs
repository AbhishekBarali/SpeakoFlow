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

    speakoflow_app_lib::run(cli_args)
}
