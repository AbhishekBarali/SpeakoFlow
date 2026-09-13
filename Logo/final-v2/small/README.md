# SpeakoFlow small app icon

The approved first imagegen artwork is restored: original waveform proportions,
rounded caps, shading and teal flow. Source: `Logo/small-icon-source.png`.
The full-size reference is pixel-identical to the approved version preserved in
`Logo/final-v2/preview/before-optical-fix.png`.

Small exports use gamma-aware Lanczos reduction with no unsharp filter. This
adjusts light mixing at small edges without redrawing or thickening the mark.
The rejected enlarged vector bars, separate background plate and runtime DPI
icon override have been removed. Existing Windows window-icon behavior is restored.
The 16–64px PNGs, ICO entries, favicon and small ICNS slots share the same renderer.
Large app slots keep the original detailed hero artwork.

The runtime window PNGs use a 48px export. An offscreen Windows `CreateIcon` +
`DrawIconEx` comparison reproduces uneven waveform strokes when the old 64px
source is drawn at 24px; the 48px source has more balanced strokes at that size.
This changes only the exported bitmap size, with no geometry edits or runtime
DPI override. This check does not establish the live Explorer taskbar result.
The ICO still contains its independent 16/20/24/32/40/48/64/128/256px entries.

From `Logo/`, run:

```sh
node build-small-icon.mjs
node apply-icon-to-app.mjs
node patch-icns-small.mjs
node preview-small-icon.mjs
cd ..
node Logo/verify-app-icons.mjs
# Optional, on Windows with PowerShell 7:
pwsh -File Logo/preview-windows-icons.ps1
```

The comparison shows the original detailed logo, the approved first generated
version and the restored version with refined reduction. View at 100% for actual
pixel sizes. Verification checks decoded ICO/ICNS assets and confirms the
1024px reference still exactly matches the approved artwork. SVG files are
raster wrappers, not replacement drawings.

A native rebuild and app restart are needed to replace an already-running icon.

## Original generation prompt

Use case: logo-brand.
Asset type: production Windows desktop app icon for SpeakoFlow, optimized for display at 16, 20, 24, 32, 40, 48 and 64 pixels.
Input image 1 is the existing detailed SpeakoFlow logo, the edit target and brand reference.
Create a polished small-size optical redesign of this exact logo. Keep its rounded square teal tile, bright aqua flowing S-shaped sweep around the left of the waveform, deeper petrol-teal right side, and central white audio waveform. It must still clearly belong to the same brand. The original has too many wispy bands, tiny dots and shaded thin bars which become muddy when reduced.
Simplify to ONE broad graceful aqua S-shaped ribbon sweeping from the top through the left-middle and curving toward the bottom, with at most one subdued supporting contour. Keep the central field behind the white waveform dark enough for crisp contrast. Retain a sense of sculpted fluid depth through clean broad tonal areas, not glass effects or tiny highlights. Remove both tiny outer dots. Use exactly FIVE substantial clean nearly-white round-ended vertical bars, short/medium/tallest/medium/short, on one common horizontal center axis, with generous equally sized gaps. Waveform should occupy about 60 percent of tile width and 46 percent of tile height. Make all bars solid nearly-white; avoid cast shadows, extrusion, fine outlines or a glow around them. Give the waveform clarity at thumbnail size without turning the background into a plain gradient.
Composition: one square icon only, flat front view, no perspective, centered and symmetric tile geometry. The rounded square tile should reach the exact top, bottom, left and right canvas edges, with corner radius about 22.5 percent of width. Actual transparent alpha outside rounded corners. No surrounding padding, presentation board, text, watermark, border, mockup, extra icons or decorative objects. Smooth clean edges, beautifully controlled curves, no grain, no texture. Deliver a single high resolution square PNG icon.
