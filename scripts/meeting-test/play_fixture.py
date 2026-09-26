#!/usr/bin/env python3
"""Play the fixture out of your speakers so the app records it as a real call.

    python play_fixture.py                       system track to the default output
    python play_fixture.py --devices             list output devices
    python play_fixture.py --mic-device 7        also inject your own lines

The idea this whole file rests on: **WASAPI loopback does not know where the
audio came from.** `audio_toolkit::audio::loopback` captures the default render
endpoint, so a Zoom call, a YouTube video and this script are the same bytes to
it. There is no such thing as needing real participants to produce system audio
-- you only ever needed audio leaving your speakers.

So the live test is:

    1. bun run tauri dev
    2. start a meeting in the app
    3. run this script
    4. read the YOU lines aloud when they are cued (or --mic-device them in)
    5. stop the meeting
    6. python score_meeting.py --id <the new meeting>

That exercises everything the seeder cannot: the loopback endpoint, the
chunker's silence handling, the transcription worker's queue, the two level
meters, the pill, and the WAV writers.

Determinism. Reading your lines aloud makes the mic track different every run,
which is fine for a smoke test and useless for comparing two runs. For a
repeatable mic track, install a virtual audio cable (VB-CABLE is the usual
free one), set the app's input device to its Output, and pass its Input as
--mic-device. Then both tracks are byte-identical every run.

Two things to check before starting, both of which produce a half-empty
recording that looks like a bug in the app:
  * your output device must be the one you are playing to -- headphones plugged
    in mid-test moves the endpoint;
  * the app's input device must not be a loopback/"Stereo Mix" source, or both
    tracks capture the same audio and the me/them split collapses.
"""

from __future__ import annotations

import argparse
import atexit
import json
import shutil
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path


def die(message: str) -> "NoReturn":  # type: ignore[valid-type]
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


def load_fixture(name: str) -> tuple[Path, dict]:
    fixture = Path(name)
    if not fixture.is_absolute():
        fixture = Path(__file__).parent / fixture
    truth_path = fixture / "truth.json"
    if not truth_path.is_file():
        die(f"{truth_path} not found; run make_fixture.py first")
    return fixture, json.loads(truth_path.read_text(encoding="utf-8"))


def list_devices() -> None:
    try:
        import sounddevice  # type: ignore
    except ImportError:
        die("--devices needs `pip install sounddevice`")
    print("output devices (use the index with --mic-device):\n")
    for index, device in enumerate(sounddevice.query_devices()):
        if device["max_output_channels"] > 0:
            default = " [default]" if index == sounddevice.default.device[1] else ""
            print(f"  {index:>3}  {device['name']}{default}")
    print(
        "\nPick the *input* side of a virtual cable (e.g. 'CABLE Input (VB-Audio)').\n"
        "Then set SpeakoFlow's microphone to the matching 'CABLE Output'."
    )


