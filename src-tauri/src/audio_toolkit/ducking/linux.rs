//! Linux: the default sink, through whichever sound tool is installed. The
//! same three, in the same order, as the old mute: `wpctl` (PipeWire), then
//! `pactl` (PulseAudio, and PipeWire's Pulse layer), then ALSA's `amixer`.
//!
//! The sink is resolved to a fixed node or name when it is lowered, so the
//! recording restores the device it lowered even if the default moves. Every
//! tool runs in the C locale: `pactl` translates its output, and a decimal
//! comma would make `wpctl` read and write the wrong volume.

// Built in tests on every platform, where only the parsers are used.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::{OutputVolume, Reading};
use std::process::{Command, Stdio};

enum Tool {
    /// A PipeWire node id, or `@DEFAULT_AUDIO_SINK@` if it could not be
    /// resolved.
    Wpctl(String),
    /// A PulseAudio sink name, or `@DEFAULT_SINK@`.
    Pactl(String),
    /// The default card's `Master` control.
    Amixer,
}

pub(super) fn default_output() -> Option<Box<dyn OutputVolume>> {
    let candidates: [fn() -> Tool; 3] = [
        || {
            let node = run("wpctl", &["inspect", "@DEFAULT_AUDIO_SINK@"])
                .and_then(|out| parse_wpctl_node_id(&out))
                .unwrap_or_else(|| "@DEFAULT_AUDIO_SINK@".to_string());
            Tool::Wpctl(node)
        },
        || {
            let sink = run("pactl", &["get-default-sink"])
                .map(|out| out.trim().to_string())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "@DEFAULT_SINK@".to_string());
            Tool::Pactl(sink)
        },
        || Tool::Amixer,
    ];
    // The first tool that can read a volume is the one used.
    candidates
        .into_iter()
        .map(|resolve| resolve())
        .find(|tool| tool.read().level.is_some())
        .map(|tool| Box::new(tool) as Box<dyn OutputVolume>)
}

impl OutputVolume for Tool {
    fn read(&self) -> Reading {
        match self {
            Tool::Wpctl(node) => run("wpctl", &["get-volume", node])
                .map(|out| parse_wpctl_volume(&out))
                .unwrap_or_default(),
            Tool::Pactl(sink) => Reading {
                level: run("pactl", &["get-sink-volume", sink]).and_then(|out| parse_percent(&out)),
                muted: run("pactl", &["get-sink-mute", sink])
                    .and_then(|out| parse_pactl_mute(&out)),
            },
            Tool::Amixer => run("amixer", &["-M", "get", "Master"])
                .map(|out| parse_amixer(&out))
                .unwrap_or_default(),
        }
    }

    fn set_level(&self, level: f32) -> bool {
        let level = level.max(0.0);
        match self {
            Tool::Wpctl(node) => succeeds("wpctl", &["set-volume", node, &format!("{level:.3}")]),
            Tool::Pactl(sink) => {
                succeeds("pactl", &["set-sink-volume", sink, &whole_percent(level)])
            }
            Tool::Amixer => succeeds(
                "amixer",
                &["-M", "-q", "set", "Master", &whole_percent(level)],
            ),
        }
    }

    fn set_muted(&self, muted: bool) -> bool {
        let flag = if muted { "1" } else { "0" };
        match self {
            Tool::Wpctl(node) => succeeds("wpctl", &["set-mute", node, flag]),
            Tool::Pactl(sink) => succeeds("pactl", &["set-sink-mute", sink, flag]),
            Tool::Amixer => succeeds(
                "amixer",
                &["-q", "set", "Master", if muted { "mute" } else { "unmute" }],
            ),
        }
    }
}

fn whole_percent(level: f32) -> String {
    format!("{}%", (level * 100.0).round() as u32)
}

