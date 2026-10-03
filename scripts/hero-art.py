"""
Render the artwork behind each feature page's banner (src/assets/hero/*.webp).

Dev-only and deterministic: every image comes from a fixed seed, so running this
again reproduces the shipped files byte-for-byte on the same numpy/Pillow.

Only two pages have a banner: Assistant and Meetings, the two whose banner
holds the page's main action (the ask and call keys; Start recording). Home,
AI cleanup and Dictionary used to have one each, and on those pages the art
was decoration: a paragraph of word-bars reads as a loading skeleton, and on
the light theme a near-black slab was the highest-contrast thing on screen.

Each piece is rendered twice, once per theme, on exactly that theme's stage
colour (`--color-hero-surface`, and its light override on `.hero-stage`) so
the CSS mask that fades it toward the text has no seam:

- Dark: every element is accumulated as *light* in a linear buffer and
  tone-mapped with an exponential curve onto the near-black stage.
- Light: the same elements are accumulated as *ink* (absorbance) and laid on
  the pale stage with Beer-Lambert, `paper * exp(-ink)`, so overlaps darken
  the way pigment does. Each light colour has an ink counterpart that reaches
  its target colour at the reference intensity (`INK_REF`).

The look is flat and crisp: shapes on the stage and nothing around them (see
`lens`). The first versions leaned on bloom, coloured room light and a
shallow depth of field, and however far those were turned down, on the dark
theme they read as glare across the top of every page.

Each image keeps its subject in the right ~60%: the banner's title and controls
sit on the left, where the image is masked away. The far right edge stays
clear too, because the banner's corner button sits there.

    python scripts/hero-art.py            # write src/assets/hero/*.webp
    python scripts/hero-art.py --sheet    # also write a contact sheet to %TEMP%

Needs numpy and Pillow (with WebP support).
"""

from __future__ import annotations

import os
import sys
import tempfile
from types import SimpleNamespace

import numpy as np
from PIL import Image

W, H = 1440, 576
OUT = os.path.join(os.path.dirname(__file__), "..", "src", "assets", "hero")

# The banner's background in each theme, so the art has no edge. Keep these
# in step with `--color-hero-surface` in App.css (dark in @theme, light in the
# `:root:not([data-theme="dark"]) .hero-stage` override).
STAGE_DARK = (11, 13, 14)
STAGE_LIGHT = (231, 238, 236)

# The intensity at which a light-theme ink lands exactly on its target colour.
# The waveform bars are drawn at this intensity; fainter strokes (a track's
# silence line) come out lighter, stronger ones (the orb's rim) darker.
INK_REF = 0.3


def to_linear(rgb) -> np.ndarray:
    c = np.asarray(rgb, dtype=np.float32) / 255.0
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def to_srgb(x: np.ndarray) -> np.ndarray:
    x = np.clip(x, 0.0, None)
    return np.where(x <= 0.0031308, 12.92 * x, 1.055 * np.power(x, 1 / 2.4) - 0.055)


def ink(rgb) -> np.ndarray:
    """The absorbance that turns the light stage into `rgb` at `INK_REF`."""
    paper = to_linear(STAGE_LIGHT)
    # Floored: a channel of 0 would need infinite ink.
    target = np.clip(to_linear(rgb), 1e-3, paper * 0.999)
    return (-np.log(target / paper) / INK_REF).astype(np.float32)


# Named for what they are, not where they go. On the dark stage these are
# lights; on the light stage, the inks that play the same part: warm white
# becomes a warm graphite, the teal stays the brand's teal.
PALETTES = {
    "dark": SimpleNamespace(
        AMBER=to_linear((255, 164, 82)),
        TUNGSTEN=to_linear((255, 206, 150)),
        TEAL=to_linear((46, 214, 190)),
        MINT=to_linear((150, 240, 222)),
        WHITE=to_linear((255, 244, 230)),
        # How much colour the orb holds inside its rim.
        ORB_BODY=0.03,
    ),
    "light": SimpleNamespace(
        AMBER=ink((178, 116, 62)),
        TUNGSTEN=ink((146, 128, 110)),
        TEAL=ink((0, 140, 124)),
        MINT=ink((74, 168, 154)),
        WHITE=ink((112, 104, 96)),
        # Teal ink has almost no red, so even a trace of it tints strongly.
        ORB_BODY=0.008,
    ),
}


