use log::{debug, warn};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::fmt;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

pub const APPLE_INTELLIGENCE_PROVIDER_ID: &str = "apple_intelligence";
pub const APPLE_INTELLIGENCE_DEFAULT_MODEL_ID: &str = "Apple Intelligence";

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

// Custom deserializer to handle both old numeric format (1-5) and new string format ("trace", "debug", etc.)
impl<'de> Deserialize<'de> for LogLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LogLevelVisitor;

        impl<'de> Visitor<'de> for LogLevelVisitor {
            type Value = LogLevel;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string or integer representing log level")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<LogLevel, E> {
                match value.to_lowercase().as_str() {
                    "trace" => Ok(LogLevel::Trace),
                    "debug" => Ok(LogLevel::Debug),
                    "info" => Ok(LogLevel::Info),
                    "warn" => Ok(LogLevel::Warn),
                    "error" => Ok(LogLevel::Error),
                    _ => Err(E::unknown_variant(
                        value,
                        &["trace", "debug", "info", "warn", "error"],
                    )),
                }
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<LogLevel, E> {
                match value {
                    1 => Ok(LogLevel::Trace),
                    2 => Ok(LogLevel::Debug),
                    3 => Ok(LogLevel::Info),
                    4 => Ok(LogLevel::Warn),
                    5 => Ok(LogLevel::Error),
                    _ => Err(E::invalid_value(de::Unexpected::Unsigned(value), &"1-5")),
                }
            }
        }

        deserializer.deserialize_any(LogLevelVisitor)
    }
}

