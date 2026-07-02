//! Screen capture for assistant vision: grabs the monitor under the cursor
//! (fallback: primary) and returns a downscaled JPEG as a data URL ready to
//! embed in an OpenAI-compatible `image_url` content part.

use base64::Engine;
use image::imageops::FilterType;
use image::DynamicImage;
use log::debug;
use std::io::Cursor;
use xcap::Monitor;

/// Hard ceiling for the **base64 data URL** when talking to a finicky gateway
/// (Azure has rejected bodies at inconsistent sizes, ~140 KB–416 KB), so the
/// conservative path stays far below the smallest observed failure.
const CONSERVATIVE_TARGET_BYTES: usize = 48 * 1024;

/// Balanced cap for the local llama.cpp vision model: readable, but small
/// enough that the image's vision-token count stays modest on small models.
const LOCAL_TARGET_BYTES: usize = 200 * 1024;

/// Generous ceiling for everyone else (OpenAI, Gemini, Anthropic, and the
/// local llama.cpp engine all accept far larger images). Higher resolution +
/// quality keeps on-screen text, code, and error messages legible, which is
/// the whole point of vision. Still small enough to stay snappy.
const GENEROUS_TARGET_BYTES: usize = 384 * 1024;

/// (longest edge, jpeg quality) attempts, best first. The first encoding whose
/// base64 fits the active target wins; later rungs are smaller fallbacks.
const CONSERVATIVE_LADDER: [(u32, u8); 6] = [
    (1280, 52),
    (1152, 48),
    (1024, 44),
    (896, 40),
    (768, 36),
    (640, 32),
];

/// Local vision: ~1280px reads most on-screen text while keeping the image to
/// roughly ~1k–1.2k vision tokens on Qwen-VL (a few hundred on Gemma 3).
const LOCAL_LADDER: [(u32, u8); 4] = [(1280, 78), (1152, 70), (1024, 62), (896, 54)];

/// Sharper ladder for cloud providers — most readable for fine text.
const GENEROUS_LADDER: [(u32, u8); 5] =
    [(1568, 80), (1440, 74), (1280, 68), (1152, 60), (1024, 52)];

/// Which size/quality budget to use, chosen from the active provider.
#[derive(Clone, Copy)]
pub enum CaptureProfile {
    /// Strict gateways (Azure): tiny body.
    Conservative,
    /// Local llama.cpp vision model: balanced legibility vs token count.
    Local,
    /// Cloud vision models: sharpest.
    Generous,
}

impl CaptureProfile {
    /// Pick the profile from a provider's base URL: tiny for Azure, balanced
    /// for loopback (the built-in/local engine), sharp for everything else.
    pub fn for_base_url(base_url: &str) -> Self {
        let url = base_url.to_ascii_lowercase();
        if url.contains("azure") {
            CaptureProfile::Conservative
        } else if url.contains("127.0.0.1") || url.contains("localhost") {
            CaptureProfile::Local
        } else {
            CaptureProfile::Generous
        }
    }
}

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
/// adaptively compressed to the budget for the chosen [`CaptureProfile`].
pub fn capture_screen_data_url(profile: CaptureProfile) -> Result<String, String> {
    let start = std::time::Instant::now();

    let monitor = pick_monitor()?;
    let rgba = monitor
        .capture_image()
        .map_err(|e| format!("Screen capture failed: {}", e))?;

    let img = DynamicImage::ImageRgba8(rgba);

    let (ladder, target): (&[(u32, u8)], usize) = match profile {
        CaptureProfile::Conservative => (&CONSERVATIVE_LADDER, CONSERVATIVE_TARGET_BYTES),
        CaptureProfile::Local => (&LOCAL_LADDER, LOCAL_TARGET_BYTES),
        CaptureProfile::Generous => (&GENEROUS_LADDER, GENEROUS_TARGET_BYTES),
    };

    let mut chosen: Option<(Vec<u8>, u32, u8)> = None;
    for &(max_dim, quality) in ladder {
        let buf = encode_jpeg(&scaled(&img, max_dim), quality)?;
        // Budget the *encoded* size: base64 grows the payload by 4/3.
        let encoded_size = buf.len().div_ceil(3) * 4;
        chosen = Some((buf, max_dim, quality));
        if encoded_size <= target {
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

/// Capture the active monitor into a raw image (no scaling/encoding), for the
/// region-snip flow: grab the frame BEFORE the selection overlay opens, then
/// crop the user's rectangle out of it afterwards.
pub fn capture_screen_raw() -> Result<DynamicImage, String> {
    let monitor = pick_monitor()?;
    let rgba = monitor
        .capture_image()
        .map_err(|e| format!("Screen capture failed: {}", e))?;
    Ok(DynamicImage::ImageRgba8(rgba))
}

/// Crop a physical-pixel region out of a captured frame and encode it as a
/// `data:image/jpeg;base64,...` URL on the ladder for the given profile.
pub fn encode_region_data_url(
    img: &DynamicImage,
    profile: CaptureProfile,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Result<String, String> {
    let (iw, ih) = (img.width(), img.height());
    let x = x.min(iw.saturating_sub(1));
    let y = y.min(ih.saturating_sub(1));
    let w = w.clamp(1, iw - x);
    let h = h.clamp(1, ih - y);
    let crop = img.crop_imm(x, y, w, h);

    let (ladder, target): (&[(u32, u8)], usize) = match profile {
        CaptureProfile::Conservative => (&CONSERVATIVE_LADDER, CONSERVATIVE_TARGET_BYTES),
        CaptureProfile::Local => (&LOCAL_LADDER, LOCAL_TARGET_BYTES),
        CaptureProfile::Generous => (&GENEROUS_LADDER, GENEROUS_TARGET_BYTES),
    };
    let mut chosen: Option<Vec<u8>> = None;
    for &(max_dim, quality) in ladder {
        let buf = encode_jpeg(&scaled(&crop, max_dim), quality)?;
        let encoded_size = buf.len().div_ceil(3) * 4;
        chosen = Some(buf);
        if encoded_size <= target {
            break;
        }
    }
    let buf = chosen.ok_or_else(|| "Region encoding produced no output".to_string())?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf);
    Ok(format!("data:image/jpeg;base64,{}", encoded))
}

/// Load an image file from disk, downscale it to a provider-friendly size, and
/// return it as a `data:image/jpeg;base64,...` URL (used for image attachments
/// picked or dropped into the assistant panel).
pub fn image_file_to_data_url(path: &str) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("Can't read file: {}", e))?;
    if meta.len() > 25 * 1024 * 1024 {
        return Err("Image is too large (over 25 MB)".to_string());
    }
    let img = image::open(path).map_err(|e| format!("Can't open image: {}", e))?;
    let buf = encode_jpeg(&scaled(&img, 1568), 80)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf);
    Ok(format!("data:image/jpeg;base64,{}", encoded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_works_on_this_machine() {
        let result = capture_screen_data_url(CaptureProfile::Conservative);
        match result {
            Ok(url) => {
                assert!(url.starts_with("data:image/jpeg;base64,"));
                // Conservative path must stay far below every observed
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
pub(crate) fn cursor_position() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok()? };
    Some((point.x, point.y))
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn cursor_position() -> Option<(i32, i32)> {
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
