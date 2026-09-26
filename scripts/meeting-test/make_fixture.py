#!/usr/bin/env python3
"""Synthesize a scripted multi-speaker call into the two tracks a meeting has.

    python make_fixture.py [--script script.json] [--out fixture] [--engine sapi|edge]

Output, in `--out`:

    system.wav       16 kHz mono f32  - the remote participants ("them")
    mic.wav          16 kHz mono f32  - the local user ("me")
    system_play.wav  16 kHz mono s16  - same audio, playable by anything
    mic_play.wav     16 kHz mono s16
    truth.json       every utterance with its speaker, source and exact ms span
    cues.txt         a read-along script, for when you speak your own lines

The f32 format is not cosmetic: `meetings::session::open_wav` writes 16 kHz mono
32-bit float, and `diarize::read_segment` reads `samples::<f32>()` with a comment
saying an i16 file "would return silent garbage from this file". A fixture in the
wrong format fails in a way that looks like a diarization bug.

Two TTS engines:

  sapi  (default) Windows SAPI via PowerShell. Offline, nothing to install, but
        this machine has only two voices, so extra speakers are made by pitch
        shifting. Good enough to prove the pipeline splits N voices and holds
        the labels stable; not a fidelity benchmark.
  edge  Microsoft Edge neural voices via `pip install edge-tts`. Needs network.
        Genuinely different speakers, so an accuracy number from it means
        something.

Neither is a substitute for one recording of real humans. Record a real call
once, keep it forever, and use these for the fast iteration loop in between.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

import numpy as np

RATE = 16_000
# Silence between turns. Above `chunker::DEFAULT_SILENCE_HOLD_MS` (1500) so each
# turn closes its own chunk instead of being glued to the next speaker's.
DEFAULT_GAP_MS = 1_700
# A tiny noise floor. Digital silence is not something a microphone ever
# produces, and VAD tuned on real rooms should be fed something realistic.
DEFAULT_NOISE_DBFS = -62.0


def die(message: str) -> "NoReturn":  # type: ignore[valid-type]
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


def ffmpeg() -> str:
    found = shutil.which("ffmpeg")
    if not found:
        die("ffmpeg is not on PATH; it converts and pitch-shifts the synthesized audio")
    return found


def run(cmd: list[str]) -> None:
    done = subprocess.run(cmd, capture_output=True, text=True)
    if done.returncode != 0:
        tail = (done.stderr or "").strip().splitlines()[-8:]
        die(f"{Path(cmd[0]).name} failed:\n" + "\n".join(tail))


# ────────────────────────────── synthesis ──────────────────────────────


def synth_sapi(turns: list[dict], speakers: dict, work: Path) -> list[Path]:
    """Synthesize every turn in one PowerShell process.

    One process rather than one per line: SAPI initialisation dominates, and a
    15-turn script drops from about half a minute to a couple of seconds.
    """
    jobs = []
    for index, turn in enumerate(turns):
        voice = speakers[turn["speaker"]].get("sapi")
        if not voice:
            die(f"speaker '{turn['speaker']}' has no `sapi` voice in the script")
        jobs.append(
            {
                "path": str(work / f"raw_{index:03d}.wav"),
                "voice": voice,
                "rate": int(speakers[turn["speaker"]].get("rate", 0)),
                "text": turn["text"],
            }
        )

    jobs_file = work / "jobs.json"
    jobs_file.write_text(json.dumps(jobs), encoding="utf-8")

    script = work / "synth.ps1"
    script.write_text(
        r"""
