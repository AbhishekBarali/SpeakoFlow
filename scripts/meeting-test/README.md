# Testing the meetings feature without a meeting

You never needed other people. `audio_toolkit::audio::loopback` captures the
default **render** endpoint — whatever your computer is playing. It has no idea
whether those samples came from Zoom, a browser tab, or a WAV file. A scripted
recording played through your speakers is not a simulation of a call; to
everything downstream of the audio driver it _is_ a call.

That one fact turns "I need three friends free at the same time" into a script
you can run forty times an hour.

## The four levels

Each level catches a class of bug the one below it cannot. Spend your time at
level 2 and touch the others rarely.

|     | What it tests                                                                                      | What it costs   | How often                                        |
| --- | -------------------------------------------------------------------------------------------------- | --------------- | ------------------------------------------------ |
| 1   | Pure decision logic — the chunker's boundaries, retention, the cluster linkage, DB round-trips     | seconds         | every save                                       |
| 2   | Diarization, notes, ask-about-this-meeting, on a fixed transcript and real audio                   | seconds         | every change to those passes                     |
| 3   | Live capture — loopback, chunker timing, transcription worker, level meters, the pill, WAV writers | one playthrough | before a release, or when capture changes        |
| 4   | Call detection, real network jitter, real voices, real crosstalk                                   | a real call     | once per release, and once with real humans ever |

### Level 1 — unit tests, which you already have

249 test markers across the 14 meetings files. `src-tauri/src/meetings/mod.rs`
is the model to copy: every scheduling and classification decision is a pure
function with its inputs as parameters, so it can be asserted without audio, an
`AppHandle`, or a clock.

```bash
cd src-tauri && cargo test --lib meetings
```

If a new bug is reproducible without audio, it belongs here and nowhere else.

### Level 2 — seed a fixture, run the pass, score it

This is the loop that matters. Recording takes as long as the meeting, but the
parts most likely to be wrong — diarization, notes, retrieval — all run _after_
capture, on the stored WAV and the stored rows. So supply those directly:

```bash
cd scripts/meeting-test
python make_fixture.py          # synthesize a 4-person call + ground truth
python seed_meeting.py          # insert it as a finished, un-diarized meeting
# start the app, open "[TEST] Q3 launch checkpoint", click Identify speakers
python score_meeting.py         # grade it against the truth
python seed_meeting.py --reset  # un-diarize, change code, measure again
```

The seeded segments carry the scripted text verbatim, which holds transcription
constant — so when the score moves, it moved because of the pass you edited and
nothing else. `--reset` exists because `diarize_meeting` answers
`Skipped{AlreadyDiarized}` on a second run, and without it a fixture can only
ever be measured once.

### Level 3 — play the call into the running app

```bash
# terminal 1
bun run tauri dev
# terminal 2, once the meeting is started in the app
python play_fixture.py
```

