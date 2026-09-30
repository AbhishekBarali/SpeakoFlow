"""
Render the artwork behind each feature page's banner (src/assets/hero/*.webp).

Dev-only and deterministic: every image comes from a fixed seed, so running this
again reproduces the shipped files byte-for-byte on the same numpy/Pillow.

The look is long-exposure light rather than vector graphics: every element is
accumulated as *light* in a linear HDR buffer, blurred at several radii (a sharp
core, a glow, a bloom, a haze), tone-mapped with an exponential curve so bright
cores burn toward white the way film does, and only then converted to sRGB.
That is what makes a handful of lines read as a photograph of light instead of
a CSS gradient. Then `camera` photographs it with a shallow depth of field:
the subject stays crisp and the frame goes soft away from it. The palette is
warm tungsten against the brand teal, the classic cinematic pairing, on the
banner's own near-black so the left edge can be faded out in CSS without a
seam.

Each image keeps its subject in the right ~60%: the banner's title and controls
sit on the left, where the image is masked away.

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


def stroke(buf, xs, ys, color, weight) -> None:
    """A polyline as light with constant energy per pixel of length.

    `weight` is a scalar or one value per vertex (for a line that fades)."""
    xs = np.asarray(xs, dtype=np.float32)
    ys = np.asarray(ys, dtype=np.float32)
    seg = np.hypot(np.diff(xs), np.diff(ys))
    length = np.concatenate([[0.0], np.cumsum(seg)])
    samples = max(2, int(length[-1] / 0.5))
    at = np.linspace(0.0, length[-1], samples)
    px = np.interp(at, length, xs)
    py = np.interp(at, length, ys)
    per_vertex = np.broadcast_to(np.asarray(weight, dtype=np.float32), xs.shape)
    w = 0.5 * np.interp(at, length, per_vertex).astype(np.float32)
    splat(buf, px, py, color, w)


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


def soften(light: np.ndarray, focus: float, drift: float, wash: float = 0.3) -> np.ndarray:
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
    # sharp lines sit in atmosphere instead of on flat black.
    out = plane[..., None] * (sharp + 0.18 * soft) + (1.0 - plane[..., None]) * soft
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
    """Dictation: a ribbon of voice on the right that settles, leftward, into
    four calm lines of text."""
    fine = new_buffer()
    x = np.linspace(360, W + 40, 1800, dtype=np.float32)
    t = (x - 360) / (W + 40 - 360)
    calm = smoothstep(0.18, 0.72, t)  # 0 = text, 1 = voice
    lines = 4
    spacing = 22.0
    strands = 42
    base = H * 0.5
    for i in range(strands):
        u = (i / (strands - 1)) * 2 - 1
        # Strands are grouped by line in the same order they sit in the
        # ribbon, so settling into text never makes them cross.
        k = min(lines - 1, i * lines // strands)
        text_y = base + (k - (lines - 1) / 2) * spacing + rng.normal(0, 0.6)
        wave = (
            np.sin(x / 118.0 + 0.9 + u * 0.9) * 58
            + np.sin(x / 47.0 + 2.1 - u * 1.6) * 20
            + np.sin(x / 23.0 + u * 3.1) * 6
        )
        envelope = smoothstep(0.25, 0.95, t) * (0.55 + 0.45 * np.sin(x / 210.0 + 1.3))
        voice_y = base + wave * envelope + u * (34 + 58 * envelope)
        y = text_y + (voice_y - text_y) * calm
        # Lines of text end raggedly on the left, like a paragraph.
        start = rng.uniform(0.0, 0.14) + (0.1 if k == lines - 1 else 0.0)
        fade = smoothstep(start, start + 0.08, t)
        centre = 1.0 - abs(u)
        if rng.random() < 0.22:
            color = mix(TEAL, MINT, rng.random())
        elif rng.random() < 0.25:
            color = ROSE
        else:
            color = mix(AMBER, TUNGSTEN, 0.3 + 0.7 * centre)
        weight = (0.2 + 0.26 * centre) * (0.7 + 0.9 * calm) * fade
        stroke(fine, x, y, color, weight)
    light = lens(fine, glow=0.75, bloom=0.6, haze=0.4)
    backdrop = new_buffer()
    disc(backdrop, 1120, base, 150, AMBER, 0.08, rim=0.0)
    disc(backdrop, 1320, base + 40, 120, TEAL, 0.05, rim=0.0)
    light += blur(backdrop, 70)
    specks = new_buffer()
    dust(specks, rng, 12, (760, W), (70, H - 70), [AMBER, TUNGSTEN, TEAL], size=(3, 12),
         strength=(0.03, 0.1))
    # Dust is always out of focus: a crisp speck reads as a dead pixel.
    light += blur(specks, 7.0) * 1.6
    return light


def art_assistant(rng) -> np.ndarray:
    """The assistant: a softly lit sphere with a warm core and a teal rim,
    with faint filaments inside like the call's voice orb."""
    cx, cy, r = 1060.0, H * 0.5, 150.0
    yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
    dx, dy = (xx - cx) / r, (yy - cy) / r
    d = np.hypot(dx, dy)
    # A soft edge (several pixels), not a cut: a hard rim on a bright orb
    # read as a sharp line on screen.
    inside = np.clip((1.0 - d) * r / 7.0, 0.0, 1.0)
    nz = np.sqrt(np.clip(1.0 - d**2, 0.0, 1.0))
    rim = (1.0 - nz) ** 2.4
    # Key light from the upper left warms that side of the rim.
    key = np.clip(-(dx * 0.7 + dy * 0.7) / np.maximum(d, 1e-3), 0.0, 1.0)
    sphere = new_buffer()
    sphere += (TEAL * 0.95)[None, None, :] * (rim * (1 - key) * inside)[..., None]
    sphere += (AMBER * 0.85)[None, None, :] * (rim * key * inside)[..., None]
    core = np.exp(-((dx + 0.18) ** 2 + (dy + 0.2) ** 2) / 0.2)
    sphere += (TUNGSTEN * 0.3)[None, None, :] * (core * inside)[..., None]
    sphere += (TEAL * 0.05)[None, None, :] * (nz * inside)[..., None]

    # Filaments inside the sphere, like the call's voice orb.
    fine = new_buffer()
    for i in range(9):
        phase = rng.uniform(0, 2 * np.pi)
        amp = rng.uniform(0.18, 0.5)
        tilt = rng.uniform(-0.35, 0.35)
        s = np.linspace(-0.92, 0.92, 700)
        fx = cx + s * r
        fy = cy + (np.sin(s * rng.uniform(2.2, 4.2) + phase) * amp + s * tilt) * r * np.sqrt(
            np.clip(1 - s**2, 0, 1)
        )
        color = mix(TEAL, TUNGSTEN, rng.random() * 0.8)
        stroke(fine, fx, fy, color, 0.14 * np.sin(np.pi * (s + 1) / 2) ** 1.5)

    light = blur(sphere, 2.5) + 0.6 * blur(sphere, 16) + 0.6 * blur(sphere, 60)
    light += lens(fine, glow=0.8, bloom=0.5, haze=0.2, core_sigma=1.4)
    halo = new_buffer()
    disc(halo, cx, cy, r * 1.2, TEAL, 0.14, rim=0.0)
    disc(halo, cx - 40, cy - 30, r * 0.9, AMBER, 0.08, rim=0.0)
    light += blur(halo, 90)
    # No lens streak: a line of light straight through the orb and across
    # the banner was the one hard edge in the picture, and it drew the eye
    # every time.
    specks = new_buffer()
    dust(specks, rng, 12, (700, W), (50, H - 50), [TEAL, AMBER, MINT], size=(3, 12),
         strength=(0.03, 0.1))
    # Dust is always out of focus: a crisp speck reads as a dead pixel.
    light += blur(specks, 7.0) * 1.6
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
    xs = np.arange(x0, x1, 5.0, dtype=np.float32)
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
        (base - gap, who, speech(), AMBER, TUNGSTEN),
        (base + gap, 1.0 - who, speech(), TEAL, MINT),
    )
    fine = new_buffer()
    for centre, active, envelope, c1, c2 in tracks:
        # The quiet side still picks up a little room noise.
        amp = max_amp * fade * (0.08 + 0.92 * active) * envelope
        for x, a in zip(xs, amp):
            if a < 0.8:
                continue
            ys = np.linspace(centre - a, centre + a, max(2, int(2 * a / 0.7)))
            color = mix(c1, c2, 0.25 + 0.5 * rng.random())
            splat(fine, np.full_like(ys, x), ys, color, 0.5)

    light = lens(fine, glow=0.6, bloom=0.5, haze=0.3, core_sigma=1.2)
    room = new_buffer()
    splat(room, [900.0], [base - gap], AMBER, [7000.0])
    splat(room, [1250.0], [base + gap], TEAL, [7000.0])
    light += 0.8 * blur(room, 90) + 0.6 * blur(room, 180)
    return light