param([string]$JobsFile)
Add-Type -AssemblyName System.Speech
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$installed = $synth.GetInstalledVoices() | ForEach-Object { $_.VoiceInfo.Name }
$jobs = Get-Content -Raw -LiteralPath $JobsFile | ConvertFrom-Json
foreach ($job in $jobs) {
    if ($installed -notcontains $job.voice) {
        Write-Error "voice not installed: $($job.voice). Installed: $($installed -join ', ')"
        exit 1
    }
    $synth.SelectVoice($job.voice)
    $synth.Rate = $job.rate
    $synth.SetOutputToWaveFile($job.path)
    $synth.Speak($job.text)
}
$synth.SetOutputToNull()
$synth.Dispose()
""".strip(),
        encoding="utf-8",
    )

    run(
        [
            "powershell",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            str(script),
            "-JobsFile",
            str(jobs_file),
        ]
    )
    return [Path(job["path"]) for job in jobs]


def synth_edge(turns: list[dict], speakers: dict, work: Path) -> list[Path]:
    try:
        import edge_tts  # type: ignore
    except ImportError:
        die("--engine edge needs `pip install edge-tts` (it also needs network access)")

    async def speak_all() -> None:
        for index, turn in enumerate(turns):
            voice = speakers[turn["speaker"]].get("edge")
            if not voice:
                die(f"speaker '{turn['speaker']}' has no `edge` voice in the script")
            out = work / f"raw_{index:03d}.mp3"
            await edge_tts.Communicate(turn["text"], voice).save(str(out))

    asyncio.run(speak_all())
    return [work / f"raw_{i:03d}.mp3" for i in range(len(turns))]


# ─────────────────────────── audio plumbing ───────────────────────────


def to_mono_16k(src: Path, dst: Path, pitch: float) -> None:
    """Decode to 16 kHz mono, optionally shifting pitch while keeping tempo.

    `asetrate` resamples without resampling the timeline, which shifts pitch and
    duration together; `atempo` then puts the duration back. The result changes
    timbre enough that a speaker-embedding model treats it as another person,
    which is the whole trick that turns two installed voices into four speakers.
    """
    if abs(pitch - 1.0) < 1e-3:
        chain = "aresample=16000"
    else:
        chain = (
            f"asetrate={int(RATE * pitch)},aresample={RATE},"
            f"atempo={1.0 / pitch:.6f}"
        )
    run(
        [
            ffmpeg(), "-y", "-loglevel", "error",
            "-i", str(src),
            "-ac", "1", "-ar", str(RATE),
            "-af", chain,
            "-c:a", "pcm_s16le",
            str(dst),
        ]
    )


def load_s16(path: Path) -> np.ndarray:
    with wave.open(str(path), "rb") as handle:
        if handle.getnchannels() != 1 or handle.getframerate() != RATE:
            die(f"{path.name} is not 16 kHz mono after conversion")
        frames = handle.readframes(handle.getnframes())
    return np.frombuffer(frames, dtype="<i2").astype(np.float32) / 32768.0


def trim_silence(samples: np.ndarray, threshold: float = 3e-3) -> np.ndarray:
    """Trim leading/trailing near-silence.

    SAPI pads its output, and that padding would land inside the span truth.json
    claims is speech. Diarization measures voiced audio per segment, so padding
    that is counted as speech but is not is exactly the thing that pushes a
    segment under `min_voiced_for_label_ms` and leaves it unlabelled.
    """
    loud = np.flatnonzero(np.abs(samples) > threshold)
    if loud.size == 0:
        return samples
    pad = int(0.05 * RATE)
    start = max(0, int(loud[0]) - pad)
    end = min(samples.size, int(loud[-1]) + pad)
    return samples[start:end]


def write_wav(path: Path, samples: np.ndarray, float32: bool) -> None:
    """Write via ffmpeg from raw PCM rather than hand-rolling a RIFF header.

    hound is strict about the float WAV layout, and a header this script wrote
    itself would be one more thing that could be wrong when a test fails.
    """
    raw = path.with_suffix(".raw")
    raw.write_bytes(np.clip(samples, -1.0, 1.0).astype("<f4").tobytes())
    run(
        [
            ffmpeg(), "-y", "-loglevel", "error",
            "-f", "f32le", "-ar", str(RATE), "-ac", "1",
            "-i", str(raw),
            "-c:a", "pcm_f32le" if float32 else "pcm_s16le",
            str(path),
        ]
    )
    raw.unlink(missing_ok=True)


# ──────────────────────────────── build ────────────────────────────────


def build(args: argparse.Namespace) -> None:
    script_path = Path(args.script)
    if not script_path.is_absolute():
        script_path = Path(__file__).parent / script_path
    spec = json.loads(script_path.read_text(encoding="utf-8"))

    speakers: dict = spec["speakers"]
    turns: list[dict] = spec["turns"]
    for turn in turns:
        if turn["speaker"] not in speakers:
            die(f"turn references unknown speaker '{turn['speaker']}'")

    out = Path(args.out)
    if not out.is_absolute():
        out = Path(__file__).parent / out
    out.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="mtgfix_") as tmp:
        work = Path(tmp)
        print(f"synthesizing {len(turns)} turns with {args.engine}...")
        raws = (synth_sapi if args.engine == "sapi" else synth_edge)(turns, speakers, work)

        clips: list[np.ndarray] = []
        for index, (raw, turn) in enumerate(zip(raws, turns)):
            converted = work / f"cv_{index:03d}.wav"
            to_mono_16k(raw, converted, float(speakers[turn["speaker"]].get("pitch", 1.0)))
            clips.append(trim_silence(load_s16(converted)))

    # Lay the turns out on one shared timeline. Both tracks span the whole
    # meeting so mic.wav and system.wav have identical length, which is what the
    # recorder produces and what keeps truth.json's offsets valid for both.
    placements: list[tuple[int, int, dict]] = []
    cursor = 0
    for turn, clip in zip(turns, clips):
        gap = int(turn.get("gap_ms", args.gap_ms))
        start = max(0, cursor + gap)
        placements.append((start, start + clip.size, turn))
        cursor = start + clip.size
    total = cursor + int(0.8 * RATE)

    amplitude = 10.0 ** (args.noise_dbfs / 20.0)
    rng = np.random.default_rng(args.seed)
    tracks = {
        "mic": rng.normal(0.0, amplitude, total).astype(np.float32),
        "system": rng.normal(0.0, amplitude, total).astype(np.float32),
    }

    truth = []
    for (start, end, turn), clip in zip(placements, clips):
        source = speakers[turn["speaker"]].get("source", "system")
        if source not in tracks:
            die(f"speaker '{turn['speaker']}' has source '{source}', expected mic or system")
        tracks[source][start:end] += clip
        truth.append(
            {
                "speaker": turn["speaker"],
                "display": speakers[turn["speaker"]].get("display", turn["speaker"]),
                "source": source,
                "start_ms": int(round(start * 1000 / RATE)),
                "end_ms": int(round(end * 1000 / RATE)),
                "text": turn["text"],
            }
        )

    write_wav(out / "system.wav", tracks["system"], float32=True)
    write_wav(out / "mic.wav", tracks["mic"], float32=True)
    write_wav(out / "system_play.wav", tracks["system"], float32=False)
    write_wav(out / "mic_play.wav", tracks["mic"], float32=False)

    duration_ms = int(round(total * 1000 / RATE))
    remote = sorted({u["speaker"] for u in truth if u["source"] == "system"})
    (out / "truth.json").write_text(
        json.dumps(
            {
                "title": spec.get("title", "Test meeting"),
                "engine": args.engine,
                "rate": RATE,
                "duration_ms": duration_ms,
                "remote_speakers": remote,
                "speakers": {
                    key: {
                        "display": value.get("display", key),
                        "source": value.get("source", "system"),
                    }
                    for key, value in speakers.items()
                },
                "utterances": truth,
            },
            indent=2,
        ),
        encoding="utf-8",
    )

    lines = [
        f"{spec.get('title', 'Test meeting')} - read-along cues",
        "Only the YOU lines need speaking; the rest plays out of your speakers.",
        "",
    ]
    for utterance in truth:
        stamp = f"{utterance['start_ms'] // 60000:02d}:{utterance['start_ms'] // 1000 % 60:02d}"
        who = "YOU" if utterance["source"] == "mic" else utterance["display"].upper()
        lines.append(f"[{stamp}] {who}: {utterance['text']}")
    (out / "cues.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")

    shortest = min(u["end_ms"] - u["start_ms"] for u in truth)
    print(f"\nwrote {out}")
    print(f"  {duration_ms / 1000:.1f}s, {len(truth)} utterances")
    print(f"  remote speakers: {', '.join(remote)}  (mic: you)")
    print(f"  shortest utterance: {shortest} ms")
    if shortest < 2_000:
        print(
            "  warning: under 2s. DiarizeConfig::window_ms is 1500, so short\n"
            "           utterances yield no embedding window and stay unlabelled."
        )
    print("\nnext: python seed_meeting.py        (test diarization and notes)")
    print("      python play_fixture.py        (test live capture)")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--script", default="script.json")
    parser.add_argument("--out", default="fixture")
    parser.add_argument("--engine", choices=("sapi", "edge"), default="sapi")
    parser.add_argument("--gap-ms", type=int, default=DEFAULT_GAP_MS,
                        help="default silence between turns (a turn can override with gap_ms)")
    parser.add_argument("--noise-dbfs", type=float, default=DEFAULT_NOISE_DBFS)
    parser.add_argument("--seed", type=int, default=7, help="noise seed, so runs are byte-identical")
    build(parser.parse_args())


if __name__ == "__main__":
    main()