/// Stdout of a tool that succeeded.
fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = command(program, args)
        .stdout(Stdio::piped())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn succeeds(program: &str, args: &[&str]) -> bool {
    command(program, args)
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn command(program: &str, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// `wpctl inspect`'s first line: `id 52, type PipeWire:Interface:Node`.
fn parse_wpctl_node_id(out: &str) -> Option<String> {
    let id = out
        .lines()
        .next()?
        .trim()
        .strip_prefix("id ")?
        .split(',')
        .next()?
        .trim();
    id.parse::<u32>().ok().map(|_| id.to_string())
}

/// `Volume: 0.40`, or `Volume: 0.40 [MUTED]`.
fn parse_wpctl_volume(out: &str) -> Reading {
    let Some(rest) = out.trim().strip_prefix("Volume:") else {
        return Reading::default();
    };
    Reading {
        level: rest
            .split_whitespace()
            .next()
            .and_then(|level| level.parse::<f32>().ok()),
        muted: Some(rest.contains("[MUTED]")),
    }
}

/// The first percentage in the output, as a fraction: `pactl`'s
/// `front-left: 26214 /  40% / -23.88 dB, ...` or `amixer`'s `[40%]`.
fn parse_percent(out: &str) -> Option<f32> {
    out.split(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | '/' | ','))
        .find_map(|token| token.strip_suffix('%')?.parse::<f32>().ok())
        .map(|percent| percent / 100.0)
}

/// `Mute: yes` or `Mute: no`.
fn parse_pactl_mute(out: &str) -> Option<bool> {
    match out.trim().strip_prefix("Mute:")?.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// `amixer get Master`: a `[40%]` and an `[on]` or `[off]` per channel. A
/// control without a switch has neither, and cannot be muted.
fn parse_amixer(out: &str) -> Reading {
    Reading {
        level: parse_percent(out),
        muted: Some(out.contains("[off]")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_wpctl() {
        assert_eq!(
            parse_wpctl_volume("Volume: 0.40\n"),
            Reading {
                level: Some(0.4),
                muted: Some(false)
            }
        );
        assert_eq!(
            parse_wpctl_volume("Volume: 1.25 [MUTED]\n"),
            Reading {
                level: Some(1.25),
                muted: Some(true)
            }
        );
        assert_eq!(
            parse_wpctl_volume("Error: no such node"),
            Reading::default()
        );
    }

    #[test]
    fn reads_the_wpctl_node_id() {
        let out = "id 52, type PipeWire:Interface:Node\n    alsa.card = \"0\"\n";
        assert_eq!(parse_wpctl_node_id(out).as_deref(), Some("52"));
        assert_eq!(parse_wpctl_node_id("Object not found\n"), None);
        assert_eq!(parse_wpctl_node_id("id x, type Node"), None);
    }

    #[test]
    fn reads_pactl() {
        let out = "Volume: front-left: 26214 /  40% / -23.88 dB,   front-right: 26214 /  40% / -23.88 dB\n        balance 0.00\n";
        assert_eq!(parse_percent(out), Some(0.4));
        assert_eq!(parse_pactl_mute("Mute: yes\n"), Some(true));
        assert_eq!(parse_pactl_mute("Mute: no\n"), Some(false));
        assert_eq!(parse_pactl_mute("Stumm: ja\n"), None);
    }

    #[test]
    fn reads_amixer() {
        let stereo = "Simple mixer control 'Master',0\n  Capabilities: pvolume pswitch\n  Playback channels: Front Left - Front Right\n  Limits: Playback 0 - 65536\n  Mono:\n  Front Left: Playback 52428 [80%] [on]\n  Front Right: Playback 52428 [80%] [on]\n";
        assert_eq!(
            parse_amixer(stereo),
            Reading {
                level: Some(0.8),
                muted: Some(false)
            }
        );
        let muted = "  Mono: Playback 52 [81%] [-12.75dB] [off]\n";
        assert_eq!(
            parse_amixer(muted),
            Reading {
                level: Some(0.81),
                muted: Some(true)
            }
        );
    }

    #[test]
    fn writes_whole_percent() {
        assert_eq!(whole_percent(0.304), "30%");
        assert_eq!(whole_percent(0.0), "0%");
        assert_eq!(whole_percent(1.2), "120%");
    }
}
