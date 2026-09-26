#!/usr/bin/env python3
"""Inject a generated fixture into meetings.db as a finished, un-diarized meeting.

    python seed_meeting.py                  seed a new [TEST] meeting
    python seed_meeting.py --reset           un-diarize the newest [TEST] meeting
    python seed_meeting.py --list            show what is in the database
    python seed_meeting.py --cleanup         delete every [TEST] meeting

Why this exists. Recording a meeting takes as long as the meeting, and the parts
most likely to be wrong -- diarization, notes, ask-about-this-meeting -- all run
*after* capture, on the stored WAV and the stored rows. Seeding gives you those
inputs in under a second, so the iteration loop on the hard half of the feature
is:

    edit diarize/cluster.rs  ->  bun run tauri dev  ->  Identify speakers
    ->  python score_meeting.py  ->  python seed_meeting.py --reset  ->  repeat

The seeded rows carry the ground-truth transcript text, so transcription is held
constant and any change in the score is attributable to the pass you edited.

Safety. This only ever INSERTs, and only titles prefixed "[TEST]". The database
is copied to meetings.db.bak-<timestamp> before any write, and --cleanup refuses
to touch a meeting whose title lacks the prefix.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sqlite3
import sys
import time
from pathlib import Path

TEST_PREFIX = "[TEST]"
IDENTIFIER = "com.abhishekbarali.speakoflow"
SPEAKER_MODEL = "wespeaker_en_voxceleb_resnet34_LM.onnx"
# `diarize::model::MIN_PLAUSIBLE_BYTES` -- a shorter file is a truncated download
# that reports "not installed" rather than failing to load.
MIN_MODEL_BYTES = 20 * 1024 * 1024


def die(message: str) -> "NoReturn":  # type: ignore[valid-type]
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


def app_data_dir(explicit: str | None) -> Path:
    """Locate the app data directory, honouring portable installs.

    Mirrors `portable::app_data_dir`: a `portable` marker next to the executable
    redirects everything to `Data/` beside it, so a dev running a portable build
    would otherwise have this script seed a database the app never opens.
    """
    if explicit:
        path = Path(explicit)
        if not path.is_dir():
            die(f"--app-data {path} is not a directory")
        return path

    appdata = os.environ.get("APPDATA")
    if not appdata:
        die("APPDATA is not set; pass --app-data explicitly")
    path = Path(appdata) / IDENTIFIER
    if not path.is_dir():
        die(
            f"{path} does not exist.\n"
            "       Run the app once so it creates its data directory, or pass\n"
            "       --app-data <dir> (a portable install keeps it in Data/ next to the exe)."
        )
    return path


def connect(db_path: Path) -> sqlite3.Connection:
    if not db_path.is_file():
        die(f"{db_path} does not exist; open the Meetings section once to create it")
    conn = sqlite3.connect(db_path)
    conn.execute("PRAGMA foreign_keys = ON")
    return conn


def backup(db_path: Path) -> Path:
    target = db_path.with_suffix(f".db.bak-{time.strftime('%Y%m%d-%H%M%S')}")
    shutil.copy2(db_path, target)
    return target


def report_model(data_dir: Path) -> None:
    model = data_dir / "models" / SPEAKER_MODEL
    size = model.stat().st_size if model.is_file() else 0
    if size >= MIN_MODEL_BYTES:
        print(f"  speaker model: installed ({size / 1e6:.0f} MB)")
    else:
        print(
            "  speaker model: NOT installed -- diarization will return\n"
            "                 Skipped{ModelNotInstalled} and change nothing.\n"
            "                 Settings > Meetings > Identify speakers downloads it (27 MB)."
        )


# ───────────────────────────────── seed ─────────────────────────────────


def seed(args: argparse.Namespace) -> None:
    fixture = Path(args.fixture)
    if not fixture.is_absolute():
        fixture = Path(__file__).parent / fixture
    truth_path = fixture / "truth.json"
    if not truth_path.is_file():
        die(f"{truth_path} not found; run make_fixture.py first")
    truth = json.loads(truth_path.read_text(encoding="utf-8"))

    for name in ("system.wav", "mic.wav"):
        if not (fixture / name).is_file():
            die(f"{fixture / name} not found; run make_fixture.py first")

    data_dir = app_data_dir(args.app_data)
    db_path = data_dir / "meetings.db"
    audio_dir = data_dir / "meetings"
    audio_dir.mkdir(parents=True, exist_ok=True)

    conn = connect(db_path)
    saved = backup(db_path)

    duration_s = max(1, round(truth["duration_ms"] / 1000))
    ended_at = int(time.time())
    started_at = ended_at - duration_s
    title = f"{TEST_PREFIX} {truth.get('title', 'Test meeting')}"

    with conn:
        cursor = conn.execute(
            """INSERT INTO meetings
               (title, started_at, ended_at, status, mic_file, system_file,
                my_notes, notes, notes_template, language, diarized)
               VALUES (?, ?, ?, 'complete', '', '', ?, NULL, NULL, 'en', 0)""",
            (title, started_at, ended_at, args.my_notes),
        )
        meeting_id = cursor.lastrowid

        mic_file = f"meeting_{meeting_id}_mic.wav"
        system_file = f"meeting_{meeting_id}_system.wav"
        shutil.copy2(fixture / "mic.wav", audio_dir / mic_file)
        shutil.copy2(fixture / "system.wav", audio_dir / system_file)
        conn.execute(
            "UPDATE meetings SET mic_file = ?, system_file = ? WHERE id = ?",
            (mic_file, system_file, meeting_id),
        )

        # Pre-diarization speaker keys, exactly as the capture path writes them:
        # `SpeakerSource::default_speaker_key` gives "me" for the microphone and
        # one shared "them" for everyone on the system side. Seeding the real
        # per-speaker key would hand diarization the answer.
        rows = [
            (
                meeting_id,
                utterance["source"],
                "me" if utterance["source"] == "mic" else "them",
                utterance["start_ms"],
                utterance["end_ms"],
                utterance["text"],
                None,
            )
            for utterance in truth["utterances"]
        ]
        conn.executemany(
            """INSERT INTO meeting_segments
               (meeting_id, source, speaker_key, start_ms, end_ms, text, confidence)
               VALUES (?, ?, ?, ?, ?, ?, ?)""",
            rows,
        )
        conn.executemany(
            """INSERT OR IGNORE INTO meeting_speakers
               (meeting_id, speaker_key, display_name) VALUES (?, ?, ?)""",
            [(meeting_id, "me", "You"), (meeting_id, "them", "Others")],
        )

    system_count = sum(1 for u in truth["utterances"] if u["source"] == "system")
    print(f"seeded meeting {meeting_id}: {title}")
    print(f"  {len(rows)} segments ({system_count} on the system side), {duration_s}s")
    print(f"  expected remote speakers: {', '.join(truth['remote_speakers'])}")
    print(f"  backup: {saved.name}")
    report_model(data_dir)
    print(
        "\nnext:\n"
        "  1. start the app (restart it, or leave and re-enter Meetings, so the\n"
        "     list re-queries -- a direct insert fires no event)\n"
        f"  2. open '{title}' and click Identify speakers\n"
        f"  3. python score_meeting.py --id {meeting_id}"
    )
    conn.close()


# ───────────────────────────── reset / list ─────────────────────────────


def newest_test_meeting(conn: sqlite3.Connection) -> int:
    row = conn.execute(
        "SELECT id FROM meetings WHERE title LIKE ? ORDER BY id DESC LIMIT 1",
        (TEST_PREFIX + "%",),
    ).fetchone()
    if not row:
        die("no [TEST] meeting found; run seed_meeting.py first")
    return int(row[0])


def reset(args: argparse.Namespace) -> None:
    """Put a seeded meeting back to its pre-diarization state.

    `diarize_meeting` returns `Skipped{AlreadyDiarized}` on a second pass, so
    without this a code change cannot be re-measured against the same audio --
    which is the whole point of a fixture.
    """
    data_dir = app_data_dir(args.app_data)
    db_path = data_dir / "meetings.db"
    conn = connect(db_path)
    meeting_id = args.id or newest_test_meeting(conn)

    row = conn.execute("SELECT title FROM meetings WHERE id = ?", (meeting_id,)).fetchone()
    if not row:
        die(f"meeting {meeting_id} does not exist")
    if not row[0].startswith(TEST_PREFIX) and not args.force:
        die(f"meeting {meeting_id} is not a {TEST_PREFIX} meeting; pass --force to reset it anyway")

    backup(db_path)
    with conn:
        conn.execute("UPDATE meetings SET diarized = 0 WHERE id = ?", (meeting_id,))
        conn.execute(
            """UPDATE meeting_segments SET speaker_key = 'them', confidence = NULL
               WHERE meeting_id = ? AND source = 'system'""",
            (meeting_id,),
        )
        conn.execute(
            "DELETE FROM meeting_speakers WHERE meeting_id = ? AND speaker_key NOT IN ('me', 'them')",
            (meeting_id,),
        )
    print(f"reset meeting {meeting_id} ({row[0]}) -- diarize it again from the app")
    conn.close()


def show(args: argparse.Namespace) -> None:
    data_dir = app_data_dir(args.app_data)
    conn = connect(data_dir / "meetings.db")
    print(f"{data_dir / 'meetings.db'}\n")
    print(f"{'id':>4}  {'status':<11} {'diar':<5} {'segs':>5}  {'spk':>4}  title")
    for row in conn.execute(
        """SELECT m.id, m.status, m.diarized, m.title,
                  (SELECT COUNT(*) FROM meeting_segments s WHERE s.meeting_id = m.id),
                  (SELECT COUNT(*) FROM meeting_speakers k
                    WHERE k.meeting_id = m.id AND k.speaker_key NOT IN ('me','them'))
             FROM meetings m ORDER BY m.id"""
    ):
        mid, status, diarized, title, segs, speakers = row
        print(f"{mid:>4}  {status:<11} {'yes' if diarized else 'no':<5} {segs:>5}  {speakers:>4}  {title}")
    report_model(data_dir)
    conn.close()


def cleanup(args: argparse.Namespace) -> None:
    data_dir = app_data_dir(args.app_data)
    db_path = data_dir / "meetings.db"
    audio_dir = data_dir / "meetings"
    conn = connect(db_path)

    victims = conn.execute(
        "SELECT id, title, mic_file, system_file FROM meetings WHERE title LIKE ?",
        (TEST_PREFIX + "%",),
    ).fetchall()
    if not victims:
        print(f"no {TEST_PREFIX} meetings to remove")
        return

    print(f"about to delete {len(victims)} {TEST_PREFIX} meeting(s):")
    for mid, title, _, _ in victims:
        print(f"  {mid}: {title}")
    if not args.yes and input("proceed? [y/N] ").strip().lower() not in ("y", "yes"):
        print("cancelled")
        return

    backup(db_path)
    with conn:
        for mid, _, mic_file, system_file in victims:
            # ON DELETE CASCADE takes the segments and speakers; the FTS triggers
            # take the index. The WAVs are ours, so they go too.
            conn.execute("DELETE FROM meetings WHERE id = ?", (mid,))
            for name in (mic_file, system_file):
                if name:
                    (audio_dir / name).unlink(missing_ok=True)
    print(f"deleted {len(victims)} meeting(s)")
    conn.close()


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--fixture", default="fixture")
    parser.add_argument("--app-data", default=None, help="override the app data directory")
    parser.add_argument("--my-notes", default="Need the migration timing before I can sign off.",
                        help="seed the user's own notes, which notes generation merges in")
    parser.add_argument("--id", type=int, default=None, help="meeting id for --reset")
    parser.add_argument("--reset", action="store_true", help="un-diarize so the pass can run again")
    parser.add_argument("--list", action="store_true", help="list meetings and model status")
    parser.add_argument("--cleanup", action="store_true", help="delete every [TEST] meeting")
    parser.add_argument("--force", action="store_true", help="allow --reset on a non-[TEST] meeting")
    parser.add_argument("--yes", action="store_true", help="skip the --cleanup confirmation")
    args = parser.parse_args()

    if args.list:
        show(args)
    elif args.reset:
        reset(args)
    elif args.cleanup:
        cleanup(args)
    else:
        seed(args)


if __name__ == "__main__":
    main()
