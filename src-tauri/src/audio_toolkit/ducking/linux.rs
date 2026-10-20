//! Linux: the default sink, through the first sound tool that can read it.
//!
//! 1. `pactl` (PulseAudio, and PipeWire through `pipewire-pulse`), version 15
//!    or later, which has `get-default-sink` and `get-sink-volume`. It reads
//!    and writes every channel, so a sink whose left and right are set apart
//!    keeps that balance.
//! 2. `wpctl` (PipeWire without the Pulse layer). Its volume is one value
//!    written to every channel, so it cannot keep a balance.
//! 3. ALSA's `amixer`, on the default card's `Master` control.
//!
//! The sink is resolved to a fixed name or node when it is lowered, so the
//! recording restores the device it lowered even if the default moves. Every
//! tool runs in the C locale: `pactl` translates its output, and a decimal
//! comma would make `wpctl` read and write the wrong volume. Inside an
//! AppImage every tool also runs without the bundle's library directories,
//! which the system's own tools are not built against.

// Built in tests on every platform, where only the parsers are used.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::{OutputVolume, Reading};
use std::process::{Command, Stdio};

/// PulseAudio's 100%, in the raw volume `pactl` prints and accepts.
const PA_VOLUME_NORM: f32 = 65536.0;

enum Tool {
    /// A PulseAudio sink name.
    Pactl(String),
    /// A PipeWire node id, or `@DEFAULT_AUDIO_SINK@` if it could not be
    /// resolved.
    Wpctl(String),
    /// The default card's `Master` control.
    Amixer,
}

pub(super) fn default_output() -> Option<Box<dyn OutputVolume>> {
    let candidates: [fn() -> Option<Tool>; 3] = [
        // Without `get-default-sink` (before version 15) there is no name to
        // pin, and `wpctl` is tried instead.
        || {
            run("pactl", &["get-default-sink"])
                .map(|out| out.trim().to_string())
                .filter(|name| !name.is_empty())
                .map(Tool::Pactl)
        },
        || {
            let node = run("wpctl", &["inspect", "@DEFAULT_AUDIO_SINK@"])
                .and_then(|out| parse_wpctl_node_id(&out))
                .unwrap_or_else(|| "@DEFAULT_AUDIO_SINK@".to_string());
            Some(Tool::Wpctl(node))
        },
        || Some(Tool::Amixer),
    ];
    // The first tool that can read a volume is the one used.
    candidates
        .into_iter()
        .filter_map(|resolve| resolve())
        .find(|tool| tool.read().level.is_some())
        .map(|tool| Box::new(tool) as Box<dyn OutputVolume>)
}

impl OutputVolume for Tool {
    fn read(&self) -> Reading {
        match self {
            Tool::Pactl(sink) => Reading {
                level: loudest(&pactl_channels(sink)),
                muted: run("pactl", &["get-sink-mute", sink])
                    .and_then(|out| parse_pactl_mute(&out)),
            },
            Tool::Wpctl(node) => run("wpctl", &["get-volume", node])
                .map(|out| parse_wpctl_volume(&out))
                .unwrap_or_default(),
            Tool::Amixer => run("amixer", &["-M", "get", "Master"])
                .map(|out| parse_amixer(&out))
                .unwrap_or_default(),
        }
    }

    fn set_level(&self, level: f32) -> bool {
        let level = level.max(0.0);
        match self {
            // Every channel is set from the channels as they are now, so the
            // balance between them needs nothing stored.
            Tool::Pactl(sink) => {
                let volumes = pactl_volumes(&pactl_channels(sink), level);
                let mut args = vec!["set-sink-volume", sink.as_str()];
                args.extend(volumes.iter().map(String::as_str));
                succeeds("pactl", &args)
            }
            // One value, written to every channel.
            Tool::Wpctl(node) => succeeds("wpctl", &["set-volume", node, &format!("{level:.3}")]),
            Tool::Amixer => {
                let channels = run("amixer", &["-M", "get", "Master"])
                    .map(|out| parse_amixer_channels(&out))
                    .unwrap_or_default();
                succeeds(
                    "amixer",
                    &[
                        "-M",
                        "-q",
                        "set",
                        "Master",
                        &amixer_volume(&channels, level),
                    ],
                )
            }
        }
    }

