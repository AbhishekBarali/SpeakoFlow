#!/usr/bin/env python3
"""Score a meeting in meetings.db against the fixture's ground truth.

    python score_meeting.py                 newest [TEST] meeting
    python score_meeting.py --id 3
    python score_meeting.py --id 3 --verbose        per-utterance detail

Read-only. It never writes to the database.

What it measures, and why each number is here:

  source attribution   Did "me" audio land on the mic side and everyone else on
                       the system side? This should be exactly 100%: the whole
                       design rests on it being a hardware fact rather than an
                       inference (see meetings/mod.rs on SpeakerSource). Anything
                       under 100% is a capture bug, not a model being wrong.

  speaker labelling    Diarization. Clusters are anonymous, so the DB's speaker
                       keys are matched to the true speakers by the assignment
                       that maximises agreement -- otherwise relabelling the same
                       correct clustering would score zero. Reported as speaker
                       error rate over voiced duration, which is the confusion
                       term of DER; the miss and false-alarm terms belong to VAD
                       and are not this pass's job.

  unlabelled           System duration still sitting on "them". Not an error:
                       diarize deliberately prefers no label to a wrong one below
                       min_voiced_for_label_ms. But a high number means the pass
                       declined rather than succeeded, which a speaker error rate
                       alone would hide.

  transcription WER    Word error rate against the scripted text. Zero on a
                       seeded meeting by construction -- that is the point, it
                       holds transcription constant so a diarization change is
                       measured alone. On a recorded meeting this is the real
                       number for the transcription path.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sqlite3
import sys
from itertools import permutations
from pathlib import Path

IDENTIFIER = "com.abhishekbarali.speakoflow"
TEST_PREFIX = "[TEST]"

# Anything at or under these passes. Loose on purpose: they exist to catch a
# regression, not to certify accuracy. Tighten them once you have a baseline you
# trust from a real recording.
MAX_SPEAKER_ERROR = 0.20
MAX_UNLABELLED = 0.35
MAX_WER = 0.35


def die(message: str) -> "NoReturn":  # type: ignore[valid-type]
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


# ──────────────────────────────── metrics ────────────────────────────────


def words(text: str) -> list[str]:
    return re.sub(r"[^\w\s']", " ", text.lower()).split()


def edit_distance(reference: list[str], hypothesis: list[str]) -> int:
    """Levenshtein over words. Two rows rather than a full matrix, because a
    whole meeting's transcript against itself is a few thousand words square."""
    if not reference:
        return len(hypothesis)
    previous = list(range(len(reference) + 1))
    for j, hyp in enumerate(hypothesis, start=1):
        current = [j] + [0] * len(reference)
        for i, ref in enumerate(reference, start=1):
            current[i] = min(
                previous[i] + 1,  # deletion
                current[i - 1] + 1,  # insertion
                previous[i - 1] + (ref != hyp),  # substitution
            )
        previous = current
    return previous[-1]


def wer(reference: str, hypothesis: str) -> tuple[float, int]:
    ref, hyp = words(reference), words(hypothesis)
    if not ref:
        return (0.0 if not hyp else 1.0), 0
    return edit_distance(ref, hyp) / len(ref), len(ref)


def overlap_ms(a: tuple[int, int], b: tuple[int, int]) -> int:
    return max(0, min(a[1], b[1]) - max(a[0], b[0]))


def best_assignment(matrix: dict[str, dict[str, int]]) -> dict[str, str]:
    """Map each DB speaker key to the true speaker it mostly is.

    Exhaustive over permutations, which is correct rather than greedy and costs
    nothing at MAX_SPEAKERS = 10 in practice; above 8 keys it falls back to
    greedy, since 9! starts to be real work for a number nobody will read.
    """
    keys = sorted(matrix)
    truths = sorted({t for row in matrix.values() for t in row})
    if not keys or not truths:
        return {}

    if len(keys) <= 8 and len(truths) <= 8:
        best, best_score = {}, -1
        longer, shorter = (keys, truths) if len(keys) >= len(truths) else (truths, keys)
        keys_are_longer = len(keys) >= len(truths)
        for candidate in permutations(longer, len(longer)):
            mapping, score = {}, 0
            for index, item in enumerate(candidate):
                if index >= len(shorter):
                    break
                key, truth = (item, shorter[index]) if keys_are_longer else (shorter[index], item)
                mapping[key] = truth
                score += matrix.get(key, {}).get(truth, 0)
            if score > best_score:
                best, best_score = mapping, score
        # Keys the permutation could not pair (more keys than speakers) still
        # need a home, or their duration would vanish from the error rate.
        for key in keys:
            best.setdefault(key, max(matrix[key], key=matrix[key].get) if matrix[key] else truths[0])
        return best

    used, mapping = set(), {}
    for key in sorted(keys, key=lambda k: -sum(matrix[k].values())):
        options = sorted(matrix[key], key=lambda t: -matrix[key][t])
        pick = next((t for t in options if t not in used), options[0] if options else truths[0])
        mapping[key] = pick
        used.add(pick)
    return mapping


