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
//! if the person typed one, and — only if they left the box ticked — the app
//! version, OS and install type. No logs, no transcripts, no identifiers.

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

#[derive(Serialize, Deserialize, Type, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackKind {
    Bug,
    Idea,
    Question,
}

#[derive(Deserialize, Type, Debug)]
pub struct FeedbackRequest {
    pub kind: FeedbackKind,
    pub message: String,
    pub email: Option<String>,
    pub include_system_info: bool,
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
}

impl FeedbackError {
    fn message(&self) -> &'static str {
        match self {
            Self::TooShort => "Write a few words so there's something to go on.",
            Self::TooLong => {
                "That's longer than 5,000 characters. Trim it a little and send again."
            }
            Self::InvalidEmail => "That email address doesn't look right.",
        }
    }
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
        400 | 413 | 415 | 422 => "The feedback couldn't be accepted. Try shortening it and send again.",
        _ => "The feedback service isn't answering right now. Your message is saved here, so try again later.",
    }
}

#[tauri::command]
#[specta::specta]
pub async fn send_feedback(app: AppHandle, request: FeedbackRequest) -> Result<(), String> {
    let clean = clean_feedback(&request.message, request.email.as_deref())
        .map_err(|e| e.message().to_string())?;
    let payload = FeedbackPayload {
        kind: request.kind,
        message: &clean.message,
        email: clean.email.as_deref(),
        system: request.include_system_info.then(|| system_info(&app)),
    };

    let client = reqwest::Client::builder()
        .user_agent(concat!("SpeakoFlow/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
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
        info!(
            "Feedback sent ({:?}, {} chars)",
            request.kind,
            clean.message.chars().count()
        );
        return Ok(());
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
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "idea", "message": "dark mode for the overlay" })
        );
    }

    #[test]
    fn rate_limiting_reads_as_a_pause_not_a_failure() {
        assert!(describe_http_failure(429).contains("minute"));
        assert!(describe_http_failure(503).contains("saved"));
    }
}