    fn set_muted(&self, muted: bool) -> bool {
        let flag = if muted { "1" } else { "0" };
        match self {
            Tool::Pactl(sink) => succeeds("pactl", &["set-sink-mute", sink, flag]),
            Tool::Wpctl(node) => succeeds("wpctl", &["set-mute", node, flag]),
            Tool::Amixer => succeeds(
                "amixer",
                &["-q", "set", "Master", if muted { "mute" } else { "unmute" }],
            ),
        }
    }
}

/// The sink's channels as fractions, empty when they cannot be read.
fn pactl_channels(sink: &str) -> Vec<f32> {
    run("pactl", &["get-sink-volume", sink])
        .map(|out| parse_pactl_channels(&out))
        .unwrap_or_default()
}

/// The loudest channel, which is what the volume reads as.
fn loudest(channels: &[f32]) -> Option<f32> {
    channels.iter().copied().reduce(f32::max)
}

/// `level` for the loudest channel, and every other channel moved in
/// proportion, so the balance between them stays as it is. Just `level`, for
/// every channel at once, when there is no reading or every channel is at 0.
fn scaled(channels: &[f32], level: f32) -> Vec<f32> {
    match loudest(channels) {
        Some(loudest) if loudest > 0.0 => channels
            .iter()
            .map(|channel| channel * level / loudest)
            .collect(),
        _ => vec![level],
    }
}

/// `pactl set-sink-volume`'s arguments, one per channel, in raw volume rather
/// than whole percent: rounding to 1% at every step of a fade would shift a
/// balance a little each time.
fn pactl_volumes(channels: &[f32], level: f32) -> Vec<String> {
    scaled(channels, level)
        .into_iter()
        .map(|volume| ((volume * PA_VOLUME_NORM).round() as u32).to_string())
        .collect()
}

