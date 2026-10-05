"""Encode numbered PNG frames as an animated WebP for the README.

Dev-only, called by scripts/readme-media.mjs for `.webp` outputs. It feeds
the encoder one frame at a time; Pillow's own save_all holds every decoded
frame at once, which for 650 frames at 1920x1200 is several gigabytes.

Every frame is lossless by default, because any lossy frame leaves trails.
libwebp's animation encoder only stores the part of a frame that changed,
and for a lossy candidate it counts a pixel as unchanged when it is within a
few levels of the previous *source* frame (QualityToMaxDiff, 3 at q90). It
never compares against what the decoder is showing. A card fading out over
the dark gradient moves each pixel by about that much per frame, so the
fade is never stored and the card stays on screen, ghosted over the next
scene, until something repaints that area. Mixed mode (lossy or lossless
per frame, whichever is smaller) still picks lossy for most frames and had
the same trails on every scene change of the 2.0 README clips. Holding the
fade back until it crossed the threshold only moved the ghost around.
Lossless compares pixels exactly, so the decoded animation is bit-identical
to the recording (verified frame by frame on 250 frames of the demo).

It costs size: on those 250 frames at 1920x1200, lossless was 1.78 MB
against 0.48 MB mixed. Downscaling made lossless larger, not smaller
(1520 px 2.45 MB, 1280 px 2.42 MB), since resampling breaks up the flat
runs and exact repeats that lossless compresses well, so keep the capture
width. `--mixed` and `--lossy` remain for experiments, not for the README.

    python scripts/readme-webp.py <frames_dir> <out.webp>
        [--width 1920] [--fps 25] [--quality 90] [--mixed | --lossy]
"""

import argparse
import glob
import os
import sys

from PIL import Image, _webp

parser = argparse.ArgumentParser()
parser.add_argument("frames")
parser.add_argument("out")
parser.add_argument("--width", type=int, default=0, help="output width; 0 keeps")
parser.add_argument("--fps", type=float, default=25)
parser.add_argument("--quality", type=float, default=90)
parser.add_argument("--method", type=int, default=6, help="0 fast .. 6 small")
mode = parser.add_mutually_exclusive_group()
mode.add_argument("--mixed", action="store_true", help="lossy or lossless per frame")
mode.add_argument("--lossy", action="store_true", help="every frame lossy")
args = parser.parse_args()
args.lossless = not args.mixed and not args.lossy

files = sorted(glob.glob(os.path.join(args.frames, "*.png")))
if not files:
    sys.exit(f"no frames in {args.frames}")

with Image.open(files[0]) as first:
    width, height = first.size
if args.width and args.width != width:
    height = round(height * args.width / width)
    width = args.width

mixed = args.mixed
# Keyframe spacing as gif2webp chooses it (also Pillow's defaults).
kmin, kmax = (9, 17) if args.lossless else (3, 5)
step = 1000.0 / args.fps

encoder = _webp.WebPAnimEncoder(
    (width, height),
    0,  # background: transparent, as packed ARGB
    0,  # loop forever
    True,  # minimize_size
    kmin,
    kmax,
    mixed,
    False,  # verbose
)
for index, path in enumerate(files):
    with Image.open(path) as frame:
        frame = frame.convert("RGBA")
        if frame.size != (width, height):
            frame = frame.resize((width, height), Image.LANCZOS)
        encoder.add(
            frame.getim(),
            round(index * step),
            args.lossless,
            args.quality,
            100,  # alpha quality: the corners stay exact
            args.method,
        )
    if index % 100 == 0:
        print(f"WEBP\t{index}/{len(files)}", flush=True)

# The end timestamp closes the last frame's duration.
encoder.add(None, round(len(files) * step), args.lossless, args.quality, 100, 0)
data = encoder.assemble(b"", b"", b"")
os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
with open(args.out, "wb") as handle:
    handle.write(data)
print(f"OK\t{args.out}\t{width}x{height}\t{len(data) / 1e6:.2f} MB")
