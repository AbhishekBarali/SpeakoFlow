"""Encode numbered PNG frames as an animated WebP for the README.

Dev-only, called by scripts/readme-media.mjs for `.webp` outputs. Two reasons
this is not ffmpeg's libwebp_anim:

- Mixed mode. libwebp can encode each frame lossy or lossless, whichever is
  smaller, and ffmpeg does not expose it. The demo is flat UI on a dark
  gradient: forced lossy left ghosted streaks in the gradient where a card
  had faded out, and forced lossless was four times the size. Mixed was the
  smallest of the three and the cleanest (measured on the first 150 frames
  of the demo at 1920x1200: mixed q90 0.28 MB, lossy q90 0.43 MB, lossless
  1.24 MB).
- Memory. Pillow's own save_all holds every decoded frame at once, which for
  650 frames at 1920x1200 is several gigabytes. This feeds the encoder one
  frame at a time.

    python scripts/readme-webp.py <frames_dir> <out.webp>
        [--width 1920] [--fps 25] [--quality 90] [--lossy | --lossless]
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
mode.add_argument("--lossy", action="store_true", help="every frame lossy")
mode.add_argument("--lossless", action="store_true", help="every frame lossless")
args = parser.parse_args()

files = sorted(glob.glob(os.path.join(args.frames, "*.png")))
if not files:
    sys.exit(f"no frames in {args.frames}")

with Image.open(files[0]) as first:
    width, height = first.size
if args.width and args.width != width:
    height = round(height * args.width / width)
    width = args.width

mixed = not args.lossy and not args.lossless
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
