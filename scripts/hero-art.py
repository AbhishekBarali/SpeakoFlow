"""
Render the artwork behind each feature page's banner (src/assets/hero/*.webp).

Dev-only and deterministic: every image comes from a fixed seed, so running this
again reproduces the shipped files byte-for-byte on the same numpy/Pillow.

Every element is accumulated as *light* in a linear HDR buffer, tone-mapped
with an exponential curve, and only then converted to sRGB, so shapes keep the
soft edge of something lit rather than the hard edge of a vector. The look is
deliberately restrained: crisp shapes, a little glow, and only a trace of
coloured room light (`GLOW`, `ACCENT_GLOW`, `AMBIENCE`). The first versions
leaned on heavy bloom and wide amber and teal washes, and on the dark theme
that read as glare across the top of every page. The palette is warm white
against the brand teal, on the banner's own near-black so the left edge can be
faded out in CSS without a seam.

Each image keeps its subject in the right ~60%: the banner's title and controls
sit on the left, where the image is masked away.

The pieces share two motifs so the pages read as one family: a voice is a
waveform of vertical bars (Meetings, Home), and text is a line of rounded bars,
one per word, seen too small to read its letters (AI cleanup, Dictionary),
which also keeps the art language-neutral. Each picture is one recognisable
object with one thing happening to it; the versions that failed were the ones
with nothing to recognise (a tangle of threads, floating capsules) or that
drew text as a single row of dashes, which reads as a dashed line.

    python scripts/hero-art.py            # write src/assets/hero/*.webp
    python scripts/hero-art.py --sheet    # also write a contact sheet to %TEMP%

Needs numpy and Pillow (with WebP support).
"""

from __future__ import annotations

import os
import sys
import tempfile

import numpy as np
from PIL import Image

W, H = 1440, 576
OUT = os.path.join(os.path.dirname(__file__), "..", "src", "assets", "hero")

# The banner's background (`--color-hero-surface`), so the art has no edge.
STAGE = (11, 13, 14)


def to_linear(rgb) -> np.ndarray:
    c = np.asarray(rgb, dtype=np.float32) / 255.0
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def to_srgb(x: np.ndarray) -> np.ndarray:
    x = np.clip(x, 0.0, None)
    return np.where(x <= 0.0031308, 12.92 * x, 1.055 * np.power(x, 1 / 2.4) - 0.055)


# Light colours, linear. Named for what they are, not where they go.
AMBER = to_linear((255, 164, 82))
TUNGSTEN = to_linear((255, 206, 150))
ROSE = to_linear((255, 118, 104))
TEAL = to_linear((46, 214, 190))
MINT = to_linear((150, 240, 222))
WHITE = to_linear((255, 244, 230))


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


def splat(buf: np.ndarray, xs, ys, colors, weights) -> None:
    """Add point light at sub-pixel positions (bilinear)."""
    xs = np.asarray(xs, dtype=np.float32)
    ys = np.asarray(ys, dtype=np.float32)
    colors = np.broadcast_to(np.asarray(colors, dtype=np.float32), (xs.size, 3))
    weights = np.broadcast_to(np.asarray(weights, dtype=np.float32), xs.shape)
    x0 = np.floor(xs).astype(np.int64)
    y0 = np.floor(ys).astype(np.int64)
    fx = xs - x0
    fy = ys - y0
    for dx, dy, w in (
        (0, 0, (1 - fx) * (1 - fy)),
        (1, 0, fx * (1 - fy)),
        (0, 1, (1 - fx) * fy),
        (1, 1, fx * fy),
    ):
        xi = x0 + dx
        yi = y0 + dy
        m = (xi >= 0) & (xi < W) & (yi >= 0) & (yi < H)
        np.add.at(buf, (yi[m], xi[m]), colors[m] * (weights[m] * w[m])[:, None])


def disc(buf, cx, cy, r, color, intensity, rim=0.35) -> None:
    """An out-of-focus highlight: a flat disc with a slightly brighter rim."""
    x0, x1 = int(max(0, cx - r - 3)), int(min(W, cx + r + 4))
    y0, y1 = int(max(0, cy - r - 3)), int(min(H, cy + r + 4))
    if x0 >= x1 or y0 >= y1:
        return
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    d = np.hypot(xx - cx, yy - cy)
    alpha = np.clip((r - d) / 1.6, 0.0, 1.0)
    edge = np.clip((d - r * 0.62) / (r * 0.38), 0.0, 1.0)
    alpha *= 1.0 + rim * edge**2
    buf[y0:y1, x0:x1] += np.asarray(color)[None, None, :] * (intensity * alpha)[..., None]