def blur(img: np.ndarray, sx: float, sy: float | None = None) -> np.ndarray:
    """Gaussian blur through the FFT, one axis at a time (sx along x)."""
    sy = sx if sy is None else sy
    out = img
    for axis, s in ((1, sx), (0, sy)):
        if s <= 0:
            continue
        pad = int(3 * s) + 2
        widths = [(0, 0)] * out.ndim
        widths[axis] = (pad, pad)
        padded = np.pad(out, widths)
        n = padded.shape[axis]
        freq = np.fft.rfftfreq(n)
        transfer = np.exp(-2.0 * (np.pi * freq * s) ** 2)
        shape = [1] * padded.ndim
        shape[axis] = freq.size
        spectrum = np.fft.rfft(padded, axis=axis) * transfer.reshape(shape)
        padded = np.fft.irfft(spectrum, n=n, axis=axis)
        keep = [slice(None)] * padded.ndim
        keep[axis] = slice(pad, pad + out.shape[axis])
        out = padded[tuple(keep)]
    return out.astype(np.float32)


def new_buffer() -> np.ndarray:
    return np.zeros((H, W, 3), dtype=np.float32)


def capsule(buf, x0, x1, cy, h, color, intensity, angle=0.0) -> None:
    """A filled, rounded bar from x0 to x1 on the line y = cy. `angle` tilts
    it about its own centre."""
    r = h / 2
    mx = (x0 + x1) / 2
    half = max(0.0, (x1 - x0) / 2 - r)
    ux, uy = np.cos(angle), np.sin(angle)
    pad = half + r + 3
    bx0, bx1 = int(max(0, mx - pad)), int(min(W, mx + pad + 1))
    by0, by1 = int(max(0, cy - pad)), int(min(H, cy + pad + 1))
    if bx0 >= bx1 or by0 >= by1:
        return
    yy, xx = np.mgrid[by0:by1, bx0:bx1].astype(np.float32)
    # Distance to the bar's spine, a segment through its centre.
    along = np.clip((xx - mx) * ux + (yy - cy) * uy, -half, half)
    d = np.hypot(xx - (mx + along * ux), yy - (cy + along * uy))
    alpha = np.clip(r - d + 0.5, 0.0, 1.0)
    buf[by0:by1, bx0:bx1] += np.asarray(color)[None, None, :] * (intensity * alpha)[..., None]


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0.0, 1.0)
    return t * t * (3 - 2 * t)


def mix(a, b, t):
    return np.asarray(a) + (np.asarray(b) - np.asarray(a)) * t


def develop(buf: np.ndarray, rng, theme: str, exposure=1.0) -> Image.Image:
    """Put the accumulated buffer on its theme's stage and quantise with
    dither: as light on the dark stage, as ink on the light one."""
    amount = np.clip(buf, 0.0, None) * exposure
    if theme == "dark":
        bg = to_linear(STAGE_DARK)
        lin = bg + (1.0 - np.exp(-amount)) * (1.0 - bg)
    else:
        lin = to_linear(STAGE_LIGHT) * np.exp(-amount)
    srgb = to_srgb(lin)
    # Half-LSB dither: soft gradients band visibly without it.
    srgb = srgb + rng.normal(0.0, 0.55 / 255.0, srgb.shape)
    return Image.fromarray(np.clip(srgb * 255.0 + 0.5, 0, 255).astype(np.uint8))


