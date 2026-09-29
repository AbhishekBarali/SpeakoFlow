//! In-app feedback: a bug report, an idea, or a question, sent to the
//! maintainer without leaving the app or needing a GitHub account.
//!
//! The app posts to a small Cloudflare Worker (`feedback-worker/`), which
//! files the message as an issue in a private GitHub repository. The Worker
//! holds the GitHub credential; nothing secret ships in the app, so the worst
//! a copied request can do is send more feedback, which the Worker
//! rate-limits.
//!
//! Sent from Rust rather than the webview so there is no CORS surface: the
//! Worker answers no browser preflight at all, which means a web page cannot
//! post to it on a visitor's behalf.
//!
//! What is sent is exactly what the dialog shows: the message, an email only
//! if the person typed one, any screenshots they attached (compressed in the
//! dialog first, since each one is committed to the feedback repository), and
//! — only if they left the box ticked — the app version, OS and install type.
//! No logs, no transcripts, no identifiers.

use log::{info, warn};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::time::Duration;
use tauri::AppHandle;

pub const FEEDBACK_ENDPOINT: &str = "https://feedback.speakoflow.com/v1/feedback";

/// Mirrors the Worker's limits so the dialog can say no before a round trip.
pub const MIN_MESSAGE_CHARS: usize = 3;
pub const MAX_MESSAGE_CHARS: usize = 5000;
const MAX_EMAIL_CHARS: usize = 254;
/// Screenshots are compressed in the dialog before they get here (usually to
/// a few hundred KB); these caps only catch a bypassed or broken compressor.
pub const MAX_ATTACHMENTS: usize = 3;
pub const MAX_ATTACHMENT_BYTES: usize = 1024 * 1024;

#[derive(Serialize, Deserialize, Type, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackKind {
    Bug,
    Idea,
    Question,
}

/// One screenshot as the dialog hands it over: a MIME type and plain base64.
#[derive(Serialize, Deserialize, Type, Clone, Debug, PartialEq, Eq)]
pub struct FeedbackAttachment {
    pub media_type: String,
    pub data: String,
}

#[derive(Deserialize, Type, Debug)]
pub struct FeedbackRequest {
    pub kind: FeedbackKind,
    pub message: String,
    pub email: Option<String>,
    pub include_system_info: bool,
    #[serde(default)]
    pub attachments: Vec<FeedbackAttachment>,
}

/// What the dialog needs to know after a successful send. Screenshots are
/// best-effort on the Worker's side, so a report can arrive without them.
#[derive(Serialize, Type, Clone, Debug, Default, PartialEq, Eq)]
pub struct FeedbackOutcome {
    pub attachments_dropped: u32,
}

#[derive(Deserialize, Debug, Default)]
struct WorkerReply {
    #[serde(default)]
    attachments_dropped: u32,
}

/// The optional "about this install" block, shown verbatim in the dialog
/// before it is sent.
#[derive(Serialize, Type, Clone, Debug, PartialEq, Eq)]
pub struct FeedbackSystemInfo {
    pub app_version: String,
    pub os: String,
    pub arch: String,
    pub install: String,
}

#[derive(Serialize, Debug)]
struct FeedbackPayload<'a> {
    kind: FeedbackKind,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<FeedbackSystemInfo>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    attachments: &'a [FeedbackAttachment],
}

/// A validated message and optional email, trimmed.
#[derive(Debug, PartialEq, Eq)]
pub struct CleanFeedback {
    pub message: String,
    pub email: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FeedbackError {
    TooShort,
    TooLong,
    InvalidEmail,
    TooManyAttachments,
    AttachmentTooLarge,
    InvalidAttachment,
}

impl FeedbackError {
    fn message(&self) -> &'static str {
        match self {
            Self::TooShort => "Write a few words so there's something to go on.",
            Self::TooLong => {
                "That's longer than 5,000 characters. Trim it a little and send again."
            }
            Self::InvalidEmail => "That email address doesn't look right.",
            Self::TooManyAttachments => "You can attach up to 3 screenshots.",
            Self::AttachmentTooLarge => {
                "One of the screenshots is too large to send. Remove it and attach a smaller one."
            }
            Self::InvalidAttachment => {
                "One of the attachments isn't a PNG, JPEG or WebP image. Remove it and try again."
            }
        }
    }
}