def capsule(buf, x0, x1, cy, h, color, intensity, angle=0.0) -> None:
    """A filled, rounded bar of light from x0 to x1 on the line y = cy: one
    word of text seen too small (or too soft) to read its letters. `angle`
    tilts it about its own centre."""
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


# How many letters a word has, weighted like running prose: mostly short.
WORD_LETTERS = np.array([2, 3, 4, 4, 5, 5, 6, 7, 8, 10])


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0.0, 1.0)
    return t * t * (3 - 2 * t)


def mix(a, b, t):
    return np.asarray(a) + (np.asarray(b) - np.asarray(a)) * t


def develop(hdr: np.ndarray, rng, exposure=1.0) -> Image.Image:
    """Tone-map linear light onto the stage colour and quantise with dither."""
    lit = 1.0 - np.exp(-np.clip(hdr, 0.0, None) * exposure)
    bg = to_linear(STAGE)
    lin = bg + lit * (1.0 - bg)
    srgb = to_srgb(lin)
    # Half-LSB dither: dark gradients band visibly without it.
    srgb = srgb + rng.normal(0.0, 0.55 / 255.0, srgb.shape)
    return Image.fromarray(np.clip(srgb * 255.0 + 0.5, 0, 255).astype(np.uint8))


def lens(fine, glow=0.55, bloom=0.4, haze=0.25, core_sigma=0.9):
    """The four-radius response that makes a line look photographed."""
    return (
        blur(fine, core_sigma)
        + glow * blur(fine, 6.0)
        + bloom * blur(fine, 28.0)
        + haze * blur(fine, 110.0)
    )


# The house style, shared by every piece: crisp shapes with a little glow,
# a touch more on the one accent (a cursor, your word), and only a trace
# of coloured room light behind the subject. The first versions used a
# heavy bloom and wide amber and teal washes; on the dark theme that read
# as glare across the top of every page. A second pass halved what was left:
# even at a "trace", the amber pool behind a subject and the halo round the
# accent were the first thing the eye went to.
GLOW = dict(glow=0.07, bloom=0.02, haze=0.0, core_sigma=0.6)
ACCENT_GLOW = dict(glow=0.16, bloom=0.05, haze=0.0, core_sigma=0.6)
AMBIENCE = 190.0


def ambience(spots) -> np.ndarray:
    """A trace of coloured light behind the subject, so it sits in a room
    rather than on flat black. `spots` is [(x, y, colour), ...]."""
    room = new_buffer()
    for x, y, color in spots:
        splat(room, [x], [y], color, [AMBIENCE])
    return blur(room, 100) + 0.5 * blur(room, 190)


def bar(buf, x, centre, half, color, intensity=0.3) -> None:
    """One bar of a waveform, a crisp rounded stroke as an audio app draws
    one, from centre - half to centre + half."""
    capsule(buf, x - half, x + half, centre, 2.6, color, intensity, angle=np.pi / 2)


def soften(light: np.ndarray, focus: float, drift: float, wash: float = 0.05) -> np.ndarray:
    """The out-of-focus version of a frame: soft, with a little horizontal
    drift like a handheld long exposure. `focus` and `drift` are in source
    pixels; the art is shown at roughly 0.45x."""
    soft = blur(light, focus)
    moving = blur(light, drift, focus * 0.7)
    wide = blur(light, focus * 9)
    return 0.5 * soft + 0.5 * moving + wash * wide


def focal_plane(cx: float, cy: float, rx: float, ry: float) -> np.ndarray:
    """How in-focus each pixel is: 1 around the subject, easing to 0 away
    from it. An ellipse rather than a band, like a fast lens focused on one
    thing."""
    yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
    d = np.sqrt(((xx - cx) / rx) ** 2 + ((yy - cy) / ry) ** 2)
    return 1.0 - smoothstep(0.3, 1.0, d)


