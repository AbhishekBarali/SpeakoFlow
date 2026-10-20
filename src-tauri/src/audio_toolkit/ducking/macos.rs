//! macOS: the output volume through `osascript`, as the old mute did. It is
//! the volume the menu bar slider shows, it needs no permission, and an output
//! without a volume control (HDMI, some USB interfaces) reads as `missing
//! value`, so it is left alone instead of guessed at.
//!
//! `osascript` only reaches the current default output, so this is the one
//! platform where the device is not pinned. The restore still never turns up
//! a device that was swapped in mid-recording: its volume is not the one the
//! worker left, so it counts as the user's.

// Built in tests on every platform, where only the parser is used.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::{OutputVolume, Reading};
use std::process::{Command, Stdio};

struct SystemOutput;

pub(super) fn default_output() -> Option<Box<dyn OutputVolume>> {
    // Nothing to resolve: whether this output can be lowered shows in its
    // first reading.
    Some(Box::new(SystemOutput))
}

impl OutputVolume for SystemOutput {
    fn read(&self) -> Reading {
        osascript("get volume settings")
            .map(|out| parse_volume_settings(&out))
            .unwrap_or_default()
    }

    fn set_level(&self, level: f32) -> bool {
        let percent = (level.clamp(0.0, 1.0) * 100.0).round() as u32;
        osascript(&format!("set volume output volume {percent}")).is_some()
    }

    fn set_muted(&self, muted: bool) -> bool {
        osascript(&format!("set volume output muted {muted}")).is_some()
    }
}

/// Stdout of a script that succeeded.
fn osascript(script: &str) -> Option<String> {
    let output = Command::new("osascript")
        .args(["-e", script])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
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
