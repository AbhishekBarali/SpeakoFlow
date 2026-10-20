use clap::Parser;

#[derive(Parser, Debug, Clone, Default)]
#[command(name = "speakoflow", about = "SpeakoFlow - Speech to Text")]
pub struct CliArgs {
    /// Start with the main window hidden
    #[arg(long)]
    pub start_hidden: bool,

    /// Disable the system tray icon
    #[arg(long)]
    pub no_tray: bool,

    /// Toggle transcription on/off (sent to running instance)
    #[arg(long)]
    pub toggle_transcription: bool,

    /// Toggle transcription with post-processing on/off (sent to running instance)
    #[arg(long)]
    pub toggle_post_process: bool,

    /// Toggle the voice assistant on/off (sent to running instance)
    #[arg(long)]
    pub toggle_assistant: bool,

    /// Start a hands-free conversation with the assistant, or end the one in
    /// progress (sent to running instance)
    #[arg(long)]
    pub toggle_call: bool,

    /// Cancel the current operation (sent to running instance)
    #[arg(long)]
    pub cancel: bool,

    /// Enable debug mode with verbose logging
    #[arg(long)]
    pub debug: bool,

    /// List the transcribe.cpp compute devices (and backend availability) then
    /// exit, without launching the app. Used to verify a packaged build's
    /// bundled ggml backend libraries load and register a device on a machine
    /// with no dev toolchain / no Vulkan SDK (the Session 7 clean-machine gate).
    #[arg(long)]
    pub list_devices: bool,

    /// Internal: enumerate the compute devices, print them as one line of JSON,
    /// and exit; the machine-readable sibling of `--list-devices`, for scripts
    /// and audits. (The app used to spawn itself with it on Linux to list GPUs
    /// out of process; the transcription worker does that on every platform
    /// now.) Hidden because it is not a user-facing switch.
    ///
    /// The transcription worker's own flag (`--transcribe-worker`) is not
    /// declared here: `main` handles it before these arguments are parsed.
    #[arg(long, hide = true)]
    pub probe_devices: bool,
}

#[cfg(test)]
mod tests {
    use super::CliArgs;
    use clap::Parser;

    #[test]
    fn parses_toggle_assistant() {
        let args = CliArgs::try_parse_from(["speakoflow", "--toggle-assistant"])
            .expect("--toggle-assistant should be a supported background command");
        assert!(args.toggle_assistant);
    }

    /// The conversation shortcut needs a flag of its own, for desktop shortcuts
    /// on Linux (Wayland), where the app's own keys cannot be heard.
    #[test]
    fn parses_toggle_call() {
        let args = CliArgs::try_parse_from(["speakoflow", "--toggle-call"])
            .expect("--toggle-call should be a supported background command");
        assert!(args.toggle_call);
    }
}