# ───────────────────────────────── loading ─────────────────────────────────


def app_data_dir(explicit: str | None) -> Path:
    if explicit:
        return Path(explicit)
    appdata = os.environ.get("APPDATA")
    if not appdata:
        die("APPDATA is not set; pass --app-data")
    return Path(appdata) / IDENTIFIER


def load(args: argparse.Namespace):
    fixture = Path(args.fixture)
    if not fixture.is_absolute():
        fixture = Path(__file__).parent / fixture
    truth_path = fixture / "truth.json"
    if not truth_path.is_file():
        die(f"{truth_path} not found; run make_fixture.py first")
    truth = json.loads(truth_path.read_text(encoding="utf-8"))

    db_path = app_data_dir(args.app_data) / "meetings.db"
    if not db_path.is_file():
        die(f"{db_path} does not exist")
    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)

    if args.id:
        meeting_id = args.id
    else:
        row = conn.execute(
            "SELECT id FROM meetings WHERE title LIKE ? ORDER BY id DESC LIMIT 1",
            (TEST_PREFIX + "%",),
        ).fetchone()
        if not row:
            die("no [TEST] meeting found; pass --id for a recorded one")
        meeting_id = int(row[0])

    meeting = conn.execute(
        "SELECT title, status, diarized, notes, my_notes FROM meetings WHERE id = ?",
        (meeting_id,),
    ).fetchone()
    if not meeting:
        die(f"meeting {meeting_id} does not exist")

    segments = conn.execute(
        """SELECT source, speaker_key, start_ms, end_ms, text, confidence
             FROM meeting_segments WHERE meeting_id = ? ORDER BY start_ms""",
        (meeting_id,),
    ).fetchall()
    names = dict(
        conn.execute(
            "SELECT speaker_key, display_name FROM meeting_speakers WHERE meeting_id = ?",
            (meeting_id,),
        ).fetchall()
    )
    conn.close()
    return truth, meeting_id, meeting, segments, names


# ───────────────────────────────── report ─────────────────────────────────


