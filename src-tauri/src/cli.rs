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
}
