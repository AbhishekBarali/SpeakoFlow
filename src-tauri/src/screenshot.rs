//! Screen capture for assistant vision: grabs the monitor under the cursor
//! (fallback: primary) and returns a downscaled JPEG as a data URL ready to
//! embed in an OpenAI-compatible `image_url` content part.

use base64::Engine;
use image::imageops::FilterType;
use image::DynamicImage;
use log::debug;
use std::io::Cursor;
use xcap::Monitor;

/// Hard ceiling for the **base64 data URL** (what actually lands in the
/// JSON). Azure's gateway has rejected bodies at inconsistent sizes
/// (observed cuts at ~140 KB–416 KB), so we stay FAR below the smallest
/// observed failure: ≤48 KB encoded keeps the whole request body around
/// ~60 KB. Vision models downscale to 512–768px tiles internally anyway,
/// so the extra resolution was mostly wasted bytes.
const TARGET_ENCODED_BYTES: usize = 48 * 1024;

/// (longest edge, jpeg quality) attempts, best first. The first encoding
/// whose base64 fits TARGET_ENCODED_BYTES wins; later rungs are
/// guaranteed-small fallbacks.
const ENCODE_LADDER: [(u32, u8); 6] = [
    (1280, 52),
    (1152, 48),
    (1024, 44),
    (896, 40),
    (768, 36),
    (640, 32),
];

fn scaled(img: &DynamicImage, max_dim: u32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w.max(h) <= max_dim {
        return img.clone();
    }
    let scale = max_dim as f32 / w.max(h) as f32;
    img.resize(
        (w as f32 * scale) as u32,
        (h as f32 * scale) as u32,
        FilterType::Triangle,
    )
}

fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
    let mut buf = Vec::new();
    rgb.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
        Cursor::new(&mut buf),
        quality,
    ))
    .map_err(|e| format!("Failed to encode screenshot: {}", e))?;
    Ok(buf)
}

/// Capture the active monitor and return a `data:image/jpeg;base64,...` URL,
/// adaptively compressed to stay under provider request-size limits.
pub fn capture_screen_data_url() -> Result<String, String> {
    let start = std::time::Instant::now();

    let monitor = pick_monitor()?;
    let rgba = monitor
        .capture_image()
        .map_err(|e| format!("Screen capture failed: {}", e))?;

    let img = DynamicImage::ImageRgba8(rgba);

    let mut chosen: Option<(Vec<u8>, u32, u8)> = None;
    for (max_dim, quality) in ENCODE_LADDER {
        let buf = encode_jpeg(&scaled(&img, max_dim), quality)?;
        // Budget the *encoded* size: base64 grows the payload by 4/3.
        let encoded_size = buf.len().div_ceil(3) * 4;
        chosen = Some((buf, max_dim, quality));
        if encoded_size <= TARGET_ENCODED_BYTES {
            break;
        }
    }
    let (buf, max_dim, quality) =
        chosen.ok_or_else(|| "Screenshot encoding produced no output".to_string())?;

    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf);
    debug!(
        "Captured screen -> {} KB jpeg ({}px q{}, {} KB base64) in {:?}",
        buf.len() / 1024,
        max_dim,
        quality,
        encoded.len() / 1024,
        start.elapsed()
    );

    Ok(format!("data:image/jpeg;base64,{}", encoded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_works_on_this_machine() {
        let result = capture_screen_data_url();
        match result {
            Ok(url) => {
                assert!(url.starts_with("data:image/jpeg;base64,"));
                // The full data URL must stay far below every observed
                // provider cutoff, with headroom for prompt + history.
                assert!(
                    url.len() <= 52 * 1024,
                    "data url too large: {} KB",
                    url.len() / 1024
                );
                println!("capture OK: {} KB data url", url.len() / 1024);
            }
            Err(e) => panic!("capture failed: {}", e),
        }
    }
}

#[cfg(target_os = "windows")]
fn cursor_position() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok()? };
    Some((point.x, point.y))
}

#[cfg(not(target_os = "windows"))]
fn cursor_position() -> Option<(i32, i32)> {
    None
}

fn pick_monitor() -> Result<Monitor, String> {
    if let Some((x, y)) = cursor_position() {
        if let Ok(monitor) = Monitor::from_point(x, y) {
            return Ok(monitor);
        }
    }
    let monitors = Monitor::all().map_err(|e| format!("Failed to enumerate monitors: {}", e))?;
    monitors
        .into_iter()
        .next()
        .ok_or_else(|| "No monitors found".to_string())
}
