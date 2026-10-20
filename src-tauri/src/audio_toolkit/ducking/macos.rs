//! macOS: the output volume through `osascript`, as the old mute did. It is
//! the volume the menu bar slider shows, it needs no permission, and an output
//! without a volume control (HDMI, some USB interfaces) reads as `missing
//! value`, so it is left alone instead of guessed at.
//!
//! `osascript` only reaches the current default output, so the device is
//! pinned another way: CoreAudio's default output device id is read when the
//! output is opened, and while the default is a different device every read
//! is empty and every change is refused. Headphones plugged in mid-recording
//! are not turned up, and the speakers are put back once they are the default
//! again. If the id cannot be read, nothing is lowered.

// Built in tests on every platform, where only the parser is used.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::{output_within, OutputVolume, Reading, TOOL_DEADLINE};
use std::process::{Command, Stdio};

struct SystemOutput {
    /// The default output device when this was opened.
    device: u32,
}

pub(super) fn default_output() -> Option<Box<dyn OutputVolume>> {
    // An output that cannot be pinned is not lowered at all. Whether a pinned
    // one can be lowered shows in its first reading.
    let device = default_device()?;
    Some(Box::new(SystemOutput { device }))
}

impl SystemOutput {
    /// Whether `osascript` still reaches the device this was opened on.
    fn is_pinned_device(&self) -> bool {
        default_device() == Some(self.device)
    }
}

impl OutputVolume for SystemOutput {
    fn read(&self) -> Reading {
        if !self.is_pinned_device() {
            return Reading::default();
        }
        osascript("get volume settings")
            .map(|out| parse_volume_settings(&out))
            .unwrap_or_default()
    }

    fn set_level(&self, level: f32) -> bool {
        if !self.is_pinned_device() {
            return false;
        }
        let percent = (level.clamp(0.0, 1.0) * 100.0).round() as u32;
        osascript(&format!("set volume output volume {percent}")).is_some()
    }

    fn set_muted(&self, muted: bool) -> bool {
        if !self.is_pinned_device() {
            return false;
        }
        osascript(&format!("set volume output muted {muted}")).is_some()
    }
}

/// CoreAudio's default output device id.
#[cfg(target_os = "macos")]
fn default_device() -> Option<u32> {
    use objc2_core_audio::{
        kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyElementMain,
        kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioObjectUnknown,
        AudioObjectGetPropertyData, AudioObjectID, AudioObjectPropertyAddress,
    };
    use std::ffi::c_void;
    use std::ptr::{self, NonNull};

    let address = AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyDefaultOutputDevice,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut device: AudioObjectID = kAudioObjectUnknown;
    let mut size = std::mem::size_of::<AudioObjectID>() as u32;
    // The address, the size and the buffer outlive the call, and the buffer
    // is `size` bytes, as the call requires.
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&address),
            0,
            ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut device).cast::<c_void>(),
        )
    };
    (status == 0 && device != kAudioObjectUnknown).then_some(device)
}

#[cfg(not(target_os = "macos"))]
fn default_device() -> Option<u32> {
    None
}

/// Stdout of a script that succeeded within [`TOOL_DEADLINE`].
fn osascript(script: &str) -> Option<String> {
    let output = output_within(
        Command::new("osascript")
            .args(["-e", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
        TOOL_DEADLINE,
    )?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `output volume:44, input volume:75, alert volume:100, output muted:false`.
/// Either output field is `missing value` on a device without that control.
fn parse_volume_settings(out: &str) -> Reading {
    let mut reading = Reading::default();
    for field in out.trim().split(',') {
        let Some((key, value)) = field.split_once(':') else {
            continue;
        };
        match key.trim() {
            "output volume" => {
                reading.level = value
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .map(|percent| percent / 100.0);
            }
            "output muted" => {
                reading.muted = match value.trim() {
                    "true" => Some(true),
                    "false" => Some(false),
                    _ => None,
                };
            }
            _ => {}
        }
    }
    reading
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_volume_settings() {
        assert_eq!(
            parse_volume_settings(
                "output volume:44, input volume:75, alert volume:100, output muted:false\n"
            ),
            Reading {
                level: Some(0.44),
                muted: Some(false)
            }
        );
        assert_eq!(
            parse_volume_settings(
                "output volume:0, input volume:50, alert volume:100, output muted:true\n"
            ),
            Reading {
                level: Some(0.0),
                muted: Some(true)
            }
        );
    }

    #[test]
    fn an_output_without_a_volume_control_reads_as_nothing() {
        assert_eq!(
            parse_volume_settings(
                "output volume:missing value, input volume:missing value, alert volume:100, output muted:missing value\n"
            ),
            Reading::default()
        );
    }
}