def play_default_device(path: Path, muted: bool = False):
    """Start the system track on the default output device, in-process.

    `winsound` rather than a PowerShell SoundPlayer subprocess, for one reason
    that was learned the hard way: a child process keeps playing after the parent
    is force-killed. `atexit` and signal handlers cannot help -- TerminateProcess
    runs no cleanup -- so the audio outlives the script that explains it, and you
    are left hunting a voice with no window. Audio owned by this process stops
    when this process does.

    Returns a callable that stops playback, or None where winsound is absent.
    """
    if muted:
        return lambda: None

    if sys.platform == "win32":
        import winsound

        winsound.PlaySound(str(path), winsound.SND_FILENAME | winsound.SND_ASYNC)
        return lambda: winsound.PlaySound(None, winsound.SND_PURGE)

    player = "afplay" if sys.platform == "darwin" else "aplay"
    if not shutil.which(player):
        die(f"{player} not found; install it or use --mic-device with sounddevice")
    process = subprocess.Popen(
        [player, str(path)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    return process.kill


def play_on_device(path: Path, device: int) -> threading.Thread:
    import sounddevice  # type: ignore
    import soundfile  # type: ignore

    samples, rate = soundfile.read(str(path), dtype="float32")

    def run() -> None:
        sounddevice.play(samples, rate, device=device, blocking=True)

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    return thread


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--fixture", default="fixture")
    parser.add_argument("--devices", action="store_true", help="list output devices and exit")
    parser.add_argument("--mic-device", type=int, default=None,
                        help="also play the mic track to this output index (needs sounddevice)")
    parser.add_argument("--countdown", type=int, default=5)
    parser.add_argument("--no-audio", action="store_true",
                        help="run the cue timeline silently; for checking this script, "
                             "not for testing the app")
    args = parser.parse_args()

    # Cues are useless if they arrive in one block at the end, which is what
    # Python does by default whenever stdout is not a terminal.
    try:
        sys.stdout.reconfigure(line_buffering=True)
    except AttributeError:
        pass

    if args.devices:
        list_devices()
        return

    fixture, truth = load_fixture(args.fixture)
    system_wav = fixture / "system_play.wav"
    mic_wav = fixture / "mic_play.wav"
    if not system_wav.is_file():
        die(f"{system_wav} not found; run make_fixture.py first")

    duration = truth["duration_ms"] / 1000
    print(f"{truth['title']}  --  {duration:.0f}s, "
          f"{len(truth['remote_speakers'])} remote speakers + you")
    if args.no_audio:
        print("--no-audio: NOTHING will be played. Cues only.")
    else:
        print(f"playing: {system_wav.name} -> default output device (loopback captures this)")
        if args.mic_device is not None:
            print(f"         {mic_wav.name} -> device {args.mic_device} (your microphone side)")
        else:
            mine = sum(1 for u in truth["utterances"] if u["source"] == "mic")
            print(f"         your {mine} lines are cued below with >>> -- read them aloud")
    print("\nStart the meeting in SpeakoFlow now.")
    for remaining in range(args.countdown, 0, -1):
        print(f"  starting in {remaining}...", end="\r", flush=True)
        time.sleep(1)
    print("  playing            ")

    started = time.monotonic()
    stop_system = play_default_device(system_wav, muted=args.no_audio)
    mic_thread = (
        play_on_device(mic_wav, args.mic_device)
        if args.mic_device is not None and not args.no_audio
        else None
    )

    def silence() -> None:
        stop_system()
        if args.mic_device is not None:
            try:
                import sounddevice  # type: ignore

                sounddevice.stop()
            except Exception:
                pass

    atexit.register(silence)
    signal.signal(signal.SIGINT, lambda *_: sys.exit("\nstopped"))

    try:
        # Cue on the same clock playback started on, so a line you read lands
        # inside the span truth.json claims it occupies and score_meeting.py can
        # align it.
        for utterance in truth["utterances"]:
            wait = utterance["start_ms"] / 1000 - (time.monotonic() - started)
            if wait > 0:
                time.sleep(wait)
            stamp = (
                f"{int(utterance['start_ms'] / 60000):02d}:"
                f"{int(utterance['start_ms'] / 1000) % 60:02d}"
            )
            if utterance["source"] == "mic":
                print(f"\n[{stamp}] >>> YOU: {utterance['text']}\n")
            else:
                print(f"[{stamp}]     {utterance['display']}: {utterance['text'][:78]}")

        remaining = duration - (time.monotonic() - started)
        if remaining > 0:
            time.sleep(remaining)
        if mic_thread:
            mic_thread.join(timeout=10)
    finally:
        silence()

    print("\ndone. Stop the meeting in SpeakoFlow, then:")
    print("  python seed_meeting.py --list          find the new meeting id")
    print("  python score_meeting.py --id <id>      grade it")
    print("\nOn a recorded run, expect a non-zero WER (that is the transcription\n"
          "path being measured) and a timing drift of a second or two (the chunker\n"
          "closes on silence, not on your script's boundaries).")


if __name__ == "__main__":
    main()