# The house style, shared by every piece: flat, crisp shapes on the stage,
# and nothing else. No glow, no bloom, no coloured room light, no depth of
# field. Three earlier passes kept turning those down and every one still
# read as glare: any halo on a dark stage is the first thing the eye goes to,
# ahead of the headline. The accent stands out by colour alone.
EDGE = 0.6  # Anti-aliasing only: enough to soften a stair-step, not a glow.


def lens(fine: np.ndarray) -> np.ndarray:
    """Draw a layer as it is: crisp, with just enough softening to anti-alias."""
    return blur(fine, EDGE)


def bar(buf, x, centre, half, color, intensity=INK_REF) -> None:
    """One bar of a waveform, a crisp rounded stroke as an audio app draws
    one, from centre - half to centre + half."""
    capsule(buf, x - half, x + half, centre, 2.6, color, intensity, angle=np.pi / 2)


# ─────────────────────────────── the pieces ────────────────────────────────


def art_assistant(rng, p) -> np.ndarray:
    """The assistant: the call's orb, at rest. A sphere whose only mark is a
    thin rim, warm on the upper left and teal on the lower right, like a
    planet at the edge of an eclipse on the dark stage and a glass bead on
    the light one.

    It is deliberately quiet. The page under it is a long list of settings,
    and a glowing ball with a highlight and light swirling inside (an earlier
    version) was the brightest object on it and kept pulling the eye away
    from them. A rim alone still reads as a sphere, and as the call's orb."""
    cx, cy, r = 1060.0, H * 0.5, 124.0
    yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
    dx, dy = (xx - cx) / r, (yy - cy) / r
    d = np.hypot(dx, dy)
    # Anti-aliased over ~2 source pixels: crisp, but not a cut.
    inside = np.clip((1.0 - d) * r / 2.0, 0.0, 1.0)
    nz = np.sqrt(np.clip(1.0 - d**2, 0.0, 1.0))
    fresnel = (1.0 - nz) ** 3.4
    # Which way each point of the rim faces: +1 toward the warm key light
    # (upper left), -1 toward the teal fill (lower right).
    facing = -(dx + dy) / np.sqrt(2.0) / np.maximum(d, 1e-3)
    warm = smoothstep(0.0, 1.0, facing)
    cool = smoothstep(0.0, 1.0, -facing)
    rim = new_buffer()
    rim += mix(p.AMBER, p.TUNGSTEN, 0.5)[None, None, :] * (0.55 * fresnel * warm * inside)[..., None]
    rim += mix(p.TEAL, p.MINT, 0.3)[None, None, :] * (0.6 * fresnel * cool * inside)[..., None]
    # A faint rim all the way round, so the silhouette never breaks.
    rim += mix(p.TUNGSTEN, p.TEAL, 0.5)[None, None, :] * (0.06 * fresnel * inside)[..., None]
    # Just enough inside that the sphere is glass, not a hole.
    body = new_buffer()
    body += (p.TEAL * p.ORB_BODY)[None, None, :] * (
        np.exp(-((dx - 0.3) ** 2 + (dy - 0.35) ** 2) / 0.35) * inside
    )[..., None]

    return blur(rim, 1.0) + blur(body, 12.0) * inside[..., None]