def art_cleanup(rng) -> np.ndarray:
    """AI cleanup: a tangle of threads on the right that combs out, leftward,
    into five clean lines of text.

    It speaks Home's language on purpose, the same silk-like strands, and the
    difference is the point: Home's strands move together as one voice,
    these each wander and loop on their own, which is what reads as mess.
    (Earlier versions scattered short sticks, which read as noise, and then
    blurred them, which read as dirt.)"""
    fine = new_buffer()
    x0, x1 = 380.0, W + 60.0
    x = np.linspace(x0, x1, 2600, dtype=np.float32)
    t = (x - x0) / (x1 - x0)
    tangle = smoothstep(0.3, 0.9, t)  # 0 = clean text, 1 = loose thread
    lines, spacing, base = 5, 24.0, H * 0.5
    strands = 40
    accents = [TEAL, ROSE, AMBER, MINT, TEAL]
    for i in range(strands):
        k = min(lines - 1, i * lines // strands)
        text_y = base + (k - (lines - 1) / 2) * spacing + rng.normal(0, 0.5)
        # Each thread wanders on its own...
        wander = np.zeros_like(x)
        for _ in range(3):
            wander += rng.uniform(22, 62) * np.sin(
                x / rng.uniform(45, 150) + rng.uniform(0, 2 * np.pi)
            )
        # ...and loops: a small circle travelled along the thread, which is
        # what turns a wave into a scribble. Radius and speed both drift, so
        # the loops are irregular like a real tangle, not a coiled spring.
        size = rng.uniform(6, 24) * (
            0.35 + 0.65 * np.abs(np.sin(x / rng.uniform(50, 150) + rng.uniform(0, 6.3)))
        )
        speed = rng.uniform(1 / 44, 1 / 22) * (
            0.6 + 0.4 * np.sin(x / rng.uniform(60, 180) + rng.uniform(0, 6.3))
        )
        phase = np.cumsum(speed * np.gradient(x)) * 2 * np.pi + rng.uniform(0, 6.3)
        xs = x + tangle * size * np.cos(phase)
        ys = text_y + tangle * (wander + size * np.sin(phase))
        # Lines of text end raggedly on the left, like a paragraph.
        start = rng.uniform(0.0, 0.1) + (0.07 if k == lines - 1 else 0.0)
        fade = smoothstep(start, start + 0.07, t)
        weight = 0.22 * fade * (0.8 + 0.4 * rng.random())
        # Clean text is warm white; a loose thread takes an accent colour.
        # Looping makes a thread several times longer per pixel of width,
        # so the loose part is drawn fainter to keep the same brightness.
        stroke(fine, xs, ys, mix(TUNGSTEN, WHITE, 0.35), weight * (1 - tangle))
        accent = mix(accents[i % len(accents)], TUNGSTEN, 0.15)
        stroke(fine, xs, ys, accent, weight * tangle * 0.42)
    light = lens(fine, glow=0.75, bloom=0.55, haze=0.35)
    back = new_buffer()
    splat(back, [1180.0], [base], AMBER, [3500.0])
    splat(back, [1320.0], [base + 20], TEAL, [3000.0])
    light += blur(back, 120)
    return light


def art_dictionary(rng) -> np.ndarray:
    """Dictionary: a field of out-of-focus light with a few words in focus."""
    light = new_buffer()
    bokeh = new_buffer()
    for _ in range(20):
        cx = rng.uniform(640, W + 30)
        cy = rng.uniform(-20, H + 20)
        r = rng.uniform(18, 70)
        color = [AMBER, TEAL, TUNGSTEN, MINT][rng.integers(4)]
        disc(bokeh, cx, cy, r, color, rng.uniform(0.04, 0.09) * (30 / (r + 10)))
    light += blur(bokeh, 5.0) + 0.5 * blur(bokeh, 40)
    haze = new_buffer()
    splat(haze, [1050.0], [H * 0.5], AMBER, [6000.0])
    splat(haze, [1250.0], [H * 0.55], TEAL, [4000.0])
    light += blur(haze, 120)

    words = new_buffer()
    chips = [(820, 214, 70), (930, 262, 108), (1118, 226, 56), (1010, 330, 84),
             (1220, 300, 124), (870, 372, 62), (1300, 196, 72)]
    for i, (cx, cy, wd) in enumerate(chips):
        h = 26.0
        s = np.linspace(0, 1, 900)
        # A capsule outline, traced as one closed stroke.
        ang = s * 2 * np.pi
        px = cx + np.sign(np.cos(ang)) * (wd / 2 - h / 2) + np.cos(ang) * h / 2
        py = cy + np.sin(ang) * h / 2
        color = TEAL if i % 3 == 1 else TUNGSTEN
        stroke(words, px, py, color, 0.7 if i in (1, 4) else 0.42)
    light += lens(words, glow=0.7, bloom=0.55, haze=0.25)
    return light


# name: (piece, seed, exposure, focal plane, focus, drift, crisp) — see
# `camera`. Each focal plane sits on the part of the picture that tells the
# story: where the voice becomes text, the orb, the two voices, where the
# threads comb out, the words among the bokeh. `crisp` caps how sharp that
# part gets: a bright orb at full sharpness read as a hard edge.
PIECES = {
    "home": (art_home, 11, 1.0, (1030, H * 0.5, 400, 230), 9.0, 30.0, 1.0),
    "assistant": (art_assistant, 23, 0.9, (1060, H * 0.5, 300, 300), 9.0, 22.0, 0.55),
    "meetings": (art_meetings, 5, 1.0, (1000, H * 0.5, 440, 240), 7.0, 18.0, 1.0),
    "cleanup": (art_cleanup, 17, 1.0, (900, H * 0.5, 440, 230), 8.0, 26.0, 1.0),
    "dictionary": (art_dictionary, 3, 1.0, (1060, 285, 380, 180), 9.0, 22.0, 1.0),
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