The remote speakers come out of your speakers and into loopback. Your own four
lines are cued on screen with `>>>` — read them aloud, or see
[a deterministic microphone](#a-deterministic-microphone) below.

This is the only level that exercises the capture path: the WASAPI endpoint, the
chunker closing on silence rather than on your script's boundaries, the
transcription worker's queue, both level meters, the pill, and the WAV writers.
Expect a non-zero WER and a second or two of timing drift here — that is the
real transcription path being measured, not a fault.

### Level 4 — a real call, alone

Two things no fixture reaches. `call_detect.rs` identifies meeting apps by
process, so it only fires when a real one is running. And real voices overlap,
clip, and arrive over a lossy codec in ways synthesized speech does not.

You still do not need participants:

- **Join your own test call.** `zoom.us/test` puts you in a real Zoom meeting by
  yourself. Teams has Test Call, Meet lets you start an empty meeting.
- **Be your second participant.** Join the same meeting from your phone. Two
  endpoints, two real voices over the real codec, one person.
- **Play a conference talk into the call.** Share a browser tab playing a panel
  discussion. Real humans, real crosstalk, several speakers, no scheduling.
- **Record real people once.** Ask three friends for ten minutes on a call, once,
  and keep the WAV forever. That single recording is worth more than every
  synthetic fixture combined, and you only have to ask once. Drop it in as
  `fixture/system.wav` (16 kHz mono f32 — see below) with a hand-written
  `truth.json` and it plugs into the same scorer.

## What the scorer reports

```
source attribution   100.00%   (0.0s of 217.1s on the wrong track)
speakers found            3     (expected 3: alex, priya, sam)
  best mapping       spk_1->priya, spk_2->alex, spk_3->sam
speaker error rate     6.10%
unlabelled            11.20%   (20.2s still on 'them')
WER system            14.30%
```

- **source attribution** must be exactly 100%. The me/them split is a hardware
  fact, not an inference (`meetings/mod.rs` on `SpeakerSource`). Anything less is
  a capture bug — most likely the input device is a loopback source like Stereo
  Mix, so both tracks recorded the same audio.
- **speaker error rate** is the confusion term of DER. Clusters are anonymous, so
  keys are matched to true speakers by the assignment that maximises agreement;
  otherwise renaming a correct clustering would score zero.
- **unlabelled** is reported separately because it is not an error —
  `diarize_meeting` deliberately prefers no label to a wrong one below
  `min_voiced_for_label_ms`. But a high value means the pass _declined_ rather
  than succeeded, which an error rate alone would hide.
- **WER** is zero on a seeded meeting by construction. That is the point.

Thresholds at the top of `score_meeting.py` are loose deliberately: they exist to
catch a regression, not to certify accuracy. Tighten them once you have a
baseline from a real recording.

## Gotchas that look like bugs

**The speaker model is a separate 27 MB download.** Without it diarization
returns `Skipped{ModelNotInstalled}` and changes nothing — silently, as far as
the numbers go. `python seed_meeting.py --list` reports whether it is installed.

**Fixture audio must be 16 kHz mono 32-bit float.** `session::open_wav` writes
that format and `diarize::read_segment` reads `samples::<f32>()`; the code's own
comment notes that an i16 file "would return silent garbage". A fixture in the
wrong format fails in a way that looks exactly like a broken model.

**Utterances under ~2s stay unlabelled.** `DiarizeConfig::window_ms` is 1500, so
a shorter segment yields no embedding window at all. `make_fixture.py` warns if
your script produces one.

**A direct DB insert fires no event.** Nothing tells the frontend a row appeared.
Restart the app, or leave and re-enter the Meetings section so the list
re-queries.

**Your output device must not move mid-test.** Plugging in headphones changes the
render endpoint, and loopback follows the endpoint it opened.

## A deterministic microphone

Reading your lines aloud is fine for a smoke test and useless for comparing two
runs. For a byte-identical mic track:

1. Install a virtual audio cable (VB-CABLE is the usual free one).
2. Set SpeakoFlow's input device to **CABLE Output**.
3. `pip install sounddevice soundfile`
4. `python play_fixture.py --devices` to find **CABLE Input**'s index.
5. `python play_fixture.py --mic-device <index>`

Both tracks are then identical every run, and a score change is attributable
entirely to code.

## Better voices

The default engine is Windows SAPI, which is offline and needs nothing installed
but has only two voices on a typical machine — extra speakers are made by pitch
shifting, which is enough to prove the pipeline splits N voices and holds the
labels stable, but is not a fidelity benchmark.

For genuinely distinct neural speakers:

```bash
pip install edge-tts
python make_fixture.py --engine edge
```

An accuracy number from `--engine edge` means something. A number from `--engine
sapi` means "did this get worse than last time", which is usually what you want.

## Files

|                    |                                                                 |
| ------------------ | --------------------------------------------------------------- |
| `script.json`      | the dialogue: speakers, voices, turns. Edit this, not the code  |
| `make_fixture.py`  | script → `fixture/{system,mic}.wav` + `truth.json` + `cues.txt` |
| `seed_meeting.py`  | insert a fixture as a meeting; `--reset`, `--list`, `--cleanup` |
| `play_fixture.py`  | play the call into the running app, with live cues              |
| `score_meeting.py` | grade a meeting against ground truth. Read-only                 |
| `silence.ps1`      | panic button: stop anything from this folder that is playing    |

`seed_meeting.py` copies `meetings.db` to `meetings.db.bak-<timestamp>` before
every write, only ever inserts, and `--cleanup` refuses to delete a meeting whose
title is not prefixed `[TEST]`.

## If a voice will not stop

```powershell
.\silence.ps1          # show what is playing
.\silence.ps1 -Kill    # stop it
```

`play_fixture.py` plays in-process via `winsound` specifically so this should
never be needed. An earlier version spawned a PowerShell child to play the WAV,
and a force-kill of the parent left the child talking for the remaining three
minutes with no window to explain it — `atexit` and signal handlers cannot help,
because `TerminateProcess` runs no cleanup. Audio owned by the script now stops
when the script does.

To check anything about the script itself — cue timing, alignment, shutdown — use
`--no-audio`. It runs the whole timeline silently. Never verify the harness by
making noise on a machine someone is working on.