def score(args: argparse.Namespace) -> int:
    truth, meeting_id, meeting, segments, names = load(args)
    title, status, diarized, notes, my_notes = meeting
    if not segments:
        die(f"meeting {meeting_id} has no segments; nothing to score")

    print(f"meeting {meeting_id}: {title}")
    print(f"  status {status} | diarized {'yes' if diarized else 'no'} | "
          f"{len(segments)} segments | notes {'yes' if notes else 'no'}")
    print(f"  fixture: {truth['engine']} engine, {len(truth['utterances'])} utterances, "
          f"{truth['duration_ms'] / 1000:.0f}s\n")

    # ── source attribution ────────────────────────────────────────────────
    matched_ms = wrong_source_ms = 0
    for utterance in truth["utterances"]:
        span = (utterance["start_ms"], utterance["end_ms"])
        for source, _key, start, end, _text, _conf in segments:
            shared = overlap_ms(span, (start, end))
            if shared <= 0:
                continue
            matched_ms += shared
            if source != utterance["source"]:
                wrong_source_ms += shared
    source_accuracy = 1.0 - (wrong_source_ms / matched_ms if matched_ms else 1.0)
    print(f"source attribution   {source_accuracy * 100:6.2f}%   "
          f"({wrong_source_ms / 1000:.1f}s of {matched_ms / 1000:.1f}s on the wrong track)")

    # ── speaker labelling ─────────────────────────────────────────────────
    true_system = [u for u in truth["utterances"] if u["source"] == "system"]
    db_system = [s for s in segments if s[0] == "system"]
    total_system_ms = sum(u["end_ms"] - u["start_ms"] for u in true_system)

    unlabelled_ms = 0
    matrix: dict[str, dict[str, int]] = {}
    for _source, key, start, end, _text, _conf in db_system:
        for utterance in true_system:
            shared = overlap_ms((start, end), (utterance["start_ms"], utterance["end_ms"]))
            if shared <= 0:
                continue
            if key in (None, "them"):
                unlabelled_ms += shared
            else:
                matrix.setdefault(key, {}).setdefault(utterance["speaker"], 0)
                matrix[key][utterance["speaker"]] += shared

    expected = len(truth["remote_speakers"])
    found = len(matrix)
    mapping = best_assignment(matrix)
    correct_ms = sum(row.get(mapping.get(key, ""), 0) for key, row in matrix.items())
    labelled_ms = sum(sum(row.values()) for row in matrix.values())
    speaker_error = 1.0 - (correct_ms / labelled_ms) if labelled_ms else 0.0
    unlabelled_share = unlabelled_ms / total_system_ms if total_system_ms else 0.0

    print(f"speakers found       {found:6d}     (expected {expected}: "
          f"{', '.join(truth['remote_speakers'])})")
    if mapping:
        pairs = ", ".join(f"{k}->{v}" for k, v in sorted(mapping.items()))
        print(f"  best mapping       {pairs}")
        for key in sorted(matrix):
            display = names.get(key, key)
            share = sum(matrix[key].values()) / 1000
            hit = matrix[key].get(mapping.get(key, ""), 0) / 1000
            print(f"    {key:<8} ({display:<12}) {share:6.1f}s voiced, "
                  f"{hit / share * 100 if share else 0:5.1f}% one speaker")
    print(f"speaker error rate   {speaker_error * 100:6.2f}%   "
          f"({(labelled_ms - correct_ms) / 1000:.1f}s of {labelled_ms / 1000:.1f}s labelled wrong)")
    print(f"unlabelled           {unlabelled_share * 100:6.2f}%   "
          f"({unlabelled_ms / 1000:.1f}s still on 'them')")

    confidences = [c for *_rest, c in db_system if c is not None]
    if confidences:
        print(f"mean confidence      {sum(confidences) / len(confidences):6.3f}   "
              f"(min {min(confidences):.3f}, {len(confidences)}/{len(db_system)} scored)")

    # ── transcription ─────────────────────────────────────────────────────
    print()
    for side in ("system", "mic"):
        reference = " ".join(u["text"] for u in truth["utterances"] if u["source"] == side)
        hypothesis = " ".join(s[4] for s in segments if s[0] == side)
        if not reference:
            continue
        rate, count = wer(reference, hypothesis)
        print(f"WER {side:<7}         {rate * 100:6.2f}%   ({count} reference words)")
    overall_wer, _ = wer(
        " ".join(u["text"] for u in truth["utterances"]),
        " ".join(s[4] for s in segments),
    )

    drifts = []
    for utterance in truth["utterances"]:
        span = (utterance["start_ms"], utterance["end_ms"])
        best = max(
            (s for s in segments if s[0] == utterance["source"]),
            key=lambda s: overlap_ms(span, (s[2], s[3])),
            default=None,
        )
        if best and overlap_ms(span, (best[2], best[3])) > 0:
            drifts.append(abs(best[2] - utterance["start_ms"]))
    if drifts:
        print(f"timing drift         {sum(drifts) / len(drifts):6.0f}ms  "
              f"mean start offset (max {max(drifts)}ms)")

    if args.verbose:
        print("\nper utterance:")
        for utterance in truth["utterances"]:
            span = (utterance["start_ms"], utterance["end_ms"])
            hits = [s for s in segments if overlap_ms(span, (s[2], s[3])) > 0]
            keys = {s[1] for s in hits if s[0] == "system"}
            got = ", ".join(sorted(k or "null" for k in keys)) or "-"
            mapped = ", ".join(sorted({mapping.get(k, "?") for k in keys if k})) or "-"
            flag = " " if (utterance["source"] == "mic" or mapped == utterance["speaker"]) else "!"
            text = " ".join(s[4] for s in hits)
            rate, _ = wer(utterance["text"], text)
            print(f" {flag} {span[0] / 1000:7.1f}s {utterance['speaker']:<6} "
                  f"key={got:<22} as={mapped:<8} wer={rate * 100:5.1f}%")

    # ── verdict ───────────────────────────────────────────────────────────
    checks = [
        ("source attribution is exact", source_accuracy >= 0.9999),
        (f"speaker count == {expected}", found == expected),
        (f"speaker error <= {MAX_SPEAKER_ERROR:.0%}", speaker_error <= MAX_SPEAKER_ERROR),
        (f"unlabelled <= {MAX_UNLABELLED:.0%}", unlabelled_share <= MAX_UNLABELLED),
        (f"overall WER <= {MAX_WER:.0%}", overall_wer <= MAX_WER),
    ]
    print()
    for label, ok in checks:
        print(f"  [{'PASS' if ok else 'FAIL'}] {label}")

    if not diarized:
        print("\n  note: this meeting is not diarized yet, so the speaker numbers above\n"
              "        are the un-diarized baseline. Open it in the app and click\n"
              "        Identify speakers, then run this again.")
    failed = [label for label, ok in checks if not ok]
    print(f"\n{'PASS' if not failed else 'FAIL: ' + '; '.join(failed)}")
    return 0 if not failed else 1


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--id", type=int, default=None)
    parser.add_argument("--fixture", default="fixture")
    parser.add_argument("--app-data", default=None)
    parser.add_argument("--verbose", "-v", action="store_true")
    sys.exit(score(parser.parse_args()))


if __name__ == "__main__":
    main()