/// `amixer set`'s volume: `40%,60%`, one per channel in the order `amixer get`
/// lists them, or a single `40%` for every channel.
fn amixer_volume(channels: &[f32], level: f32) -> String {
    scaled(channels, level)
        .into_iter()
        .map(whole_percent)
        .collect::<Vec<_>>()
        .join(",")
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
    #[cfg(target_os = "linux")]
    crate::helpers::appimage::scrub_appimage_library_path(&mut command);
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

/// `pactl get-sink-volume`'s first line, as one fraction per channel:
/// `Volume: front-left: 26214 /  40% / -23.88 dB,   front-right: ...`. The
/// raw volume before the first `/` is read, as the exact value.
fn parse_pactl_channels(out: &str) -> Vec<f32> {
    out.lines()
        .next()
        .unwrap_or_default()
        .split(',')
        .map_while(|channel| {
            channel
                .split('/')
                .next()?
                .split_whitespace()
                .last()?
                .parse::<u32>()
                .ok()
        })
        .map(|raw| raw as f32 / PA_VOLUME_NORM)
        .collect()
}

/// `Mute: yes` or `Mute: no`.
fn parse_pactl_mute(out: &str) -> Option<bool> {
    match out.trim().strip_prefix("Mute:")?.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// `amixer get Master`'s playback channels, one `[40%]` per line, as
/// fractions.
fn parse_amixer_channels(out: &str) -> Vec<f32> {
    out.lines()
        .filter(|line| line.contains("Playback"))
        .filter_map(|line| {
            line.split(['[', ']'])
                .find_map(|token| token.strip_suffix('%')?.parse::<f32>().ok())
        })
        .map(|percent| percent / 100.0)
        .collect()
}

/// `amixer get Master`: a `[40%]` and an `[on]` or `[off]` per channel. A
/// control without a switch has neither, and cannot be muted.
fn parse_amixer(out: &str) -> Reading {
    Reading {
        level: loudest(&parse_amixer_channels(out)),
        muted: Some(out.contains("[off]")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.001)
    }

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
    fn reads_every_pactl_channel() {
        let balanced = "Volume: front-left: 26214 /  40% / -23.88 dB,   front-right: 26214 /  40% / -23.88 dB\n        balance 0.00\n";
        assert!(close(&parse_pactl_channels(balanced), &[0.4, 0.4]));

        let apart = "Volume: front-left: 26214 /  40% / -23.88 dB,   front-right: 39321 /  60% / -13.31 dB\n        balance 0.33\n";
        let channels = parse_pactl_channels(apart);
        assert!(close(&channels, &[0.4, 0.6]), "{channels:?}");
        assert!(close(&[loudest(&channels).unwrap()], &[0.6]));

        let mono = "Volume: mono: 52429 /  80% / -5.81 dB\n        balance 0.00\n";
        assert!(close(&parse_pactl_channels(mono), &[0.8]));

        let surround = "Volume: front-left: 32768 /  50% / -18.06 dB,   front-right: 32768 /  50% / -18.06 dB,   rear-left: 19661 /  30% / -31.37 dB,   rear-right: 19661 /  30% / -31.37 dB,   front-center: 65536 / 100% / 0.00 dB,   lfe: 45875 /  70% / -9.29 dB\n        balance 0.00\n";
        let channels = parse_pactl_channels(surround);
        assert!(
            close(&channels, &[0.5, 0.5, 0.3, 0.3, 1.0, 0.7]),
            "{channels:?}"
        );
        assert_eq!(loudest(&channels), Some(1.0));

        assert!(parse_pactl_channels("Failed to get sink volume: No such entity\n").is_empty());
        assert!(parse_pactl_channels("").is_empty());
    }

    #[test]
    fn reads_pactl_mute() {
        assert_eq!(parse_pactl_mute("Mute: yes\n"), Some(true));
        assert_eq!(parse_pactl_mute("Mute: no\n"), Some(false));
        assert_eq!(parse_pactl_mute("Stumm: ja\n"), None);
    }

    #[test]
    fn a_new_level_moves_every_channel_in_proportion() {
        assert!(close(&scaled(&[0.4, 0.6], 0.3), &[0.2, 0.3]));
        assert!(close(&scaled(&[0.8], 0.4), &[0.4]));
        assert!(close(&scaled(&[], 0.5), &[0.5]));
        assert!(close(&scaled(&[0.0, 0.0], 0.5), &[0.5]));
        assert_eq!(
            pactl_volumes(&[26214.0 / PA_VOLUME_NORM, 39321.0 / PA_VOLUME_NORM], 0.3),
            ["13107", "19661"]
        );
        assert_eq!(pactl_volumes(&[], 1.0), ["65536"]);
    }

    #[test]
    fn a_fade_down_and_back_up_keeps_the_balance() {
        // Left at 40%, right at 60%, faded to a tenth and back the way the
        // worker does it: each step reads the channels and writes them again.
        let mut raw = vec!["26214".to_string(), "39321".to_string()];
        let levels = (0..=30)
            .map(|step| 0.6 - 0.54 * step as f32 / 30.0)
            .chain((0..=30).map(|step| 0.06 + 0.54 * step as f32 / 30.0));
        for level in levels {
            let channels: Vec<f32> = raw
                .iter()
                .map(|volume| volume.parse::<f32>().unwrap() / PA_VOLUME_NORM)
                .collect();
            raw = pactl_volumes(&channels, level);
        }
        // Within a tenth of a percent of where each channel started. Whole
        // percent at every step would have brought the left back at 34%.
        let back: Vec<i64> = raw.iter().map(|volume| volume.parse().unwrap()).collect();
        assert!((back[0] - 26214).abs() < 66, "{back:?}");
        assert!((back[1] - 39321).abs() < 66, "{back:?}");
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
        assert!(close(&parse_amixer_channels(stereo), &[0.8, 0.8]));
        let apart =
            "  Front Left: Playback 26214 [40%] [on]\n  Front Right: Playback 39321 [60%] [on]\n";
        assert!(close(&parse_amixer_channels(apart), &[0.4, 0.6]));
        assert_eq!(parse_amixer(apart).level, Some(0.6));
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
    fn writes_amixer_volume_per_channel() {
        assert_eq!(amixer_volume(&[0.4, 0.6], 0.3), "20%,30%");
        assert_eq!(amixer_volume(&[0.81], 0.4), "40%");
        assert_eq!(amixer_volume(&[], 0.304), "30%");
    }

    #[test]
    fn writes_whole_percent() {
        assert_eq!(whole_percent(0.304), "30%");
        assert_eq!(whole_percent(0.0), "0%");
        assert_eq!(whole_percent(1.2), "120%");
    }
}