impl From<LogLevel> for tauri_plugin_log::LogLevel {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => tauri_plugin_log::LogLevel::Trace,
            LogLevel::Debug => tauri_plugin_log::LogLevel::Debug,
            LogLevel::Info => tauri_plugin_log::LogLevel::Info,
            LogLevel::Warn => tauri_plugin_log::LogLevel::Warn,
            LogLevel::Error => tauri_plugin_log::LogLevel::Error,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ShortcutBinding {
    pub id: String,
    pub name: String,
    pub description: String,
    pub default_binding: String,
    pub current_binding: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct LLMPrompt {
    pub id: String,
    pub name: String,
    pub prompt: String,
}

/// Case transform applied to the output of a text replacement rule.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default, Type)]
#[serde(rename_all = "snake_case")]
pub enum Capitalization {
    /// Leave the replacement text as written.
    #[default]
    None,
    /// UPPERCASE the whole replacement.
    Uppercase,
    /// lowercase the whole replacement.
    Lowercase,
    /// Capitalize the first character of the replacement.
    Capitalize,
}

/// A single deterministic find/replace rule applied to the transcript.
///
/// Rules run as a fast, offline, deterministic pass that complements (does not
/// duplicate) the optional LLM post-processing. `search` is matched literally
/// by default; set `is_regex` to treat it as a regular expression. `replace`
/// may contain magic commands such as `[date]`, `[time]`, `[uppercase]`,
/// `[lowercase]`, `[capitalize]`, and `[nospace]`.
#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct Replacement {
    /// Text (or regex pattern when `is_regex` is set) to search for.
    pub search: String,
    /// Replacement text. Supports the magic commands described on the struct.
    pub replace: String,
    /// Treat `search` as a regular expression instead of a literal string.
    #[serde(default)]
    pub is_regex: bool,
    /// Whether this rule is applied. Disabled rules are kept but skipped.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Remove whitespace immediately before each match.
    #[serde(default)]
    pub trim_before: bool,
    /// Remove whitespace immediately after each match.
    #[serde(default)]
    pub trim_after: bool,
    /// Case transform applied to this rule's output.
    #[serde(default)]
    pub capitalization: Capitalization,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct PostProcessProvider {
    pub id: String,
    pub label: String,
    pub base_url: String,
    #[serde(default)]
    pub allow_base_url_edit: bool,
    #[serde(default)]
    pub models_endpoint: Option<String>,
    #[serde(default)]
    pub supports_structured_output: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum OverlayPosition {
    None,
    Top,
    Bottom,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ModelUnloadTimeout {
    Never,
    Immediately,
    Min2,
    Min5,
    Min10,
    Min15,
    Hour1,
    Sec15, // Debug mode only
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum PasteMethod {
    CtrlV,
    Direct,
    None,
    ShiftInsert,
    CtrlShiftV,
    ExternalScript,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardHandling {
    DontModify,
    CopyToClipboard,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum AutoSubmitKey {
    Enter,
    CtrlEnter,
    CmdEnter,
}

/// Desired length of the assistant's replies. Appended as a directive to the
/// system prompt at request time, so it works with the single main prompt
/// (no separate summary layer). `Default` injects nothing.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default, Type)]
#[serde(rename_all = "snake_case")]
pub enum AssistantResponseLength {
    /// No length directive — use the system prompt as-is.
    #[default]
    Default,
    Short,
    Medium,
    Long,
}

impl AssistantResponseLength {
    /// The instruction appended to the system prompt, or `None` for `Default`.
    pub fn directive(&self) -> Option<&'static str> {
        match self {
            AssistantResponseLength::Default => None,
            AssistantResponseLength::Short => Some(
                "Keep your reply very short — usually one or two sentences. Match the user's intent: a greeting or trivial message gets a brief, friendly reply, never a long one.",
            ),
            AssistantResponseLength::Medium => Some(
                "Keep replies fairly brief — a short paragraph at most. Don't pad simple messages with extra detail.",
            ),
            AssistantResponseLength::Long => Some(
                "Give thorough, detailed replies when the question genuinely calls for it. Still match the user's intent: greetings or trivial messages get a short reply, not filler.",
            ),
        }
    }
}

/// How thorough a web search should be. This is the single dial that replaces
/// the old raw "max results" number: it controls how many queries run, how many
/// pages get scraped, and how much source text the model receives. All three
/// tiers are tuned to stay fast (one retrieval pass, heavy parallelism, tight
/// timeouts) — this is "answer-with-search like ChatGPT/Gemini do in seconds",
/// not minutes-long deep research.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default, Type)]
#[serde(rename_all = "snake_case")]
pub enum AssistantSearchDepth {
    /// Fastest. One query, snippets + a couple of scraped pages. Quick facts.
    Low,
    /// Balanced default. A few queries, rerank, scrape the top handful.
    #[default]
    Medium,
    /// Broadest single pass. More queries/sources, scrape more winners.
    High,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum RecordingRetentionPeriod {
    Never,
    PreserveLimit,
    Days3,
    Weeks2,
    Months3,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardImplementation {
    Tauri,
    HandyKeys,
}

impl Default for KeyboardImplementation {
    fn default() -> Self {
        #[cfg(target_os = "linux")]
        return KeyboardImplementation::Tauri;
        #[cfg(not(target_os = "linux"))]
        return KeyboardImplementation::HandyKeys;
    }
}

impl Default for ModelUnloadTimeout {
    fn default() -> Self {
        ModelUnloadTimeout::Min5
    }
}

impl Default for PasteMethod {
    fn default() -> Self {
        // Default to CtrlV for macOS and Windows, Direct for Linux
        #[cfg(target_os = "linux")]
        return PasteMethod::Direct;
        #[cfg(not(target_os = "linux"))]
        return PasteMethod::CtrlV;
    }
}

impl Default for ClipboardHandling {
    fn default() -> Self {
        ClipboardHandling::DontModify
    }
}

impl Default for AutoSubmitKey {
    fn default() -> Self {
        AutoSubmitKey::Enter
    }
}

impl ModelUnloadTimeout {
    pub fn to_minutes(self) -> Option<u64> {
        match self {
            ModelUnloadTimeout::Never => None,
            ModelUnloadTimeout::Immediately => Some(0), // Special case for immediate unloading
            ModelUnloadTimeout::Min2 => Some(2),
            ModelUnloadTimeout::Min5 => Some(5),
            ModelUnloadTimeout::Min10 => Some(10),
            ModelUnloadTimeout::Min15 => Some(15),
            ModelUnloadTimeout::Hour1 => Some(60),
            ModelUnloadTimeout::Sec15 => Some(0), // Special case for debug - handled separately
        }
    }

    pub fn to_seconds(self) -> Option<u64> {
        match self {
            ModelUnloadTimeout::Never => None,
            ModelUnloadTimeout::Immediately => Some(0), // Special case for immediate unloading
            ModelUnloadTimeout::Sec15 => Some(15),
            _ => self.to_minutes().map(|m| m * 60),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SoundTheme {
    Marimba,
    Pop,
    Click,
    Custom,
}

impl SoundTheme {
    fn as_str(&self) -> &'static str {
        match self {
            SoundTheme::Marimba => "marimba",
            SoundTheme::Pop => "pop",
            SoundTheme::Click => "click",
            SoundTheme::Custom => "custom",
        }
    }

    pub fn to_start_path(&self) -> String {
        format!("resources/{}_start.wav", self.as_str())
    }

    pub fn to_stop_path(&self) -> String {
        format!("resources/{}_stop.wav", self.as_str())
    }
}

/// UI appearance preference. `System` follows the OS; `Light` / `Dark` pin the
/// theme regardless of the OS setting. Serialized lowercase ("light", "dark",
/// "system") to match the `data-theme` attribute the frontend sets on <html>.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    System,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::System
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum TypingTool {
    Auto,
    Wtype,
    Kwtype,
    Dotool,
    Ydotool,
    Xdotool,
}

impl Default for TypingTool {
    fn default() -> Self {
        TypingTool::Auto
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum WhisperAcceleratorSetting {
    Auto,
    Cpu,
    Gpu,
}

impl Default for WhisperAcceleratorSetting {
    fn default() -> Self {
        WhisperAcceleratorSetting::Auto
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum OrtAcceleratorSetting {
    Auto,
    Cpu,
    Cuda,
    #[serde(rename = "directml")]
    DirectMl,
    Rocm,
}

impl Default for OrtAcceleratorSetting {
    fn default() -> Self {
        OrtAcceleratorSetting::Auto
    }
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub(crate) struct SecretMap(HashMap<String, String>);

impl fmt::Debug for SecretMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted: HashMap<&String, &str> = self
            .0
            .iter()
            .map(|(k, v)| (k, if v.is_empty() { "" } else { "[REDACTED]" }))
            .collect();
        redacted.fmt(f)
    }
}

impl std::ops::Deref for SecretMap {
    type Target = HashMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for SecretMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone, Default, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct SecretString(pub String);

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            f.write_str("\"\"")
        } else {
            f.write_str("\"[REDACTED]\"")
        }
    }
}

/* still handy for composing the initial JSON in the store ------------- */
#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct AppSettings {
    pub bindings: HashMap<String, ShortcutBinding>,
    pub push_to_talk: bool,
    pub audio_feedback: bool,
    #[serde(default = "default_audio_feedback_volume")]
    pub audio_feedback_volume: f32,
    #[serde(default = "default_sound_theme")]
    pub sound_theme: SoundTheme,
    #[serde(default = "default_start_hidden")]
    pub start_hidden: bool,
    #[serde(default = "default_autostart_enabled")]
    pub autostart_enabled: bool,
    #[serde(default = "default_update_checks_enabled")]
    pub update_checks_enabled: bool,
    #[serde(default = "default_model")]
    pub selected_model: String,
    #[serde(default = "default_always_on_microphone")]
    pub always_on_microphone: bool,
    #[serde(default)]
    pub selected_microphone: Option<String>,
    #[serde(default)]
    pub clamshell_microphone: Option<String>,
    #[serde(default)]
    pub selected_output_device: Option<String>,
    #[serde(default = "default_translate_to_english")]
    pub translate_to_english: bool,
    #[serde(default = "default_selected_language")]
    pub selected_language: String,
    #[serde(default = "default_overlay_position")]
    pub overlay_position: OverlayPosition,
    #[serde(default = "default_debug_mode")]
    pub debug_mode: bool,
    #[serde(default = "default_log_level")]
    pub log_level: LogLevel,
    #[serde(default)]
    pub custom_words: Vec<String>,
    /// Master switch for the deterministic text-replacements pass.
    #[serde(default)]
    pub replacements_enabled: bool,
    /// Ordered list of find/replace rules applied after LLM post-processing.
    #[serde(default = "default_text_replacements")]
    pub text_replacements: Vec<Replacement>,
    #[serde(default)]
    pub model_unload_timeout: ModelUnloadTimeout,
    /// Idle timeout after which the built-in local LLM engine (llama.cpp
    /// sidecar) is unloaded to free RAM/VRAM. Mirrors `model_unload_timeout`
    /// but applies to the LLM used for post-processing and the assistant.
    #[serde(default = "default_local_llm_unload_timeout")]
    pub local_llm_unload_timeout: ModelUnloadTimeout,
    #[serde(default = "default_word_correction_threshold")]
    pub word_correction_threshold: f64,
    #[serde(default = "default_history_limit")]
    pub history_limit: usize,
    #[serde(default = "default_recording_retention_period")]
    pub recording_retention_period: RecordingRetentionPeriod,
    #[serde(default)]
    pub paste_method: PasteMethod,
    #[serde(default)]
    pub clipboard_handling: ClipboardHandling,
    #[serde(default = "default_auto_submit")]
    pub auto_submit: bool,
    #[serde(default)]
    pub auto_submit_key: AutoSubmitKey,
    #[serde(default = "default_post_process_enabled")]
    pub post_process_enabled: bool,
    #[serde(default = "default_post_process_provider_id")]
    pub post_process_provider_id: String,
    #[serde(default = "default_post_process_providers")]
    pub post_process_providers: Vec<PostProcessProvider>,
    #[serde(default = "default_post_process_api_keys")]
    pub post_process_api_keys: SecretMap,
    #[serde(default = "default_post_process_models")]
    pub post_process_models: HashMap<String, String>,
    #[serde(default = "default_post_process_prompts")]
    pub post_process_prompts: Vec<LLMPrompt>,
    #[serde(default)]
    pub post_process_selected_prompt_id: Option<String>,
    #[serde(default)]
    pub mute_while_recording: bool,
    #[serde(default)]
    pub append_trailing_space: bool,
    #[serde(default = "default_app_language")]
    pub app_language: String,
    #[serde(default)]
    pub experimental_enabled: bool,
    #[serde(default)]
    pub lazy_stream_close: bool,
    #[serde(default)]
    pub keyboard_implementation: KeyboardImplementation,
    #[serde(default = "default_show_tray_icon")]
    pub show_tray_icon: bool,
    #[serde(default = "default_paste_delay_ms")]
    pub paste_delay_ms: u64,
    #[serde(default = "default_typing_tool")]
    pub typing_tool: TypingTool,
    pub external_script_path: Option<String>,
    #[serde(default)]
    pub custom_filler_words: Option<Vec<String>>,
    #[serde(default)]
    pub whisper_accelerator: WhisperAcceleratorSetting,
    #[serde(default)]
    pub ort_accelerator: OrtAcceleratorSetting,
    #[serde(default = "default_whisper_gpu_device")]
    pub whisper_gpu_device: i32,
    #[serde(default)]
    pub extra_recording_buffer_ms: u64,
    #[serde(default = "default_assistant_provider_id")]
    pub assistant_provider_id: String,
    #[serde(default)]
    pub assistant_models: HashMap<String, String>,
    #[serde(default = "default_assistant_system_prompt")]
    pub assistant_system_prompt: String,
    #[serde(default = "default_assistant_screenshot_enabled")]
    pub assistant_screenshot_enabled: bool,
    #[serde(default)]
    pub assistant_tts_enabled: bool,
    #[serde(default = "default_assistant_tts_engine")]
    pub assistant_tts_engine: String,
    #[serde(default = "default_assistant_tts_voice")]
    pub assistant_tts_voice: String,
    #[serde(default = "default_assistant_tts_base_url")]
    pub assistant_tts_base_url: String,
    #[serde(default)]
    pub assistant_tts_api_key: SecretString,
    #[serde(default = "default_assistant_tts_model")]
    pub assistant_tts_model: String,
    #[serde(default = "default_assistant_tts_remote_voice")]
    pub assistant_tts_remote_voice: String,
    #[serde(default = "default_assistant_tts_kokoro_dtype")]
    pub assistant_tts_kokoro_dtype: String,
    /// Playback speed multiplier for spoken assistant summaries. 1.0 is normal;
    /// 0.5 is half speed, 2.0 is double, etc. Applied locally for Kokoro (via
    /// the webview audio element) and natively for remote engines where the
    /// API supports it.
    #[serde(default = "default_assistant_tts_speed")]
    pub assistant_tts_speed: f64,
    #[serde(default = "default_assistant_max_history_messages")]
    pub assistant_max_history_messages: u32,
    /// Context window (in tokens) the built-in local LLM engine launches with.
    /// Applied when the engine starts; ignored by external providers
    /// (Ollama / LM Studio / cloud), which manage their own context.
    #[serde(default = "default_local_llm_context_size")]
    pub local_llm_context_size: u32,
    #[serde(default)]
    pub assistant_response_length: AssistantResponseLength,
    #[serde(default = "default_assistant_panel_opacity")]
    pub assistant_panel_opacity: f64,
    #[serde(default = "default_assistant_font_size")]
    pub assistant_font_size: String,
    #[serde(default = "default_assistant_accent")]
    pub assistant_accent: String,
    #[serde(default = "default_assistant_panel_size")]
    pub assistant_panel_size: String,
    /// Appearance of the floating assistant panel: "auto" (follow the app
    /// theme), "light", or "dark". A light/dark choice overrides the app-wide
    /// theme for the panel only.
    #[serde(default = "default_assistant_panel_theme")]
    pub assistant_panel_theme: String,
    /// Whether the assistant may search the web. When on, an automatic
    /// heuristic decides per-question whether a search is actually worthwhile
    /// (factual/time-sensitive questions yes; chit-chat, code, math no), so
    /// casual messages stay instant.
    #[serde(default)]
    pub assistant_web_search_enabled: bool,
    /// Which search backend to use: "serper" (default), "brave", "tavily",
    /// "exa", or "serpapi". All are snippet-only and use a single API key.
    #[serde(default = "default_assistant_web_search_provider")]
    pub assistant_web_search_provider: String,
    /// How many results to feed the model. Kept modest to bound prompt size;
    /// clamped to 1–10 at search time.
    #[serde(default = "default_assistant_web_search_max_results")]
    pub assistant_web_search_max_results: u32,
    /// DEPRECATED / unused since web search became snippet-only (Firecrawl and
    /// its page-scrape stage were removed). Kept so existing settings files and
    /// generated bindings stay stable; no current provider reads it.
    #[serde(default = "default_assistant_web_search_fetch_content")]
    pub assistant_web_search_fetch_content: bool,
    /// How thorough web search is (Low/Medium/High). Replaces the old raw
    /// "max results" number as the primary control; tuned to stay fast.
    #[serde(default)]
    pub assistant_search_depth: AssistantSearchDepth,
    /// DEPRECATED / unused since the Firecrawl credit guard was removed (search
    /// is now snippet-only over per-request SERP APIs). Kept so existing
    /// settings files and generated bindings stay stable.
    #[serde(default = "default_assistant_web_search_daily_credit_budget")]
    pub assistant_web_search_daily_credit_budget: u32,
    /// Built-in local model ONLY: when true, decide whether to search with the
    /// same LLM planner the cloud providers use (smarter, but an extra
    /// generation pass — slower, especially on weak hardware). When false
    /// (default), use the instant keyword heuristic. No effect on cloud/custom
    /// providers, which always use the planner.
    #[serde(default)]
    pub assistant_local_search_smart: bool,
    /// API keys for the keyed search providers, keyed by provider id
    /// ("serper", "brave", "tavily", "exa", "serpapi").
    #[serde(default = "default_web_search_api_keys")]
    pub web_search_api_keys: SecretMap,
    #[serde(default)]
    pub theme: Theme,
}

fn default_model() -> String {
    "".to_string()
}

fn default_always_on_microphone() -> bool {
    false
}

fn default_translate_to_english() -> bool {
    false
}

fn default_start_hidden() -> bool {
    false
}

fn default_autostart_enabled() -> bool {
    false
}

fn default_update_checks_enabled() -> bool {
    true
}

fn default_selected_language() -> String {
    "auto".to_string()
}

fn default_overlay_position() -> OverlayPosition {
    #[cfg(target_os = "linux")]
    return OverlayPosition::None;
    #[cfg(not(target_os = "linux"))]
    return OverlayPosition::Bottom;
}

fn default_debug_mode() -> bool {
    false
}

fn default_log_level() -> LogLevel {
    LogLevel::Debug
}

fn default_word_correction_threshold() -> f64 {
    0.18
}

fn default_paste_delay_ms() -> u64 {
    60
}

fn default_auto_submit() -> bool {
    false
}

fn default_history_limit() -> usize {
    5
}

fn default_recording_retention_period() -> RecordingRetentionPeriod {
    RecordingRetentionPeriod::PreserveLimit
}

fn default_audio_feedback_volume() -> f32 {
    1.0
}

fn default_sound_theme() -> SoundTheme {
    SoundTheme::Click
}

fn default_post_process_enabled() -> bool {
    false
}

fn default_app_language() -> String {
    tauri_plugin_os::locale()
        .map(|l| l.replace('_', "-"))
        .unwrap_or_else(|| "en".to_string())
}

fn default_show_tray_icon() -> bool {
    true
}

fn default_post_process_provider_id() -> String {
    "openai".to_string()
}

fn default_post_process_providers() -> Vec<PostProcessProvider> {
    let mut providers = vec![
        PostProcessProvider {
            id: "openai".to_string(),
            label: "OpenAI".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
        },
        PostProcessProvider {
            id: "zai".to_string(),
            label: "Z.AI".to_string(),
            base_url: "https://api.z.ai/api/paas/v4".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
        },
        PostProcessProvider {
            id: "openrouter".to_string(),
            label: "OpenRouter".to_string(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
        },
        PostProcessProvider {
            id: "anthropic".to_string(),
            label: "Anthropic".to_string(),
            base_url: "https://api.anthropic.com/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: false,
        },
        PostProcessProvider {
            id: "groq".to_string(),
            label: "Groq".to_string(),
            base_url: "https://api.groq.com/openai/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: false,
        },
        PostProcessProvider {
            id: "cerebras".to_string(),
            label: "Cerebras".to_string(),
            base_url: "https://api.cerebras.ai/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
        },
        // Google Gemini via its OpenAI-compatible surface. Base URL has NO
        // trailing `/v1` — the app appends `/chat/completions` and `/models`.
        PostProcessProvider {
            id: "gemini".to_string(),
            label: "Google Gemini".to_string(),
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
        },
        PostProcessProvider {
            id: "xai".to_string(),
            label: "xAI (Grok)".to_string(),
            base_url: "https://api.x.ai/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
        },
        PostProcessProvider {
            id: "deepseek".to_string(),
            label: "DeepSeek".to_string(),
            base_url: "https://api.deepseek.com/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: false,
        },
        PostProcessProvider {
            id: "mistral".to_string(),
            label: "Mistral".to_string(),
            base_url: "https://api.mistral.ai/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
        },
        PostProcessProvider {
            id: "moonshot".to_string(),
            label: "Moonshot (Kimi)".to_string(),
            base_url: "https://api.moonshot.ai/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: false,
        },
        PostProcessProvider {
            id: "together".to_string(),
            label: "Together AI".to_string(),
            base_url: "https://api.together.xyz/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: false,
        },
        PostProcessProvider {
            id: "fireworks".to_string(),
            label: "Fireworks AI".to_string(),
            base_url: "https://api.fireworks.ai/inference/v1".to_string(),
            allow_base_url_edit: false,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: false,
        },
        PostProcessProvider {
            id: "perplexity".to_string(),
            label: "Perplexity".to_string(),
            base_url: "https://api.perplexity.ai".to_string(),
            allow_base_url_edit: false,
            models_endpoint: None,
            supports_structured_output: false,
        },
        // Azure OpenAI via the v1 API surface. Users must edit the base URL to
        // their resource, e.g. https://my-res.openai.azure.com/openai/v1
        // (classic dated `?api-version=` deployment endpoints are not supported;
        // the model name is the deployment name). Key auth is sent as both
        // `Authorization: Bearer` and the `api-key` header.
        PostProcessProvider {
            id: "azure_openai".to_string(),
            label: "Azure OpenAI".to_string(),
            base_url: "https://YOUR-RESOURCE.openai.azure.com/openai/v1".to_string(),
            allow_base_url_edit: true,
            models_endpoint: Some("/models".to_string()),
            supports_structured_output: true,
        },
    ];

    // Note: We always include Apple Intelligence on macOS ARM64 without checking availability
    // at startup. The availability check is deferred to when the user actually tries to use it
    // (in actions.rs). This prevents crashes on macOS 26.x beta where accessing
    // SystemLanguageModel.default during early app initialization causes SIGABRT.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        providers.push(PostProcessProvider {
            id: APPLE_INTELLIGENCE_PROVIDER_ID.to_string(),
            label: "Apple Intelligence".to_string(),
            base_url: "apple-intelligence://local".to_string(),
            allow_base_url_edit: false,
            models_endpoint: None,
            supports_structured_output: true,
        });
    }

    // AWS Bedrock via Mantle (OpenAI-compatible endpoint)
    providers.push(PostProcessProvider {
        id: "bedrock_mantle".to_string(),
        label: "AWS Bedrock (Mantle)".to_string(),
        base_url: "https://bedrock-mantle.us-east-1.api.aws/v1".to_string(),
        allow_base_url_edit: false,
        models_endpoint: Some("/models".to_string()),
        supports_structured_output: true,
    });

    // Built-in local LLM (no setup, no API key). Served by the bundled
    // llama.cpp sidecar on a loopback port; the LocalLlmManager starts it on
    // demand against the GGUF model the user downloads from the Models tab.
    providers.push(PostProcessProvider {
        id: "builtin".to_string(),
        label: "Built-in (Local)".to_string(),
        base_url: "http://127.0.0.1:11435/v1".to_string(),
        allow_base_url_edit: false,
        models_endpoint: Some("/models".to_string()),
        supports_structured_output: false,
    });

    // Local OpenAI-compatible servers (Ollama, LM Studio, llama.cpp, vLLM)
    providers.push(PostProcessProvider {
        id: "local".to_string(),
        label: "Local (Ollama / LM Studio)".to_string(),
        base_url: "http://localhost:11434/v1".to_string(),
        allow_base_url_edit: true,
        models_endpoint: Some("/models".to_string()),
        supports_structured_output: false,
    });

    // Custom provider always comes last
    providers.push(PostProcessProvider {
        id: "custom".to_string(),
        label: "Custom".to_string(),
        base_url: "http://localhost:11434/v1".to_string(),
        allow_base_url_edit: true,
        models_endpoint: Some("/models".to_string()),
        supports_structured_output: false,
    });

    providers
}

fn default_post_process_api_keys() -> SecretMap {
    let mut map = HashMap::new();
    for provider in default_post_process_providers() {
        map.insert(provider.id, String::new());
    }
    SecretMap(map)
}

fn default_model_for_provider(provider_id: &str) -> String {
    if provider_id == APPLE_INTELLIGENCE_PROVIDER_ID {
        return APPLE_INTELLIGENCE_DEFAULT_MODEL_ID.to_string();
    }
    String::new()
}

fn default_post_process_models() -> HashMap<String, String> {
    let mut map = HashMap::new();
    for provider in default_post_process_providers() {
        map.insert(
            provider.id.clone(),
            default_model_for_provider(&provider.id),
        );
    }
    map
}

/// Default value helper for `#[serde(default = "default_true")]` fields.
fn default_true() -> bool {
    true
}

/// Default text-replacement rules. Empty by default — users add their own.
fn default_text_replacements() -> Vec<crate::settings::Replacement> {
    Vec::new()
}

fn default_post_process_prompts() -> Vec<LLMPrompt> {
    vec![LLMPrompt {
        id: "default_improve_transcriptions".to_string(),
        name: "Improve Transcriptions".to_string(),
        prompt: "Clean this transcript:\n1. Fix spelling, capitalization, and punctuation errors\n2. Convert number words to digits (twenty-five → 25, ten percent → 10%, five dollars → $5)\n3. Replace spoken punctuation with symbols (period → ., comma → ,, question mark → ?)\n4. Remove filler words (um, uh, like as filler)\n5. Keep the language in the original version (if it was french, keep it in french for example)\n\nPreserve exact meaning and word order. Do not paraphrase or reorder content.\n\nReturn only the cleaned transcript.\n\nTranscript:\n${output}".to_string(),
    }]
}

fn default_whisper_gpu_device() -> i32 {
    -1 // auto
}

fn default_typing_tool() -> TypingTool {
    TypingTool::Auto
}

fn default_assistant_provider_id() -> String {
    "custom".to_string()
}

/// Stable system prompt for the assistant. Keep this byte-identical across
/// requests — provider-side prompt caching keys off the exact prefix.
fn default_assistant_system_prompt() -> String {
    "You are a helpful voice assistant. The user talks to you by speaking; their speech is transcribed and sent to you, so expect occasional transcription errors and infer the intended meaning. Be concise and direct. Use plain text formatting suitable for a small chat panel. When a screenshot of the user's screen is attached, describe or use what you actually see in it.".to_string()
}

fn default_assistant_screenshot_enabled() -> bool {
    true
}

fn default_assistant_tts_voice() -> String {
    "af_heart".to_string()
}

fn default_assistant_tts_engine() -> String {
    "kokoro".to_string()
}

fn default_assistant_tts_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

/// Sensible default TTS base URL for a given engine. Used when the engine is
/// switched so a stale value (e.g. the OpenAI URL lingering under the Azure
/// engine and 404ing on Load voices) never leaks across engines.
pub fn default_tts_base_url_for_engine(engine: &str) -> String {
    match engine {
        "openai" => "https://api.openai.com/v1".to_string(),
        // Azure Speech / ElevenLabs / Kokoro don't reuse the OpenAI base URL; an
        // empty value shows the field's placeholder so the user enters the right
        // endpoint (or needs none, for ElevenLabs/Kokoro).
        _ => String::new(),
    }
}

/// Sensible default TTS model for a given engine.
pub fn default_tts_model_for_engine(engine: &str) -> String {
    match engine {
        "openai" => "gpt-4o-mini-tts".to_string(),
        "elevenlabs" => "eleven_flash_v2_5".to_string(),
        _ => String::new(),
    }
}

/// Sensible default remote voice for a given engine.
pub fn default_tts_remote_voice_for_engine(engine: &str) -> String {
    match engine {
        "openai" => "alloy".to_string(),
        // Azure falls back to en-US-JennyNeural when empty; ElevenLabs needs a
        // user-provided voice id.
        _ => String::new(),
    }
}

fn default_assistant_tts_model() -> String {
    "gpt-4o-mini-tts".to_string()
}

fn default_assistant_tts_remote_voice() -> String {
    "alloy".to_string()
}

fn default_assistant_tts_kokoro_dtype() -> String {
    // fp32 is recommended for WebGPU; users on weak/no GPU can pick a
    // quantized dtype (q8/q4/q4f16) for much faster CPU/WASM synthesis.
    "fp32".to_string()
}

fn default_assistant_tts_speed() -> f64 {
    // Normal speaking rate. The UI offers presets (0.5x–3x) and free entry;
    // values are clamped to a sane range when persisted.
    1.0
}

fn default_assistant_max_history_messages() -> u32 {
    // How many prior messages (user+assistant) the model sees as context.
    12
}

fn default_local_llm_context_size() -> u32 {
    // Mirrors LocalLlmManager's default; kept modest so memory stays reasonable
    // on the small models this feature targets.
    crate::managers::local_llm::DEFAULT_CONTEXT_SIZE
}

fn default_local_llm_unload_timeout() -> ModelUnloadTimeout {
    // Same default as the transcription model: unload after 5 minutes idle so
    // the built-in LLM frees RAM/VRAM when unused, while staying warm during
    // active use. Paired with prewarm-on-record, reloads stay mostly hidden.
    ModelUnloadTimeout::Min5
}

fn default_assistant_panel_size() -> String {
    "standard".to_string()
}

fn default_assistant_panel_theme() -> String {
    // Follow the app-wide theme by default; the user can override per-panel.
    "auto".to_string()
}

fn default_assistant_panel_opacity() -> f64 {
    1.0
}

fn default_assistant_font_size() -> String {
    "medium".to_string()
}

fn default_assistant_accent() -> String {
    "violet".to_string()
}

fn default_assistant_web_search_provider() -> String {
    // Serper is the default snippet backend: fast (~1–2 s) Google SERP results,
    // a generous free tier, and cheap at scale. Requires a (free) API key.
    "serper".to_string()
}

fn default_assistant_web_search_max_results() -> u32 {
    // A handful of full-content sources gives the model enough to synthesize a
    // solid answer without flooding the prompt.
    5
}

fn default_assistant_web_search_fetch_content() -> bool {
    // DEPRECATED / unused (web search is snippet-only). Default kept for
    // back-compat with existing settings files.
    true
}

fn default_assistant_web_search_daily_credit_budget() -> u32 {
    // DEPRECATED / unused (the Firecrawl credit guard was removed; search is
    // snippet-only). Default kept for back-compat with existing settings files.
    2000
}

fn default_web_search_api_keys() -> SecretMap {
    let mut map = HashMap::new();
    map.insert("serper".to_string(), String::new());
    map.insert("brave".to_string(), String::new());
    map.insert("tavily".to_string(), String::new());
    map.insert("exa".to_string(), String::new());
    map.insert("serpapi".to_string(), String::new());
    SecretMap(map)
}

fn ensure_assistant_defaults(settings: &mut AppSettings) -> bool {
    let mut changed = false;
    for provider in default_post_process_providers() {
        if !settings.assistant_models.contains_key(&provider.id) {
            settings
                .assistant_models
                .insert(provider.id.clone(), String::new());
            changed = true;
        }
    }
    if settings.assistant_system_prompt.trim().is_empty() {
        settings.assistant_system_prompt = default_assistant_system_prompt();
        changed = true;
    }
    if settings.assistant_tts_voice.trim().is_empty() {
        settings.assistant_tts_voice = default_assistant_tts_voice();
        changed = true;
    }
    if !matches!(
        settings.assistant_tts_engine.as_str(),
        "kokoro" | "openai" | "elevenlabs" | "azure"
    ) {
        settings.assistant_tts_engine = default_assistant_tts_engine();
        changed = true;
    }
    if settings.assistant_tts_base_url.trim().is_empty() {
        settings.assistant_tts_base_url = default_assistant_tts_base_url();
        changed = true;
    }
    if settings.assistant_tts_model.trim().is_empty() {
        settings.assistant_tts_model = default_assistant_tts_model();
        changed = true;
    }
    if settings.assistant_tts_remote_voice.trim().is_empty() {
        settings.assistant_tts_remote_voice = default_assistant_tts_remote_voice();
        changed = true;
    }
    if !matches!(
        settings.assistant_tts_kokoro_dtype.as_str(),
        "fp32" | "fp16" | "q8" | "q4" | "q4f16"
    ) {
        settings.assistant_tts_kokoro_dtype = default_assistant_tts_kokoro_dtype();
        changed = true;
    }
    // Keep conversation memory in a sane range (0 = no memory, 200 hard cap).
    if settings.assistant_max_history_messages > 200 {
        settings.assistant_max_history_messages = 200;
        changed = true;
    }
    if !matches!(
        settings.assistant_panel_size.as_str(),
        "compact" | "standard" | "large"
    ) {
        settings.assistant_panel_size = default_assistant_panel_size();
        changed = true;
    }
    if !matches!(
        settings.assistant_panel_theme.as_str(),
        "auto" | "light" | "dark"
    ) {
        settings.assistant_panel_theme = default_assistant_panel_theme();
        changed = true;
    }
    if !(0.5..=1.0).contains(&settings.assistant_panel_opacity) {
        settings.assistant_panel_opacity = default_assistant_panel_opacity();
        changed = true;
    }
    if !matches!(
        settings.assistant_font_size.as_str(),
        "small" | "medium" | "large"
    ) {
        settings.assistant_font_size = default_assistant_font_size();
        changed = true;
    }
    if settings.assistant_accent.trim().is_empty() {
        settings.assistant_accent = default_assistant_accent();
        changed = true;
    }
    // Web search: validate provider and backfill API-key slots for keyed
    // providers so the settings UI always has entries to bind to. Legacy values
    // (e.g. the removed "firecrawl"/"duckduckgo") fail this match and migrate to
    // the default (Serper).
    if !matches!(
        settings.assistant_web_search_provider.as_str(),
        "serper" | "brave" | "tavily" | "exa" | "serpapi"
    ) {
        settings.assistant_web_search_provider = default_assistant_web_search_provider();
        changed = true;
    }
    if settings.assistant_web_search_max_results == 0
        || settings.assistant_web_search_max_results > 10
    {
        settings.assistant_web_search_max_results = default_assistant_web_search_max_results();
        changed = true;
    }
    for provider_id in ["serper", "brave", "tavily", "exa", "serpapi"] {
        if !settings.web_search_api_keys.contains_key(provider_id) {
            settings
                .web_search_api_keys
                .insert(provider_id.to_string(), String::new());
            changed = true;
        }
    }
    changed
}

fn ensure_post_process_defaults(settings: &mut AppSettings) -> bool {
    let mut changed = false;
    for provider in default_post_process_providers() {
        // Use match to do a single lookup - either sync existing or add new
        match settings
            .post_process_providers
            .iter_mut()
            .find(|p| p.id == provider.id)
        {
            Some(existing) => {
                // Sync supports_structured_output field for existing providers (migration)
                if existing.supports_structured_output != provider.supports_structured_output {
                    debug!(
                        "Updating supports_structured_output for provider '{}' from {} to {}",
                        provider.id,
                        existing.supports_structured_output,
                        provider.supports_structured_output
                    );
                    existing.supports_structured_output = provider.supports_structured_output;
                    changed = true;
                }
            }
            None => {
                // Provider doesn't exist, add it
                settings.post_process_providers.push(provider.clone());
                changed = true;
            }
        }

        if !settings.post_process_api_keys.contains_key(&provider.id) {
            settings
                .post_process_api_keys
                .insert(provider.id.clone(), String::new());
            changed = true;
        }

        let default_model = default_model_for_provider(&provider.id);
        match settings.post_process_models.get_mut(&provider.id) {
            Some(existing) => {
                if existing.is_empty() && !default_model.is_empty() {
                    *existing = default_model.clone();
                    changed = true;
                }
            }
            None => {
                settings
                    .post_process_models
                    .insert(provider.id.clone(), default_model);
                changed = true;
            }
        }
    }

    changed
}

pub const SETTINGS_STORE_PATH: &str = "settings_store.json";

pub fn get_default_settings() -> AppSettings {
    #[cfg(target_os = "windows")]
    let default_shortcut = "ctrl+space";
    #[cfg(target_os = "macos")]
    let default_shortcut = "option+space";
    #[cfg(target_os = "linux")]
    let default_shortcut = "ctrl+space";
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let default_shortcut = "alt+space";

    let mut bindings = HashMap::new();
    bindings.insert(
        "transcribe".to_string(),
        ShortcutBinding {
            id: "transcribe".to_string(),
            name: "Transcribe".to_string(),
            description: "Press to start recording, press again to stop and type it out."
                .to_string(),
            default_binding: default_shortcut.to_string(),
            current_binding: default_shortcut.to_string(),
        },
    );
    #[cfg(target_os = "windows")]
    let default_post_process_shortcut = "ctrl+shift+space";
    #[cfg(target_os = "macos")]
    let default_post_process_shortcut = "option+shift+space";
    #[cfg(target_os = "linux")]
    let default_post_process_shortcut = "ctrl+shift+space";
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let default_post_process_shortcut = "alt+shift+space";

    bindings.insert(
        "transcribe_with_post_process".to_string(),
        ShortcutBinding {
            id: "transcribe_with_post_process".to_string(),
            name: "Transcribe with Post-Processing".to_string(),
            description: "Converts your speech into text and applies AI post-processing."
                .to_string(),
            default_binding: default_post_process_shortcut.to_string(),
            current_binding: default_post_process_shortcut.to_string(),
        },
    );
    bindings.insert(
        "cancel".to_string(),
        ShortcutBinding {
            id: "cancel".to_string(),
            name: "Cancel".to_string(),
            description: "Cancels the current recording.".to_string(),
            default_binding: "escape".to_string(),
            current_binding: "escape".to_string(),
        },
    );

    #[cfg(target_os = "macos")]
    let default_assistant_shortcut = "option+ctrl+space";
    #[cfg(not(target_os = "macos"))]
    let default_assistant_shortcut = "ctrl+alt+space";

    bindings.insert(
        "assistant".to_string(),
        ShortcutBinding {
            id: "assistant".to_string(),
            name: "Assistant".to_string(),
            description:
                "Ask the AI assistant by voice; the answer appears in the assistant panel."
                    .to_string(),
            default_binding: default_assistant_shortcut.to_string(),
            current_binding: default_assistant_shortcut.to_string(),
        },
    );

    // Note: there's intentionally no dedicated "Assistant + Screen" shortcut.
    // Ctrl/Cmd+Alt+Shift+Space is reserved as the assistant's hands-free (lock)
    // variant. Attach a screenshot from the assistant panel's camera button
    // instead; a dedicated screen shortcut may return later on a free combo.

    #[cfg(target_os = "macos")]
    let default_panel_toggle_shortcut = "option+ctrl+a";
    #[cfg(not(target_os = "macos"))]
    let default_panel_toggle_shortcut = "ctrl+alt+a";

    bindings.insert(
        "assistant_panel_toggle".to_string(),
        ShortcutBinding {
            id: "assistant_panel_toggle".to_string(),
            name: "Toggle Assistant Panel".to_string(),
            description: "Shows or hides the floating assistant panel.".to_string(),
            default_binding: default_panel_toggle_shortcut.to_string(),
            current_binding: default_panel_toggle_shortcut.to_string(),
        },
    );

    AppSettings {
        bindings,
        push_to_talk: true,
        audio_feedback: false,
        audio_feedback_volume: default_audio_feedback_volume(),
        sound_theme: default_sound_theme(),
        start_hidden: default_start_hidden(),
        autostart_enabled: default_autostart_enabled(),
        update_checks_enabled: default_update_checks_enabled(),
        selected_model: "".to_string(),
        always_on_microphone: false,
        selected_microphone: None,
        clamshell_microphone: None,
        selected_output_device: None,
        translate_to_english: false,
        selected_language: "auto".to_string(),
        overlay_position: default_overlay_position(),
        debug_mode: false,
        log_level: default_log_level(),
        custom_words: Vec::new(),
        replacements_enabled: false,
        text_replacements: default_text_replacements(),
        model_unload_timeout: ModelUnloadTimeout::default(),
        local_llm_unload_timeout: default_local_llm_unload_timeout(),
        word_correction_threshold: default_word_correction_threshold(),
        history_limit: default_history_limit(),
        recording_retention_period: default_recording_retention_period(),
        paste_method: PasteMethod::default(),
        clipboard_handling: ClipboardHandling::default(),
        auto_submit: default_auto_submit(),
        auto_submit_key: AutoSubmitKey::default(),
        post_process_enabled: default_post_process_enabled(),
        post_process_provider_id: default_post_process_provider_id(),
        post_process_providers: default_post_process_providers(),
        post_process_api_keys: default_post_process_api_keys(),
        post_process_models: default_post_process_models(),
        post_process_prompts: default_post_process_prompts(),
        post_process_selected_prompt_id: None,
        mute_while_recording: false,
        append_trailing_space: false,
        app_language: default_app_language(),
        experimental_enabled: false,
        lazy_stream_close: false,
        keyboard_implementation: KeyboardImplementation::default(),
        show_tray_icon: default_show_tray_icon(),
        paste_delay_ms: default_paste_delay_ms(),
        typing_tool: default_typing_tool(),
        external_script_path: None,
        custom_filler_words: None,
        whisper_accelerator: WhisperAcceleratorSetting::default(),
        ort_accelerator: OrtAcceleratorSetting::default(),
        whisper_gpu_device: default_whisper_gpu_device(),
        extra_recording_buffer_ms: 0,
        assistant_provider_id: default_assistant_provider_id(),
        assistant_models: {
            let mut map = HashMap::new();
            for provider in default_post_process_providers() {
                map.insert(provider.id, String::new());
            }
            map
        },
        assistant_system_prompt: default_assistant_system_prompt(),
        assistant_screenshot_enabled: default_assistant_screenshot_enabled(),
        assistant_tts_enabled: false,
        assistant_tts_engine: default_assistant_tts_engine(),
        assistant_tts_voice: default_assistant_tts_voice(),
        assistant_tts_base_url: default_assistant_tts_base_url(),
        assistant_tts_api_key: SecretString::default(),
        assistant_tts_model: default_assistant_tts_model(),
        assistant_tts_remote_voice: default_assistant_tts_remote_voice(),
        assistant_tts_kokoro_dtype: default_assistant_tts_kokoro_dtype(),
        assistant_tts_speed: default_assistant_tts_speed(),
        assistant_max_history_messages: default_assistant_max_history_messages(),
        local_llm_context_size: default_local_llm_context_size(),
        assistant_response_length: AssistantResponseLength::default(),
        assistant_panel_opacity: default_assistant_panel_opacity(),
        assistant_font_size: default_assistant_font_size(),
        assistant_accent: default_assistant_accent(),
        assistant_panel_size: default_assistant_panel_size(),
        assistant_panel_theme: default_assistant_panel_theme(),
        assistant_web_search_enabled: false,
        assistant_web_search_provider: default_assistant_web_search_provider(),
        assistant_web_search_max_results: default_assistant_web_search_max_results(),
        assistant_web_search_fetch_content: default_assistant_web_search_fetch_content(),
        assistant_search_depth: AssistantSearchDepth::default(),
        assistant_web_search_daily_credit_budget: default_assistant_web_search_daily_credit_budget(
        ),
        assistant_local_search_smart: false,
        web_search_api_keys: default_web_search_api_keys(),
        theme: Theme::System,
    }
}

impl AppSettings {
    pub fn active_post_process_provider(&self) -> Option<&PostProcessProvider> {
        self.post_process_providers
            .iter()
            .find(|provider| provider.id == self.post_process_provider_id)
    }

    pub fn active_assistant_provider(&self) -> Option<&PostProcessProvider> {
        self.post_process_providers
            .iter()
            .find(|provider| provider.id == self.assistant_provider_id)
    }

    pub fn post_process_provider(&self, provider_id: &str) -> Option<&PostProcessProvider> {
        self.post_process_providers
            .iter()
            .find(|provider| provider.id == provider_id)
    }

    pub fn post_process_provider_mut(
        &mut self,
        provider_id: &str,
    ) -> Option<&mut PostProcessProvider> {
        self.post_process_providers
            .iter_mut()
            .find(|provider| provider.id == provider_id)
    }
}

// ---------------------------------------------------------------------------
// Secret handling (OS keychain)
//
// API keys live in the OS keychain (see `crate::secret_store`), not in
// `settings_store.json`. The flow:
//   * `write_settings` mirrors the in-memory secrets into the keychain, then
//     blanks them before the struct is written to disk.
//   * `get_settings` re-fills the in-memory secrets from the keychain so every
//     existing read site keeps working unchanged.
//   * `load_or_create_app_settings` performs a one-time migration of any
//     pre-existing plaintext keys out of the store and into the keychain.
// When the keychain is unavailable (e.g. headless Linux), secrets simply stay
// in the store and the app behaves exactly as before (a warning is logged once
// by `secret_store`).
// ---------------------------------------------------------------------------

/// Persist the in-memory secrets into the OS keychain and blank each one in the
/// struct **only after** the keychain confirms it holds the value. A failed
/// keychain write therefore leaves the key in the on-disk store as a fallback
/// rather than losing it. An empty value removes any stored credential, so
/// clearing a key in the UI actually clears it (and it won't be re-hydrated).
///
/// Input must be hydrated (the live values), which is always the case for
/// `write_settings` since every caller goes through `get_settings` first.
fn persist_hydrated_secrets(settings: &mut AppSettings) {
    let provider_ids: Vec<String> = settings.post_process_api_keys.keys().cloned().collect();
    for id in provider_ids {
        let value = settings
            .post_process_api_keys
            .get(&id)
            .cloned()
            .unwrap_or_default();
        if crate::secret_store::sync(&crate::secret_store::account_post_process(&id), &value) {
            if let Some(slot) = settings.post_process_api_keys.get_mut(&id) {
                slot.clear();
            }
        }
    }
    let provider_ids: Vec<String> = settings.web_search_api_keys.keys().cloned().collect();
    for id in provider_ids {
        let value = settings
            .web_search_api_keys
            .get(&id)
            .cloned()
            .unwrap_or_default();
        if crate::secret_store::sync(&crate::secret_store::account_web_search(&id), &value) {
            if let Some(slot) = settings.web_search_api_keys.get_mut(&id) {
                slot.clear();
            }
        }
    }
    if crate::secret_store::sync(
        crate::secret_store::ACCOUNT_ASSISTANT_TTS,
        &settings.assistant_tts_api_key.0,
    ) {
        settings.assistant_tts_api_key = SecretString::default();
    }
}

/// One-time migration of legacy plaintext keys from the store into the keychain.
///
/// Only touches NON-EMPTY values and only blanks a value once the keychain
/// confirms it was stored. This is deliberately different from
/// `persist_hydrated_secrets`: its input comes straight from disk (not yet
/// hydrated), so an empty slot means "already migrated / never set", NOT "the
/// user cleared this key". It must therefore never delete a keychain entry based
/// on an empty on-disk value — otherwise a restart would wipe the keychain.
/// Returns true if anything was moved (so the caller can persist the stripped
/// store).
fn migrate_plaintext_secrets(settings: &mut AppSettings) -> bool {
    let mut changed = false;
    let provider_ids: Vec<String> = settings.post_process_api_keys.keys().cloned().collect();
    for id in provider_ids {
        let value = settings
            .post_process_api_keys
            .get(&id)
            .cloned()
            .unwrap_or_default();
        if !value.is_empty()
            && crate::secret_store::set(&crate::secret_store::account_post_process(&id), &value)
        {
            if let Some(slot) = settings.post_process_api_keys.get_mut(&id) {
                slot.clear();
            }
            changed = true;
        }
    }
    let provider_ids: Vec<String> = settings.web_search_api_keys.keys().cloned().collect();
    for id in provider_ids {
        let value = settings
            .web_search_api_keys
            .get(&id)
            .cloned()
            .unwrap_or_default();
        if !value.is_empty()
            && crate::secret_store::set(&crate::secret_store::account_web_search(&id), &value)
        {
            if let Some(slot) = settings.web_search_api_keys.get_mut(&id) {
                slot.clear();
            }
            changed = true;
        }
    }
    if !settings.assistant_tts_api_key.0.is_empty()
        && crate::secret_store::set(
            crate::secret_store::ACCOUNT_ASSISTANT_TTS,
            &settings.assistant_tts_api_key.0,
        )
    {
        settings.assistant_tts_api_key = SecretString::default();
        changed = true;
    }
    changed
}

/// Re-fill the in-memory secret fields from the OS keychain. No-op when the
/// keychain is unavailable, which leaves any plaintext fallback values from the
/// store in place.
fn hydrate_secrets(settings: &mut AppSettings) {
    if !crate::secret_store::is_available() {
        return;
    }
    for (provider_id, value) in settings.post_process_api_keys.iter_mut() {
        if let Some(secret) =
            crate::secret_store::get(&crate::secret_store::account_post_process(provider_id))
        {
            *value = secret;
        }
    }
    for (provider_id, value) in settings.web_search_api_keys.iter_mut() {
        if let Some(secret) =
            crate::secret_store::get(&crate::secret_store::account_web_search(provider_id))
        {
            *value = secret;
        }
    }
    if let Some(secret) = crate::secret_store::get(crate::secret_store::ACCOUNT_ASSISTANT_TTS) {
        settings.assistant_tts_api_key = SecretString(secret);
    }
}

pub fn load_or_create_app_settings(app: &AppHandle) -> AppSettings {
    // Initialize store
    let store = app
        .store(crate::portable::store_path(SETTINGS_STORE_PATH))
        .expect("Failed to initialize store");

    let mut settings = if let Some(settings_value) = store.get("settings") {
        // Parse the entire settings object
        match serde_json::from_value::<AppSettings>(settings_value) {
            Ok(mut settings) => {
                debug!("Found existing settings: {:?}", settings);
                let default_settings = get_default_settings();
                let mut updated = false;

                // Merge default bindings into existing settings
                for (key, value) in default_settings.bindings {
                    if !settings.bindings.contains_key(&key) {
                        debug!("Adding missing binding: {}", key);
                        settings.bindings.insert(key, value);
                        updated = true;
                    }
                }

                // Drop obsolete bindings from older settings files so they stop
                // being registered:
                //  - transcribe_toggle: the main shortcuts now lock hands-free
                //    via their Shift variant, so the standalone toggle is gone.
                //  - assistant_vision: Ctrl/Cmd+Alt+Shift+Space is now the
                //    assistant's hands-free variant; screenshots come from the
                //    panel's camera button instead.
                for obsolete in ["transcribe_toggle", "assistant_vision"] {
                    if settings.bindings.remove(obsolete).is_some() {
                        debug!("Removing obsolete '{}' binding", obsolete);
                        updated = true;
                    }
                }

                if updated {
                    debug!("Settings updated with new bindings");
                    store.set("settings", serde_json::to_value(&settings).unwrap());
                }

                settings
            }
            Err(e) => {
                warn!("Failed to parse settings: {}", e);
                // Fall back to default settings if parsing fails
                let default_settings = get_default_settings();
                store.set("settings", serde_json::to_value(&default_settings).unwrap());
                default_settings
            }
        }
    } else {
        let default_settings = get_default_settings();
        store.set("settings", serde_json::to_value(&default_settings).unwrap());
        default_settings
    };

    if ensure_post_process_defaults(&mut settings) | ensure_assistant_defaults(&mut settings) {
        store.set("settings", serde_json::to_value(&settings).unwrap());
    }

    // One-time migration: move any plaintext keys that pre-date keychain storage
    // into the OS keychain. Only keys the keychain confirms it stored are
    // stripped from the JSON; the rest stay on disk (fallback). This never
    // deletes a keychain entry based on an already-stripped value, so restarts
    // are safe. Persist only if something actually moved.
    if crate::secret_store::is_available() && migrate_plaintext_secrets(&mut settings) {
        store.set("settings", serde_json::to_value(&settings).unwrap());
    }

    // Fill the in-memory secrets from the keychain so callers see real keys.
    hydrate_secrets(&mut settings);

    settings
}

pub fn get_settings(app: &AppHandle) -> AppSettings {
    let store = app
        .store(crate::portable::store_path(SETTINGS_STORE_PATH))
        .expect("Failed to initialize store");

    let mut settings = if let Some(settings_value) = store.get("settings") {
        serde_json::from_value::<AppSettings>(settings_value).unwrap_or_else(|_| {
            let default_settings = get_default_settings();
            store.set("settings", serde_json::to_value(&default_settings).unwrap());
            default_settings
        })
    } else {
        let default_settings = get_default_settings();
        store.set("settings", serde_json::to_value(&default_settings).unwrap());
        default_settings
    };

    if ensure_post_process_defaults(&mut settings) | ensure_assistant_defaults(&mut settings) {
        store.set("settings", serde_json::to_value(&settings).unwrap());
    }

    // Fill the in-memory secrets from the keychain (served from cache after the
    // first read, so this stays cheap on the hot path). No-op — leaving any
    // plaintext fallback in place — when the keychain is unavailable.
    hydrate_secrets(&mut settings);

    settings
}

pub fn write_settings(app: &AppHandle, mut settings: AppSettings) {
    let store = app
        .store(crate::portable::store_path(SETTINGS_STORE_PATH))
        .expect("Failed to initialize store");

    // Keep API keys in the OS keychain, never in the on-disk store. Each key is
    // blanked from the serialized copy only after the keychain confirms it holds
    // it; a failed keychain write leaves the key on disk (fallback) rather than
    // losing it. When the keychain is unavailable, secrets stay on disk as
    // before.
    if crate::secret_store::is_available() {
        persist_hydrated_secrets(&mut settings);
    }

    store.set("settings", serde_json::to_value(&settings).unwrap());
}

pub fn get_bindings(app: &AppHandle) -> HashMap<String, ShortcutBinding> {
    let settings = get_settings(app);

    settings.bindings
}

pub fn get_stored_binding(app: &AppHandle, id: &str) -> ShortcutBinding {
    let bindings = get_bindings(app);

    let binding = bindings.get(id).unwrap().clone();

    binding
}

pub fn get_history_limit(app: &AppHandle) -> usize {
    let settings = get_settings(app);
    settings.history_limit
}

pub fn get_recording_retention_period(app: &AppHandle) -> RecordingRetentionPeriod {
    let settings = get_settings(app);
    settings.recording_retention_period
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_disable_auto_submit() {
        let settings = get_default_settings();
        assert!(!settings.auto_submit);
        assert_eq!(settings.auto_submit_key, AutoSubmitKey::Enter);
    }

    #[test]
    fn debug_output_redacts_api_keys() {
        let mut settings = get_default_settings();
        settings
            .post_process_api_keys
            .insert("openai".to_string(), "sk-proj-secret-key-12345".to_string());
        settings.post_process_api_keys.insert(
            "anthropic".to_string(),
            "sk-ant-secret-key-67890".to_string(),
        );
        settings
            .post_process_api_keys
            .insert("empty_provider".to_string(), "".to_string());

        let debug_output = format!("{:?}", settings);

        assert!(!debug_output.contains("sk-proj-secret-key-12345"));
        assert!(!debug_output.contains("sk-ant-secret-key-67890"));
        assert!(debug_output.contains("[REDACTED]"));
    }

    #[test]
    fn secret_map_debug_redacts_values() {
        let map = SecretMap(HashMap::from([("key".into(), "secret".into())]));
        let out = format!("{:?}", map);
        assert!(!out.contains("secret"));
        assert!(out.contains("[REDACTED]"));
    }
}