/// The format the leading bytes say a file is, if it is one we send.
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Decodes each screenshot to check it really is the image it says, and
/// within the size cap, before anything leaves the machine. The Worker checks
/// the same things; checking here turns a rejected upload into a sentence in
/// the dialog instead of a generic "couldn't be accepted".
pub fn check_attachments(attachments: &[FeedbackAttachment]) -> Result<(), FeedbackError> {
    use base64::Engine;
    if attachments.len() > MAX_ATTACHMENTS {
        return Err(FeedbackError::TooManyAttachments);
    }
    for attachment in attachments {
        // Cheap bound first, so a huge string is never decoded.
        if attachment.data.len() > MAX_ATTACHMENT_BYTES.div_ceil(3) * 4 {
            return Err(FeedbackError::AttachmentTooLarge);
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(attachment.data.as_bytes())
            .map_err(|_| FeedbackError::InvalidAttachment)?;
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(FeedbackError::AttachmentTooLarge);
        }
        if sniff_image(&bytes) != Some(attachment.media_type.as_str()) {
            return Err(FeedbackError::InvalidAttachment);
        }
    }
    Ok(())
}

pub fn clean_feedback(message: &str, email: Option<&str>) -> Result<CleanFeedback, FeedbackError> {
    let message = message.trim();
    let chars = message.chars().count();
    if chars < MIN_MESSAGE_CHARS {
        return Err(FeedbackError::TooShort);
    }
    if chars > MAX_MESSAGE_CHARS {
        return Err(FeedbackError::TooLong);
    }
    let email = email.map(str::trim).filter(|e| !e.is_empty());
    if let Some(email) = email {
        if !looks_like_email(email) {
            return Err(FeedbackError::InvalidEmail);
        }
    }
    Ok(CleanFeedback {
        message: message.to_string(),
        email: email.map(str::to_string),
    })
}

/// Deliberately loose: one `@`, something on each side, a dot in the domain,
/// no whitespace. The only goal is catching a typo in the field, not RFC 5322.
pub fn looks_like_email(email: &str) -> bool {
    if email.chars().count() > MAX_EMAIL_CHARS || email.chars().any(char::is_whitespace) {
        return false;
    }
    let mut parts = email.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
}

/// `PRETTY_NAME` from /etc/os-release, e.g. "Ubuntu 24.04.1 LTS".
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn os_release_pretty_name(os_release: &str) -> Option<String> {
    os_release.lines().find_map(|line| {
        let value = line.strip_prefix("PRETTY_NAME=")?;
        let value = value.trim().trim_matches('"').trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn os_description() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Some(name) = std::fs::read_to_string("/etc/os-release")
            .ok()
            .as_deref()
            .and_then(os_release_pretty_name)
        {
            return name;
        }
    }
    let name = match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    };
    format!("{name} {}", tauri_plugin_os::version())
}

fn install_description() -> String {
    let bundle = match tauri::utils::platform::bundle_type() {
        Some(bundle) => format!("{bundle:?}").to_lowercase(),
        None => "unpackaged".to_string(),
    };
    if crate::portable::is_portable() {
        format!("{bundle} (portable)")
    } else {
        bundle
    }
}

