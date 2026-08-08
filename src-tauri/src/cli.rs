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
    /// and exit. The app spawns itself with this on Linux so that a SIGILL or a
    /// hang inside the vendored ggml/Vulkan code kills only this short-lived
    /// child instead of the app at launch (see
    /// `managers::transcription::probe_devices_out_of_process`). Hidden because
    /// it is an implementation detail, not a user-facing switch.
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
}
