//! Windows Speech API (SAPI 5) synthesis.
//!
//! SAPI uses the voice selected in Windows' speech settings, including voices
//! installed by the system or a SAPI adapter. Audio is rendered to a temporary
//! WAV file so the existing native playback path can handle cancellation,
//! output-device selection, and volume consistently with other TTS engines.

#![cfg(windows)]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::PCWSTR;
use windows::Win32::Media::Speech::{
    ISpStream, ISpVoice, SpFileStream, SpVoice, SPFM_CREATE_ALWAYS, SPF_IS_NOT_XML,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};

/// Synthesize text with the user's default Windows SAPI voice.
pub fn synthesize(text: &str, speed: f64) -> Result<Vec<u8>, String> {
    if text.trim().is_empty() {
        return Err("Windows SAPI cannot synthesize empty text".to_string());
    }

    let path = temporary_wav_path();
    let result = synthesize_to_file(text, speed, &path);
    let audio = result.and_then(|()| {
        fs::read(&path).map_err(|e| format!("Failed to read Windows SAPI audio: {}", e))
    });
    let _ = fs::remove_file(&path);
    audio
}

fn synthesize_to_file(text: &str, speed: f64, path: &PathBuf) -> Result<(), String> {
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    initialized
        .ok()
        .map_err(|e| format!("Could not initialize Windows Speech API COM: {}", e))?;

    struct ComGuard;
    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    let _com_guard = ComGuard;

    let path_wide = wide_null(&path.to_string_lossy());
    let text_wide = wide_null(text);

    let result = (|| unsafe {
        let stream: ISpStream = CoCreateInstance(&SpFileStream, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("Could not create Windows SAPI file stream: {}", e))?;
        stream
            .BindToFile(
                PCWSTR(path_wide.as_ptr()),
                SPFM_CREATE_ALWAYS,
                None,
                None,
                0,
            )
            .map_err(|e| format!("Could not open Windows SAPI audio stream: {}", e))?;

        let voice: ISpVoice = CoCreateInstance(&SpVoice, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("Could not create Windows SAPI voice: {}", e))?;
        voice
            .SetOutput(&stream, true)
            .map_err(|e| format!("Could not configure Windows SAPI output: {}", e))?;
        voice
            .SetRate(sapi_rate(speed))
            .map_err(|e| format!("Could not configure Windows SAPI speaking rate: {}", e))?;
        voice
            .Speak(PCWSTR(text_wide.as_ptr()), SPF_IS_NOT_XML.0 as u32, None)
            .map_err(|e| format!("Windows SAPI synthesis failed: {}", e))?;
        stream
            .Close()
            .map_err(|e| format!("Could not close Windows SAPI audio stream: {}", e))?;

        // SAPI finishes synchronous Speak before returning. Drop the COM
        // objects before reading so the file handle is closed and all bytes
        // are flushed.
        drop(voice);
        drop(stream);
        Ok(())
    })();

    result
}

/// Map the app's 0.25x–4x multiplier to SAPI's -10…10 rate adjustment.
fn sapi_rate(speed: f64) -> i32 {
    (((speed.clamp(0.25, 4.0) - 1.0) * 10.0).round() as i32).clamp(-10, 10)
}

fn temporary_wav_path() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "speakoflow-sapi-{}-{}.wav",
        std::process::id(),
        stamp
    ))
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::sapi_rate;

    #[test]
    fn speed_maps_to_sapi_rate_range() {
        assert_eq!(sapi_rate(0.25), -8);
        assert_eq!(sapi_rate(1.0), 0);
        assert_eq!(sapi_rate(2.0), 10);
        assert_eq!(sapi_rate(4.0), 10);
    }
}