def camera(light: np.ndarray, plane: np.ndarray, focus: float, drift: float) -> np.ndarray:
    """Photograph the scene with a shallow depth of field.

    The subject keeps its crisp lines and their glow; everything away from
    it goes soft. Both extremes were tried and failed: every line pin-sharp
    read as busy detail to inspect, and the whole frame out of focus read as
    a smudge. Sharp where the eye lands and soft around it is how a
    photograph separates a subject from its setting, and it is what lets the
    headline sit in front of the art without competing with it.

    The blend happens in linear light, so the transition is a real change of
    focus rather than a crossfade between two images.
    """
    sharp = light
    soft = soften(light, focus, drift)
    # The in-focus part keeps a whisper of the soft layer's glow, so the
    # sharp lines sit in atmosphere instead of on flat black. Only a
    # whisper: this and `soften`'s wide wash are glow too, and at their old
    # strength they undid the house style's restraint.
    out = plane[..., None] * (sharp + 0.04 * soft) + (1.0 - plane[..., None]) * soft
    # A gentle vertical vignette, so the light fades into the stage above
    # and below instead of being cut off by the banner's edge.
    y = (np.arange(H, dtype=np.float32) - H / 2) / (H / 2)
    return out * (1.0 - 0.38 * y**2)[:, None, None]


def dust(buf, rng, count, x_range, y_range, colors, size=(4, 22), strength=(0.04, 0.16)):
    """Scattered out-of-focus specks, for depth."""
    for _ in range(count):
        cx = rng.uniform(*x_range)
        cy = rng.uniform(*y_range)
        r = rng.uniform(*size)
        color = colors[rng.integers(len(colors))]
        # Bigger = further out of focus = dimmer per pixel.
        disc(buf, cx, cy, r, color, rng.uniform(*strength) * (10.0 / (r + 6.0)))


# ─────────────────────────────── the pieces ────────────────────────────────


def art_home(rng) -> np.ndarray:
    """Dictation: a voice that runs into a text cursor. One waveform, drawn
    the way Meetings draws a voice, swells in from the left and then narrows
    as it reaches the teal cursor, taking on the cursor's colour as it goes.
    Speak, and it types where you are.

    No text is drawn: the cursor says "this is where words appear" on its
    own. (An earlier version collapsed each spoken word into a dash of
    text, and a row of dashes reads as a dashed line, not as writing.)

    This is the first thing on screen when the app opens, and it set the
    house style the other pieces follow (see `GLOW`)."""
    base = H * 0.5
    caret_x, caret_half = 1240.0, 25.0
    x0, x1 = 560.0, caret_x - 24.0
    max_amp = 74.0
    xs = np.arange(x0, x1, 6.0, dtype=np.float32)
    t = (xs - x0) / (x1 - x0)

    # Speech: words as swells of syllables with short silences between
    # them, the rhythm Meetings has, so it reads as someone talking.
    speech = np.zeros_like(xs)
    edge = x0
    while edge < x1:
        length = rng.uniform(28, 90)
        at = (xs >= edge) & (xs < edge + length)
        n = int(at.sum())
        if n:
            raw = np.abs(rng.normal(0, 1, n)) ** 1.4
            kernel = np.hanning(min(3, n) + 2)[1:-1]
            syllables = np.convolve(raw, kernel / kernel.sum(), mode="same")
            shape = np.sin(np.pi * (np.arange(n) + 0.5) / n) ** 0.6
            speech[at] = (0.25 + 0.75 * syllables / max(syllables.max(), 1e-3)) * shape
        edge += length + rng.uniform(8, 20)
    # Swell in from the left, then narrow into the cursor, ending well
    # below its height so the cursor still stands on its own.
    swell = smoothstep(0.0, 0.2, t) * (1.0 - 0.88 * smoothstep(0.4, 1.0, t))
    half = max_amp * swell * speech
    into_caret = smoothstep(0.55, 1.0, t)

    bars, caret = new_buffer(), new_buffer()
    for xb, a, c in zip(xs, half, into_caret):
        if a < 1.2:
            continue
        # Warm white rather than saturated amber.
        warm = mix(TUNGSTEN, WHITE, 0.25 + 0.2 * rng.random())
        bar(bars, xb, base, a, mix(warm, mix(TEAL, MINT, 0.4), c))
    capsule(caret, caret_x - caret_half, caret_x + caret_half, base, 3.6,
            mix(TEAL, MINT, 0.5), 0.75, angle=np.pi / 2)

    light = lens(bars, **GLOW) + lens(caret, **ACCENT_GLOW)
    light += ambience([(820.0, base, AMBER), (caret_x - 40, base, TEAL)])
    return light