def art_meetings(rng, p) -> np.ndarray:
    """Meetings: a two-track recording of a call. You are the warm track on
    top, the other side the teal track below, and the two take turns — the
    thing the feature actually does, keeping the two voices apart.

    The tracks fade out before the right edge rather than running off it:
    the banner's corner button sits there, and a waveform running under it
    made the button look like part of the recording."""
    base = H * 0.5
    gap = 60.0
    max_amp = 52.0
    x0, x1 = 500.0, W + 20.0
    xs = np.arange(x0, x1, 6.0, dtype=np.float32)
    t = (xs - x0) / (x1 - x0)

    # Who is talking, as 1 (you) → 0 (them), with soft handovers.
    who = np.zeros_like(xs)
    edge, speaker = x0, 1.0
    while edge < x1:
        length = rng.uniform(110, 240)
        who[(xs >= edge) & (xs < edge + length)] = speaker
        edge += length
        speaker = 1.0 - speaker
    who = np.convolve(who, np.hanning(9) / np.hanning(9).sum(), mode="same")

    def speech() -> np.ndarray:
        """A speech-like envelope: syllable bumps inside words, with gaps."""
        raw = np.abs(rng.normal(0, 1, xs.size)) ** 1.4
        syllables = np.convolve(raw, np.hanning(4) / np.hanning(4).sum(), mode="same")
        words = np.convolve(
            (rng.random(xs.size) > 0.22).astype(np.float32),
            np.hanning(5) / np.hanning(5).sum(),
            mode="same",
        )
        return np.clip(0.15 + 0.85 * syllables / np.percentile(syllables, 97), 0, 1) * words

    # In from the left, out before the corner button on the right.
    fade = smoothstep(0.0, 0.2, t) * (1.0 - smoothstep(W - 300.0, W - 150.0, xs))
    tracks = (
        (base - gap, who, speech(), p.TUNGSTEN, p.WHITE),
        (base + gap, 1.0 - who, speech(), p.TEAL, p.MINT),
    )
    bars = new_buffer()
    for centre, active, envelope, c1, c2 in tracks:
        # Between words a track is a thin, continuous line, as an audio
        # editor draws silence. Drawing the room noise as bars instead left
        # rows of dots, which read as a dotted line.
        line = smoothstep(x0, x0 + 160.0, xs) * (1.0 - smoothstep(W - 300.0, W - 150.0, xs))
        for xa, xb_, wa in zip(xs[:-1], xs[1:], line[:-1]):
            capsule(bars, xa, xb_ + 0.5, centre, 1.2, mix(c1, c2, 0.3), 0.12 * wa)
        amp = max_amp * fade * (0.08 + 0.92 * active) * envelope
        for x, a in zip(xs, amp):
            if a < 2.5:
                continue
            bar(bars, x, centre, a, mix(c1, c2, 0.25 + 0.25 * rng.random()))

    return lens(bars)


# name: (piece, seed, exposure per theme). Each is written as `<name>.webp`
# (dark) and `<name>-light.webp`, from the same seed, so the two themes show
# the same picture. The light orb is exposed down: ink on paper reads heavier
# than the same rim as light on black, and the orb is meant to be the
# quietest thing on its page.
PIECES = {
    "assistant": (art_assistant, 23, {"dark": 1.0, "light": 0.7}),
    "meetings": (art_meetings, 5, {"dark": 1.0, "light": 1.0}),
}


def main() -> None:
    os.makedirs(OUT, exist_ok=True)
    rendered = []
    for name, (fn, seed, exposure) in PIECES.items():
        for theme, suffix in (("dark", ""), ("light", "-light")):
            rng = np.random.default_rng(seed)
            image = develop(fn(rng, PALETTES[theme]), rng, theme, exposure[theme])
            path = os.path.join(OUT, f"{name}{suffix}.webp")
            # Soft gradients band at lower qualities; they also compress well.
            image.save(path, "WEBP", quality=90, method=6)
            rendered.append(image)
            print(f"{name}{suffix}\t{os.path.getsize(path) // 1024} KB\t{path}")
    if "--sheet" in sys.argv:
        # Roughly the size the art is shown at, so the sheet judges what the
        # eye sees rather than the source pixels.
        tw, th = 640, 256
        sheet = Image.new("RGB", (tw, th * len(rendered)), (128, 128, 128))
        for i, image in enumerate(rendered):
            sheet.paste(image.resize((tw, th), Image.LANCZOS), (0, i * th))
        path = os.path.join(tempfile.gettempdir(), "sf-shots", "hero-sheet.png")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        sheet.save(path)
        print(f"sheet\t{path}")


if __name__ == "__main__":
    main()