pub fn system_info(app: &AppHandle) -> FeedbackSystemInfo {
    FeedbackSystemInfo {
        app_version: app.package_info().version.to_string(),
        os: os_description(),
        arch: std::env::consts::ARCH.to_string(),
        install: install_description(),
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_feedback_system_info(app: AppHandle) -> FeedbackSystemInfo {
    system_info(&app)
}

/// What the person sees when sending fails, keyed on how it failed.
pub fn describe_http_failure(status: u16) -> &'static str {
    match status {
        429 => "You've sent a lot of feedback in a short time. Try again in a minute.",
        413 => "The screenshots are too large to send. Remove one and try again.",
        400 | 415 | 422 => "The feedback couldn't be accepted. Try shortening it and send again.",
        _ => "The feedback service isn't answering right now. Your message is saved here, so try again later.",
    }
}

#[tauri::command]
#[specta::specta]
pub async fn send_feedback(
    app: AppHandle,
    request: FeedbackRequest,
) -> Result<FeedbackOutcome, String> {
    let clean = clean_feedback(&request.message, request.email.as_deref())
        .map_err(|e| e.message().to_string())?;
    check_attachments(&request.attachments).map_err(|e| e.message().to_string())?;
    let payload = FeedbackPayload {
        kind: request.kind,
        message: &clean.message,
        email: clean.email.as_deref(),
        system: request.include_system_info.then(|| system_info(&app)),
        attachments: &request.attachments,
    };

    let client = reqwest::Client::builder()
        .user_agent(concat!("SpeakoFlow/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        // Screenshots mean up to ~4 MB of upload and one GitHub commit each
        // on the Worker's side before the issue is filed.
        .timeout(Duration::from_secs(if request.attachments.is_empty() {
            25
        } else {
            60
        }))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .post(FEEDBACK_ENDPOINT)
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            warn!("Feedback send failed: {e}");
            "Couldn't reach the feedback service. Check your connection; your message is saved here.".to_string()
        })?;

    let status = response.status();
    if status.is_success() {
        let reply: WorkerReply = response.json().await.unwrap_or_default();
        info!(
            "Feedback sent ({:?}, {} chars, {} screenshot(s), {} not stored)",
            request.kind,
            clean.message.chars().count(),
            request.attachments.len(),
            reply.attachments_dropped
        );
        return Ok(FeedbackOutcome {
            attachments_dropped: reply.attachments_dropped,
        });
    }
    let body = response.text().await.unwrap_or_default();
    warn!(
        "Feedback rejected: HTTP {} {}",
        status.as_u16(),
        body.chars().take(200).collect::<String>()
    );
    Err(describe_http_failure(status.as_u16()).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_trimmed_and_bounded() {
        assert_eq!(
            clean_feedback("  it crashed  ", None),
            Ok(CleanFeedback {
                message: "it crashed".into(),
                email: None
            })
        );
        assert_eq!(clean_feedback("  ok ", None), Err(FeedbackError::TooShort));
        let long = "é".repeat(MAX_MESSAGE_CHARS + 1);
        assert_eq!(clean_feedback(&long, None), Err(FeedbackError::TooLong));
        // Counted in characters, not bytes: 5,000 accented letters are fine.
        assert!(clean_feedback(&"é".repeat(MAX_MESSAGE_CHARS), None).is_ok());
    }

    #[test]
    fn an_empty_email_is_no_email() {
        let clean = clean_feedback("the overlay hides", Some("   ")).unwrap();
        assert_eq!(clean.email, None);
    }

    #[test]
    fn email_checking_catches_typos_not_edge_cases() {
        for ok in ["a@b.co", "first.last+tag@mail.example.com"] {
            assert!(looks_like_email(ok), "{ok}");
        }
        for bad in [
            "a@b", "a@@b.co", "@b.co", "a b@c.co", "a@.co", "a@b.co.", "a@b..co",
        ] {
            assert!(!looks_like_email(bad), "{bad}");
        }
        assert_eq!(
            clean_feedback("hello there", Some("nope")),
            Err(FeedbackError::InvalidEmail)
        );
    }

    #[test]
    fn linux_names_the_distribution() {
        let text = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(
            os_release_pretty_name(text).as_deref(),
            Some("Ubuntu 24.04.1 LTS")
        );
        assert_eq!(os_release_pretty_name("ID=arch\n"), None);
    }

    #[test]
    fn the_payload_omits_what_the_person_left_out() {
        let payload = FeedbackPayload {
            kind: FeedbackKind::Idea,
            message: "dark mode for the overlay",
            email: None,
            system: None,
            attachments: &[],
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "idea", "message": "dark mode for the overlay" })
        );
    }

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];

    fn png() -> FeedbackAttachment {
        FeedbackAttachment {
            media_type: "image/png".into(),
            data: b64(PNG),
        }
    }

    #[test]
    fn screenshots_travel_under_the_workers_field_names() {
        let attachments = [png()];
        let payload = FeedbackPayload {
            kind: FeedbackKind::Bug,
            message: "the overlay is clipped",
            email: None,
            system: None,
            attachments: &attachments,
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json["attachments"],
            serde_json::json!([{ "media_type": "image/png", "data": b64(PNG) }])
        );
    }

    #[test]
    fn an_older_dialog_without_attachments_still_deserializes() {
        let request: FeedbackRequest = serde_json::from_value(serde_json::json!({
            "kind": "bug", "message": "hello there", "email": null,
            "include_system_info": false
        }))
        .unwrap();
        assert!(request.attachments.is_empty());
    }

    #[test]
    fn only_real_images_within_the_cap_are_sent() {
        assert_eq!(check_attachments(&[png(), png(), png()]), Ok(()));
        assert_eq!(
            check_attachments(&[png(), png(), png(), png()]),
            Err(FeedbackError::TooManyAttachments)
        );

        let jpeg_claiming_png = FeedbackAttachment {
            media_type: "image/png".into(),
            data: b64(&[0xff, 0xd8, 0xff, 0xe0, 0, 0]),
        };
        assert_eq!(
            check_attachments(&[jpeg_claiming_png]),
            Err(FeedbackError::InvalidAttachment)
        );

        let not_base64 = FeedbackAttachment {
            media_type: "image/png".into(),
            data: "not base64!".into(),
        };
        assert_eq!(
            check_attachments(&[not_base64]),
            Err(FeedbackError::InvalidAttachment)
        );

        let mut big = PNG.to_vec();
        big.resize(MAX_ATTACHMENT_BYTES + 1, 0);
        let too_big = FeedbackAttachment {
            media_type: "image/png".into(),
            data: b64(&big),
        };
        assert_eq!(
            check_attachments(&[too_big]),
            Err(FeedbackError::AttachmentTooLarge)
        );

        // Exactly at the cap is fine.
        big.truncate(MAX_ATTACHMENT_BYTES);
        let at_cap = FeedbackAttachment {
            media_type: "image/png".into(),
            data: b64(&big),
        };
        assert_eq!(check_attachments(&[at_cap]), Ok(()));
    }

    #[test]
    fn webp_and_jpeg_are_recognised() {
        let mut webp = b"RIFF\x10\0\0\0WEBPVP8 ".to_vec();
        webp.extend_from_slice(&[0; 8]);
        assert_eq!(sniff_image(&webp), Some("image/webp"));
        assert_eq!(sniff_image(&[0xff, 0xd8, 0xff, 0xdb]), Some("image/jpeg"));
        assert_eq!(sniff_image(b"GIF89a"), None);
        assert_eq!(sniff_image(b"RIFF"), None);
    }

    #[test]
    fn a_worker_reply_names_dropped_screenshots_or_nothing() {
        let reply: WorkerReply =
            serde_json::from_str(r#"{"ok":true,"attachments_dropped":2}"#).unwrap();
        assert_eq!(reply.attachments_dropped, 2);
        let reply: WorkerReply = serde_json::from_str(r#"{"ok":true}"#).unwrap();
        assert_eq!(reply.attachments_dropped, 0);
    }

    #[test]
    fn rate_limiting_reads_as_a_pause_not_a_failure() {
        assert!(describe_http_failure(429).contains("minute"));
        assert!(describe_http_failure(503).contains("saved"));
    }
}