def art_assistant(rng) -> np.ndarray:
    """The assistant: the call's orb, at rest. A dark sphere whose only light
    is a thin rim, warm on the upper left and teal on the lower right, like
    a planet at the edge of an eclipse.

    It is deliberately the quietest picture of the set. The page under it is
    a long list of settings, and a glowing ball with a highlight and light
    swirling inside (the previous version) was the brightest object on it and
    kept pulling the eye away from them. A rim alone still reads as a sphere,
    and as the call's orb, at a fraction of the brightness."""
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
    rim += mix(AMBER, TUNGSTEN, 0.5)[None, None, :] * (0.55 * fresnel * warm * inside)[..., None]
    rim += mix(TEAL, MINT, 0.3)[None, None, :] * (0.6 * fresnel * cool * inside)[..., None]
    # A faint rim all the way round, so the silhouette never breaks.
    rim += mix(TUNGSTEN, TEAL, 0.5)[None, None, :] * (0.06 * fresnel * inside)[..., None]
    # Just enough light inside that the sphere is glass, not a hole.
    body = new_buffer()
    body += (TEAL * 0.03)[None, None, :] * (
        np.exp(-((dx - 0.3) ** 2 + (dy - 0.35) ** 2) / 0.35) * inside
    )[..., None]

    light = blur(rim, 1.0) + GLOW["glow"] * blur(rim, 6) + GLOW["bloom"] * blur(rim, 28)
    light += blur(body, 12.0)
    light += ambience([(cx - 50, cy - 40, AMBER), (cx + 20, cy + 20, TEAL)])
    return light


def art_meetings(rng) -> np.ndarray:
    """Meetings: a two-track recording of a call. You are the warm track on
    top, the other side the teal track below, and the two take turns — the
    thing the feature actually does, keeping the two voices apart.

    (The first version drew a row of dashes per speaker, which nobody read as
    a conversation.)"""
    base = H * 0.5
    gap = 66.0
    max_amp = 58.0
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

    fade = smoothstep(0.0, 0.2, t)
    tracks = (
        (base - gap, who, speech(), TUNGSTEN, WHITE),
        (base + gap, 1.0 - who, speech(), TEAL, MINT),
    )
    bars = new_buffer()
    for centre, active, envelope, c1, c2 in tracks:
        # Between words a track is a thin, continuous line, as an audio
        # editor draws silence. Drawing the room noise as bars instead left
        # rows of dots, which read as a dotted line.
        line = smoothstep(x0, x0 + 160.0, xs)
        for xa, xb_, wa in zip(xs[:-1], xs[1:], line[:-1]):
            capsule(bars, xa, xb_ + 0.5, centre, 1.2, mix(c1, c2, 0.3), 0.12 * wa)
        amp = max_amp * fade * (0.08 + 0.92 * active) * envelope
        for x, a in zip(xs, amp):
            if a < 2.5:
                continue
            bar(bars, x, centre, a, mix(c1, c2, 0.25 + 0.25 * rng.random()))

    light = lens(bars, **GLOW)
    light += ambience([(900.0, base - gap, AMBER), (1250.0, base + gap, TEAL)])
    return light


def art_cleanup(rng) -> np.ndarray:
    """AI cleanup: a paragraph that tidies itself as it reads. On the left
    the words are jumbled (off the line, tilted, crowding each other) with
    a few rose "um"s among them; to the right the same lines sit straight
    and evenly spaced, and that half is the brightest.

    Each word is a rounded bar of light, text seen too small to read, so
    the picture works in every language. (The previous version was a tangle
    of looping threads combing out into lines: it read as busy rather than
    as messy, and nothing in it looked like text.)"""
    fine = new_buffer()
    lines, spacing, h = 5, 38.0, 10.0
    base = H * 0.5
    letter = 7.5
    clean_word = mix(TUNGSTEN, WHITE, 0.45)
    for k in range(lines):
        cy = base + (k - (lines - 1) / 2) * spacing
        # A paragraph: ragged at the right, last line short.
        end = 1340.0 - (rng.uniform(0, 60) if k < lines - 1 else rng.uniform(240, 300))
        x = 560.0 - rng.uniform(0, 70)
        while x < end:
            # 1 where the text is still messy, 0 once it is clean.
            m = float(1.0 - smoothstep(860.0, 1060.0, x))
            if m > 0.5 and rng.random() < 0.2 * m:
                # A filler word: short, rose, a little off the line.
                wd = h * rng.uniform(1.4, 2.0)
                capsule(fine, x, x + wd, cy + rng.normal(0, 5) * m, h,
                        mix(ROSE, AMBER, 0.15), 0.3 * m)
                x += wd + rng.uniform(3, 12)
                continue
            n = float(rng.choice(WORD_LETTERS))
            wd = n * letter * (1 + rng.normal(0, 0.12) * m)
            dy = rng.normal(0, 8.0) * m
            tilt = rng.normal(0, 0.16) * m
            height = h * (1 + rng.normal(0, 0.12) * m)
            color = mix(clean_word, mix(AMBER, TUNGSTEN, 0.5), 0.7 * m)
            # The tidy half is the subject, so it is also the brightest.
            capsule(fine, x, x + wd, cy + dy, height, color,
                    (0.32 - 0.1 * m) * (1 + rng.normal(0, 0.12) * m), angle=tilt)
            # A word space; mess crowds some words together and strands others.
            x += wd + 8.0 + m * rng.uniform(-8, 16)
    light = lens(fine, **GLOW)
    light += ambience([(780.0, base, AMBER), (1220.0, base, TEAL)])
    return light


def art_dictionary(rng) -> np.ndarray:
    """Dictionary: a paragraph in quiet warm white, and one word in it lit
    teal. That word is yours, the name or term SpeakoFlow now spells your
    way every time it comes up.

    It shares cleanup's vocabulary, words as rounded bars of light, so the
    two writing pages read as one family. (The previous version floated
    capsule outlines over bokeh, which nobody read as words, let alone as
    one word among many.)"""
    lines, spacing, h, letter = 4, 36.0, 10.0, 7.5
    base = H * 0.5
    focus_line, focus_at, focus_len = 1, 990.0, 11 * letter
    prose = new_buffer()
    word = new_buffer()
    focus_x = 0.0
    for k in range(lines):
        cy = base + (k - (lines - 1) / 2) * spacing
        end = 1350.0 - (rng.uniform(0, 60) if k < lines - 1 else rng.uniform(220, 280))
        x = 600.0 - rng.uniform(0, 80)
        placed = False
        while x < end:
            if k == focus_line and not placed and x > focus_at:
                focus_x = x
                capsule(word, x, x + focus_len, cy, h, mix(TEAL, MINT, 0.5), 0.75)
                x += focus_len + 9.0
                placed = True
                continue
            wd = float(rng.choice(WORD_LETTERS)) * letter
            capsule(prose, x, x + wd, cy, h, mix(TUNGSTEN, WHITE, 0.3),
                    0.22 * (1 + rng.normal(0, 0.06)))
            x += wd + 9.0
    focus_y = base + (focus_line - (lines - 1) / 2) * spacing
    light = lens(prose, **GLOW) + lens(word, **ACCENT_GLOW)
    light += ambience([(800.0, base + 20, AMBER), (focus_x + focus_len / 2, focus_y, TEAL)])
    return light


# name: (piece, seed, exposure, focal plane, focus, drift, crisp) — see
# `camera`. The focal planes are wide: every subject is meant to be read
# sharp, with only the far edges going soft. (Tight planes left the rest of
# each picture as a blurred smudge of light, which is glow by another name.)
PIECES = {
    "home": (art_home, 11, 1.0, (930, H * 0.5, 540, 230), 6.0, 12.0, 1.0),
    "assistant": (art_assistant, 23, 1.0, (1060, H * 0.5, 260, 260), 8.0, 16.0, 0.9),
    "meetings": (art_meetings, 5, 1.0, (980, H * 0.5, 640, 280), 7.0, 18.0, 1.0),
    "cleanup": (art_cleanup, 17, 1.0, (980, H * 0.5, 600, 260), 6.0, 12.0, 1.0),
    "dictionary": (art_dictionary, 3, 1.0, (1000, H * 0.5 - 18, 520, 200), 7.0, 12.0, 1.0),
}


def main() -> None:
    os.makedirs(OUT, exist_ok=True)
    rendered = []
    for name, (fn, seed, exposure, plane, focus, drift, crisp) in PIECES.items():
        rng = np.random.default_rng(seed)
        image = develop(
            camera(fn(rng), crisp * focal_plane(*plane), focus, drift), rng, exposure
        )
        path = os.path.join(OUT, f"{name}.webp")
        # Soft gradients band at lower qualities; they also compress well.
        image.save(path, "WEBP", quality=90, method=6)
        rendered.append(image)
        print(f"{name}\t{os.path.getsize(path) // 1024} KB\t{path}")
    if "--sheet" in sys.argv:
        # Roughly the size the art is shown at, so the sheet judges what the
        # eye sees rather than the source pixels.
        tw, th = 640, 256
        sheet = Image.new("RGB", (tw, th * len(rendered)), STAGE)
        for i, image in enumerate(rendered):
            sheet.paste(image.resize((tw, th), Image.LANCZOS), (0, i * th))
        path = os.path.join(tempfile.gettempdir(), "sf-shots", "hero-sheet.png")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        sheet.save(path)
        print(f"sheet\t{path}")


if __name__ == "__main__":
    main()
