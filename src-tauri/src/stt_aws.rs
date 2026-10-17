//! Amazon Transcribe streaming, spoken to directly rather than through the AWS
//! SDK.
//!
//! Amazon Transcribe has no OpenAI-compatible endpoint, and its batch API is not
//! a transcription request at all: it wants the audio uploaded to an S3 bucket,
//! then a job started and polled until it finishes. That is the slow path people
//! end up building middleware around. Its streaming API is the fast one, and it
//! is the only one used here — the session itself lives in
//! [`crate::stt_cloud_stream`], next to ElevenLabs and Deepgram.
//!
//! What makes AWS different from every other provider is collected in this
//! module, so the session only has to move frames:
//!
//! - **SigV4 presigning** ([`streaming_url`]). The WebSocket upgrade carries the
//!   credentials in its query string, signed with the account's secret key. The
//!   signer is tested against AWS's own published presigned-URL example.
//! - **Event-stream framing** ([`audio_event`], [`decode_messages`]). Audio goes
//!   up, and transcripts come back, as AWS's binary event-stream messages: a
//!   length prelude, typed headers, the payload, and two CRC32 checksums.
//! - **What the generic settings cannot express**: an access key ID, a secret
//!   and an optional session token instead of one API key ([`AwsCredentials`]),
//!   a region instead of an endpoint ([`region_from`]), and a full locale
//!   (`en-US`) or an explicit list of candidate languages for auto-detection
//!   instead of a bare language code ([`streaming_language`]).
//!
//! No AWS SDK: `aws-sdk-transcribestreaming` brings the whole Smithy runtime and
//! a second HTTP stack for one signed URL and a frame format that fits on a page.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// The service name Transcribe streaming is signed for.
const SERVICE: &str = "transcribe";

/// Path of the WebSocket streaming endpoint.
const STREAMING_PATH: &str = "/stream-transcription-websocket";

/// How long a presigned URL stays valid: AWS's maximum for this endpoint. The
/// URL is used once, straight away; what this really bounds is how far the
/// computer's clock may be off before AWS refuses the signature.
const PRESIGN_EXPIRES_SECS: u32 = 300;

/// SHA-256 of the empty string, the payload hash of the upgrade `GET`.
const EMPTY_PAYLOAD_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// One set of AWS credentials.
///
/// Stored as a single secret in the provider's ordinary keychain slot —
/// `KEY_ID:SECRET`, or `KEY_ID:SECRET:SESSION_TOKEN` for temporary credentials —
/// which the Settings card composes from separate fields. A colon cannot occur in
/// any of the three: an access key ID is alphanumeric, and a secret access key and
/// a session token are base64. Reusing the one slot is what lets the keychain
/// hydration, the on-disk redaction and the per-provider `set_cloud_stt_api_key`
/// command work for AWS unchanged, instead of a parallel settings path for one
/// provider.
///
/// No `Debug`: it would print the secret.
#[derive(Clone)]
pub(crate) struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    /// Present for temporary credentials (STS, IAM Identity Center); `None` for
    /// an IAM user's long-term key.
    pub session_token: Option<String>,
}

impl AwsCredentials {
    /// Read the stored `KEY_ID:SECRET[:SESSION_TOKEN]` form. `None` for anything
    /// that cannot be a working key, which the resolver reports as a missing key:
    /// the Settings card only ever saves the complete form.
    pub(crate) fn parse(stored: &str) -> Option<Self> {
        let mut parts = stored.trim().split(':').map(str::trim);
        let access_key_id = parts.next()?;
        let secret_access_key = parts.next()?;
        let session_token = parts.next();
        if parts.next().is_some() {
            return None;
        }
        // AWS documents access key IDs as 16–128 word characters; in practice
        // they are 20 uppercase letters and digits (`AKIA…`, `ASIA…`).
        let key_id_ok = (16..=128).contains(&access_key_id.len())
            && access_key_id.chars().all(|c| c.is_ascii_alphanumeric());
        let plain = |value: &str| !value.is_empty() && !value.chars().any(char::is_whitespace);
        if !key_id_ok || !plain(secret_access_key) {
            return None;
        }
        let session_token = match session_token {
            None | Some("") => None,
            Some(token) if plain(token) => Some(token.to_string()),
            Some(_) => return None,
        };
        Some(Self {
            access_key_id: access_key_id.to_string(),
            secret_access_key: secret_access_key.to_string(),
            session_token,
        })
    }
}

/// Regions where Amazon Transcribe streaming runs, from the streaming table of
/// "Amazon Transcribe endpoints and quotas" (AWS General Reference). The Settings
/// picker offers exactly these; `src/lib/cloudStt.ts` mirrors the list.
pub(crate) const STREAMING_REGIONS: &[&str] = &[
    "us-east-1",
    "us-east-2",
    "us-west-2",
    "ca-central-1",
    "mx-central-1",
    "sa-east-1",
    "eu-west-1",
    "eu-west-2",
    "eu-central-1",
    "eu-central-2",
    "af-south-1",
    "ap-south-1",
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-southeast-1",
    "ap-southeast-2",
    "ap-southeast-5",
    "ap-southeast-7",
    "us-gov-east-1",
    "us-gov-west-1",
];

/// Regions where the languages AWS marks with an asterisk in its
/// supported-languages table have no streaming (Tokyo, Cape Town, Malaysia,
/// Thailand). AWS lists `cn-northwest-1` too; the China regions sit on another
/// domain and are not offered here at all.
const REGIONS_WITHOUT_EXTENDED_LANGUAGES: &[&str] = &[
    "af-south-1",
    "ap-northeast-1",
    "ap-southeast-5",
    "ap-southeast-7",
];

/// The AWS region named by whatever is stored in the provider's endpoint slot.
///
/// The Settings card stores a bare region (`eu-west-1`). A pasted streaming
/// endpoint (`transcribestreaming.eu-west-1.amazonaws.com`, with or without a
/// scheme and port) is read too, since that is the other thing AWS's
/// documentation hands people. `None` means nothing usable.
///
/// Any well-formed region is accepted, not only [`STREAMING_REGIONS`]: a region
/// AWS adds later should not need an app release, and the picker is what keeps
/// people on the known ones.
pub(crate) fn region_from(raw: &str) -> Option<String> {
    let lowered = raw.trim().to_ascii_lowercase();
    let without_scheme = lowered
        .split_once("://")
        .map_or(lowered.as_str(), |(_, rest)| rest);
    let host = without_scheme
        .split(['/', '?', ':'])
        .next()
        .unwrap_or_default();
    let candidate = host
        .strip_prefix("transcribestreaming.")
        .and_then(|rest| rest.strip_suffix(".amazonaws.com"))
        .unwrap_or(host);
    is_region_code(candidate).then(|| candidate.to_string())
}

/// `us-east-1`, `ap-southeast-5`, `us-gov-west-1`: lowercase words joined by
/// hyphens, ending in a number.
fn is_region_code(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    let Some((number, words)) = parts.split_last() else {
        return false;
    };
    words.len() >= 2
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
        && words
            .iter()
            .all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase()))
}

/// The streaming locale for each dictation language the app offers, from AWS's
/// supported-languages table ("Data input" includes streaming), and whether AWS
/// marks it as missing from [`REGIONS_WITHOUT_EXTENDED_LANGUAGES`].
///
/// AWS wants a full locale (`en-US`), not the bare code the other providers
/// take. Where it has several for one language, the most widely spoken is used:
/// `en-US`, `es-US` (Latin American Spanish), `pt-BR`, `ar-SA` (Modern Standard
/// Arabic), `sw-KE`. A language missing here — Latin, Māori, Urdu, Mongolian,
/// Nynorsk and the rest — has no streaming model at AWS, and choosing it fails
/// the configuration rather than sending the audio to a different language's
/// model.
const LANGUAGES: &[(&str, &str, bool)] = &[
    ("af", "af-ZA", false),
    ("am", "am-ET", true),
    ("ar", "ar-SA", false),
    ("be", "be-BY", true),
    ("bg", "bg-BG", true),
    ("bn", "bn-IN", true),
    ("bs", "bs-BA", true),
    ("ca", "ca-ES", false),
    ("cs", "cs-CZ", false),
    ("cy", "cy-WL", true),
    ("da", "da-DK", false),
    ("de", "de-DE", false),
    ("el", "el-GR", false),
    ("en", "en-US", false),
    ("es", "es-US", false),
    ("et", "et-ET", true),
    ("eu", "eu-ES", false),
    ("fa", "fa-IR", false),
    ("fi", "fi-FI", false),
    ("fr", "fr-FR", false),
    ("gl", "gl-ES", false),
    ("gu", "gu-IN", true),
    ("he", "he-IL", false),
    ("hi", "hi-IN", false),
    ("hr", "hr-HR", false),
    ("ht", "ht-HT", true),
    ("hu", "hu-HU", true),
    ("hy", "hy-AM", true),
    ("id", "id-ID", false),
    ("is", "is-IS", true),
    ("it", "it-IT", false),
    ("ja", "ja-JP", false),
    // Whisper's code for Javanese is `jw`, which the app's list follows.
    ("jw", "jv-ID", true),
    ("jv", "jv-ID", true),
    ("ka", "ka-GE", true),
    ("kk", "kk-KZ", true),
    ("km", "km-KH", true),
    ("kn", "kn-IN", true),
    ("ko", "ko-KR", false),
    ("lt", "lt-LT", true),
    ("lv", "lv-LV", false),
    ("mk", "mk-MK", true),
    ("ml", "ml-IN", true),
    ("mr", "mr-IN", true),
    ("ms", "ms-MY", false),
    ("my", "my-MM", true),
    ("ne", "ne-NP", true),
    ("nl", "nl-NL", false),
    ("no", "no-NO", false),
    ("pa", "pa-IN", true),
    ("pl", "pl-PL", false),
    ("ps", "ps-AF", true),
    ("pt", "pt-BR", false),
    ("ro", "ro-RO", false),
    ("ru", "ru-RU", false),
    ("si", "si-LK", true),
    ("sk", "sk-SK", false),
    ("sl", "sl-SI", true),
    ("so", "so-SO", false),
    ("sq", "sq-AL", true),
    ("sr", "sr-RS", false),
    ("su", "su-ID", true),
    ("sv", "sv-SE", false),
    ("sw", "sw-KE", true),
    ("ta", "ta-IN", true),
    ("te", "te-IN", true),
    ("th", "th-TH", false),
    ("tl", "tl-PH", false),
    ("tr", "tr-TR", true),
    ("uk", "uk-UA", false),
    ("uz", "uz-UZ", true),
    ("vi", "vi-VN", false),
    // Cantonese. AWS files it under `zh-HK`.
    ("yue", "zh-HK", false),
    ("zh", "zh-CN", false),
    ("zu", "zu-ZA", false),
];

/// Candidates for auto-detection after the app's own language, most widely
/// spoken first. All are available in every streaming region.
const AUTO_CANDIDATES: &[&str] = &[
    "en-US", "es-US", "fr-FR", "de-DE", "pt-BR", "it-IT", "zh-CN", "ja-JP", "hi-IN",
];

/// AWS needs at least two candidates and advises against more than five, because
/// every extra one is another language a short clip can be mistaken for.
const MAX_AUTO_CANDIDATES: usize = 5;

/// How a session tells AWS which language it will hear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamingLanguage {
    /// `language-code`: the user chose a language.
    Code(&'static str),
    /// `identify-language` with these `language-options`, the first of which is
    /// also sent as `preferred-language`.
    Identify(Vec<&'static str>),
}

/// The streaming locale for an app language code, and whether it is one of the
/// languages some regions lack.
fn locale_for(code: &str) -> Option<(&'static str, bool)> {
    let code = code.trim();
    // Script and region subtags decide Chinese only: the app's list carries
    // `zh-Hans` / `zh-Hant`, and a UI locale may arrive as `zh-TW`.
    let lowered = code.to_ascii_lowercase();
    if lowered.starts_with("zh-") {
        let traditional = ["hant", "tw", "hk", "mo"]
            .iter()
            .any(|tag| lowered.split('-').skip(1).any(|part| part == *tag));
        return Some((if traditional { "zh-TW" } else { "zh-CN" }, false));
    }
    let primary = lowered.split(['-', '_']).next().unwrap_or_default();
    LANGUAGES
        .iter()
        .find(|(app, _, _)| *app == primary)
        .map(|(_, locale, extended)| (*locale, *extended))
}

fn available_in(region: &str, extended: bool) -> bool {
    !extended || !REGIONS_WITHOUT_EXTENDED_LANGUAGES.contains(&region)
}

/// How to tell AWS the language, or `None` when it cannot stream it in `region`.
///
/// `selected` is the app's dictation language; `app_language` is the interface
/// language, used only when dictation is set to auto-detect.
///
/// Auto-detection at AWS is not open-ended: it picks the closest match among the
/// candidates it is given, and a speaker of anything else gets a confident
/// transcript in the wrong language. So the candidates are chosen with care —
/// the interface language first (the best evidence there is of what someone
/// speaks, though not proof of it), then the most widely spoken languages, five
/// in all — and Settings tells people to choose their language when it is none
/// of those. The first candidate also goes as `preferred-language`, which AWS
/// documents as what speeds identification up on short clips, and dictations
/// are mostly short clips.
pub(crate) fn streaming_language(
    selected: &str,
    app_language: &str,
    region: &str,
) -> Option<StreamingLanguage> {
    let selected = selected.trim();
    if selected.is_empty() || selected.eq_ignore_ascii_case("auto") {
        return Some(StreamingLanguage::Identify(auto_candidates(
            app_language,
            region,
        )));
    }
    let (locale, extended) = locale_for(selected)?;
    available_in(region, extended).then_some(StreamingLanguage::Code(locale))
}

/// Up to [`MAX_AUTO_CANDIDATES`] locales, one per language: AWS refuses two
/// dialects of the same language in one stream.
fn auto_candidates(app_language: &str, region: &str) -> Vec<&'static str> {
    let interface = locale_for(app_language)
        .filter(|(_, extended)| available_in(region, *extended))
        .map(|(locale, _)| locale);
    let mut candidates: Vec<&'static str> = Vec::with_capacity(MAX_AUTO_CANDIDATES);
    for locale in interface.into_iter().chain(AUTO_CANDIDATES.iter().copied()) {
        let taken = candidates
            .iter()
            .any(|chosen| primary_subtag(chosen) == primary_subtag(locale));
        if !taken && candidates.len() < MAX_AUTO_CANDIDATES {
            candidates.push(locale);
        }
    }
    candidates
}

fn primary_subtag(locale: &str) -> &str {
    locale.split('-').next().unwrap_or(locale)
}

/// Everything a signed streaming URL needs beyond the generic cloud settings,
/// carried on [`crate::settings::ResolvedCloudStt::amazon`]. Like that struct it
/// has no `Debug`, because it holds the secret.
#[derive(Clone)]
pub(crate) struct AmazonTranscribeConfig {
    pub region: String,
    pub credentials: AwsCredentials,
    pub language: StreamingLanguage,
}

/// Host of the streaming endpoint, port included. The port is part of the signed
/// `host` header, and the WebSocket client sends the same authority in `Host`.
pub(crate) fn streaming_host(region: &str) -> String {
    format!("transcribestreaming.{region}.amazonaws.com:8443")
}

/// The presigned `wss://` URL for one streaming session, signed at `now`.
///
/// Partial-result stabilization is on, at AWS's `low` level: it lets a live
/// segment settle sooner, and of the three levels `low` costs the least
/// accuracy, which is what matters for text that gets pasted.
pub(crate) fn streaming_url(
    config: &AmazonTranscribeConfig,
    sample_rate: u32,
    now: DateTime<Utc>,
) -> String {
    let host = streaming_host(&config.region);
    let mut params = vec![
        ("media-encoding".to_string(), "pcm".to_string()),
        ("sample-rate".to_string(), sample_rate.to_string()),
        (
            "enable-partial-results-stabilization".to_string(),
            "true".to_string(),
        ),
        ("partial-results-stability".to_string(), "low".to_string()),
    ];
    match &config.language {
        StreamingLanguage::Code(locale) => {
            params.push(("language-code".to_string(), locale.to_string()));
        }
        StreamingLanguage::Identify(candidates) => {
            params.push(("identify-language".to_string(), "true".to_string()));
            params.push(("language-options".to_string(), candidates.join(",")));
            if let Some(first) = candidates.first() {
                params.push(("preferred-language".to_string(), first.to_string()));
            }
        }
    }
    let signed = presign(&PresignRequest {
        method: "GET",
        host: &host,
        path: STREAMING_PATH,
        params,
        region: &config.region,
        service: SERVICE,
        credentials: &config.credentials,
        now,
        expires_secs: PRESIGN_EXPIRES_SECS,
        payload_hash: EMPTY_PAYLOAD_SHA256,
    });
    format!("wss://{host}{STREAMING_PATH}?{}", signed.query)
}

/// A request to presign with Signature Version 4, with `host` as the only signed
/// header (what a presigned URL carries).
struct PresignRequest<'a> {
    method: &'a str,
    host: &'a str,
    /// Already canonical: the paths signed here are plain ASCII.
    path: &'a str,
    params: Vec<(String, String)>,
    region: &'a str,
    service: &'a str,
    credentials: &'a AwsCredentials,
    now: DateTime<Utc>,
    expires_secs: u32,
    payload_hash: &'a str,
}

/// The intermediate steps are kept so the test can compare each against AWS's
/// published example, which is how a mismatch is found rather than just seen.
#[cfg_attr(not(test), allow(dead_code))]
struct Presigned {
    canonical_request: String,
    string_to_sign: String,
    signature: String,
    /// The full query string, `X-Amz-Signature` last.
    query: String,
}

fn presign(request: &PresignRequest) -> Presigned {
    let amz_date = request.now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = request.now.format("%Y%m%d").to_string();
    let scope = format!("{date}/{}/{}/aws4_request", request.region, request.service);

    let mut params = request.params.clone();
    params.push((
        "X-Amz-Algorithm".to_string(),
        "AWS4-HMAC-SHA256".to_string(),
    ));
    params.push((
        "X-Amz-Credential".to_string(),
        format!("{}/{scope}", request.credentials.access_key_id),
    ));
    params.push(("X-Amz-Date".to_string(), amz_date.clone()));
    params.push((
        "X-Amz-Expires".to_string(),
        request.expires_secs.to_string(),
    ));
    if let Some(token) = &request.credentials.session_token {
        params.push(("X-Amz-Security-Token".to_string(), token.clone()));
    }
    params.push(("X-Amz-SignedHeaders".to_string(), "host".to_string()));
    let query = canonical_query(&params);

    let canonical_request = format!(
        "{}\n{}\n{query}\nhost:{}\n\nhost\n{}",
        request.method, request.path, request.host, request.payload_hash
    );
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let key = signing_key(
        &request.credentials.secret_access_key,
        &date,
        request.region,
        request.service,
    );
    let signature = hex(&hmac_sha256(&key, string_to_sign.as_bytes()));
    let query = format!("{query}&X-Amz-Signature={signature}");
    Presigned {
        canonical_request,
        string_to_sign,
        signature,
        query,
    }
}

/// Names and values URI-encoded, then sorted by name and value, as SigV4
/// requires. The same string is the URL's query, so what is signed is exactly
/// what is sent.
fn canonical_query(params: &[(String, String)]) -> String {
    let mut encoded: Vec<(String, String)> = params
        .iter()
        .map(|(name, value)| (uri_encode(name), uri_encode(value)))
        .collect();
    encoded.sort();
    encoded
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// SigV4's `UriEncode`: everything but the RFC 3986 unreserved characters as
/// `%XX` with uppercase hex. `/` is encoded too, which is right for query
/// values (the credential scope) and is never needed in the paths signed here.
fn uri_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> [u8; 32] {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    // HMAC is defined for keys of every length (long ones are hashed first), so
    // this constructor cannot fail; the error type exists for fixed-key MACs.
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The 12-byte prelude: total length, headers length, and their CRC.
const PRELUDE_LEN: usize = 12;

/// Prelude plus the trailing message CRC.
const MESSAGE_OVERHEAD: usize = 16;

/// Far beyond anything Transcribe sends; a length past this is corruption, not
/// a message, and must not become an allocation.
const MAX_MESSAGE_LEN: usize = 16 * 1024 * 1024;

/// The headers every audio frame carries.
const AUDIO_EVENT_HEADERS: [(&str, &str); 3] = [
    (":content-type", "application/octet-stream"),
    (":event-type", "AudioEvent"),
    (":message-type", "event"),
];

/// One chunk of 16-bit little-endian PCM as an `AudioEvent` message.
///
/// An empty chunk is how a stream is ended: AWS sends the last transcripts and
/// then closes the socket itself. So it must never be sent mid-stream.
pub(crate) fn audio_event(pcm: &[u8]) -> Vec<u8> {
    encode_message(&AUDIO_EVENT_HEADERS, pcm)
}

/// Encode one event-stream message whose headers are all strings, the only kind
/// this client sends.
///
/// ```text
/// [total length u32][headers length u32][prelude CRC u32][headers][payload][message CRC u32]
/// ```
///
/// Each header is a one-byte name length, the name, the type byte (7, string), a
/// two-byte value length and the value. Both CRCs are CRC32 (the gzip one), and
/// every integer is big-endian.
pub(crate) fn encode_message(headers: &[(&str, &str)], payload: &[u8]) -> Vec<u8> {
    let mut header_bytes = Vec::new();
    for (name, value) in headers {
        debug_assert!(name.len() <= u8::MAX as usize && value.len() <= u16::MAX as usize);
        header_bytes.push(name.len() as u8);
        header_bytes.extend_from_slice(name.as_bytes());
        header_bytes.push(7);
        header_bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
        header_bytes.extend_from_slice(value.as_bytes());
    }
    let total = MESSAGE_OVERHEAD + header_bytes.len() + payload.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(&(header_bytes.len() as u32).to_be_bytes());
    let prelude_crc = crc32fast::hash(&out);
    out.extend_from_slice(&prelude_crc.to_be_bytes());
    out.extend_from_slice(&header_bytes);
    out.extend_from_slice(payload);
    let message_crc = crc32fast::hash(&out);
    out.extend_from_slice(&message_crc.to_be_bytes());
    out
}

/// A header value, by the type byte that precedes it on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HeaderValue {
    Bool(bool),
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Bytes(Vec<u8>),
    String(String),
    /// Milliseconds since the Unix epoch.
    Timestamp(i64),
    Uuid([u8; 16]),
}

/// One decoded event-stream message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EventMessage {
    pub headers: Vec<(String, HeaderValue)>,
    pub payload: Vec<u8>,
}

impl EventMessage {
    pub(crate) fn header_str(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .and_then(|(_, value)| match value {
                HeaderValue::String(text) => Some(text.as_str()),
                _ => None,
            })
    }
}

/// Decode every message in one WebSocket frame.
///
/// A frame normally holds exactly one, but nothing in the format forbids more,
/// so the decoder walks them by their length prefixes. Both checksums are
/// verified: a corrupt frame is an error, never a transcript.
pub(crate) fn decode_messages(mut bytes: &[u8]) -> Result<Vec<EventMessage>, String> {
    let mut messages = Vec::new();
    while !bytes.is_empty() {
        let (message, used) = decode_message(bytes)?;
        messages.push(message);
        bytes = &bytes[used..];
    }
    Ok(messages)
}

fn decode_message(bytes: &[u8]) -> Result<(EventMessage, usize), String> {
    if bytes.len() < MESSAGE_OVERHEAD {
        return Err("event-stream message is truncated".to_string());
    }
    let word =
        |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let total = word(0) as usize;
    let headers_len = word(4) as usize;
    if crc32fast::hash(&bytes[..8]) != word(8) {
        return Err("event-stream prelude checksum does not match".to_string());
    }
    if !(MESSAGE_OVERHEAD..=MAX_MESSAGE_LEN).contains(&total) || total > bytes.len() {
        return Err(format!(
            "event-stream message length {total} does not fit the {} bytes received",
            bytes.len()
        ));
    }
    if headers_len > total - MESSAGE_OVERHEAD {
        return Err("event-stream headers overrun the message".to_string());
    }
    if crc32fast::hash(&bytes[..total - 4]) != word(total - 4) {
        return Err("event-stream message checksum does not match".to_string());
    }
    let headers = parse_headers(&bytes[PRELUDE_LEN..PRELUDE_LEN + headers_len])?;
    let payload = bytes[PRELUDE_LEN + headers_len..total - 4].to_vec();
    Ok((EventMessage { headers, payload }, total))
}

/// A forward-only reader over header bytes.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        if self.bytes.len() < count {
            return Err("event-stream header is truncated".to_string());
        }
        let (head, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn sized(&mut self) -> Result<&'a [u8], String> {
        let len = u16::from_be_bytes(self.array()?) as usize;
        self.take(len)
    }
}

fn parse_headers(bytes: &[u8]) -> Result<Vec<(String, HeaderValue)>, String> {
    let mut reader = Reader { bytes };
    let mut headers = Vec::new();
    while !reader.bytes.is_empty() {
        let name_len = reader.array::<1>()?[0] as usize;
        let name = String::from_utf8(reader.take(name_len)?.to_vec())
            .map_err(|_| "event-stream header name is not UTF-8".to_string())?;
        let value = match reader.array::<1>()?[0] {
            0 => HeaderValue::Bool(true),
            1 => HeaderValue::Bool(false),
            2 => HeaderValue::Byte(i8::from_be_bytes(reader.array()?)),
            3 => HeaderValue::Short(i16::from_be_bytes(reader.array()?)),
            4 => HeaderValue::Int(i32::from_be_bytes(reader.array()?)),
            5 => HeaderValue::Long(i64::from_be_bytes(reader.array()?)),
            6 => HeaderValue::Bytes(reader.sized()?.to_vec()),
            7 => HeaderValue::String(
                String::from_utf8(reader.sized()?.to_vec())
                    .map_err(|_| format!("event-stream header {name} is not UTF-8"))?,
            ),
            8 => HeaderValue::Timestamp(i64::from_be_bytes(reader.array()?)),
            9 => HeaderValue::Uuid(reader.array()?),
            other => {
                return Err(format!(
                    "event-stream header {name} has unknown type {other}"
                ))
            }
        };
        headers.push((name, value));
    }
    Ok(headers)
}

/// One transcript result, as the stream session needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranscriptResult {
    pub text: String,
    /// A partial result is the running draft of the current segment; the final
    /// one (`IsPartial: false`) closes the segment.
    pub is_partial: bool,
    /// The locale AWS settled on, when it was asked to identify the language.
    pub language: Option<String>,
}

/// What one message from the server means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerMessage {
    /// Transcript results, in the order AWS listed them.
    Results(Vec<TranscriptResult>),
    /// A terminal error. AWS closes the socket right after it.
    Exception { kind: String, message: String },
    /// An event this client has no use for.
    Ignored,
}

/// Read a decoded message.
///
/// Transcripts arrive as `TranscriptEvent` events. Errors arrive two ways: as an
/// `exception` message, whose kind is in `:exception-type` (the WebSocket
/// documentation) or `:event-type` (the HTTP/2 one), with `{"Message": …}` as the
/// payload; or as an event-stream `error` message with `:error-code` and
/// `:error-message` headers.
pub(crate) fn interpret(message: &EventMessage) -> ServerMessage {
    match message.header_str(":message-type") {
        Some("event") => match message.header_str(":event-type") {
            Some("TranscriptEvent") => {
                ServerMessage::Results(parse_transcript_event(&message.payload))
            }
            _ => ServerMessage::Ignored,
        },
        Some("exception") => ServerMessage::Exception {
            kind: message
                .header_str(":exception-type")
                .or_else(|| message.header_str(":event-type"))
                .unwrap_or("Exception")
                .to_string(),
            message: error_body_message(&message.payload),
        },
        Some("error") => ServerMessage::Exception {
            kind: message
                .header_str(":error-code")
                .unwrap_or("Error")
                .to_string(),
            message: message
                .header_str(":error-message")
                .unwrap_or_default()
                .to_string(),
        },
        _ => ServerMessage::Ignored,
    }
}

/// The results of one `TranscriptEvent`. A payload that does not parse yields
/// none, which the session treats as a heartbeat; AWS sends events with an empty
/// `Results` list while nobody speaks.
fn parse_transcript_event(payload: &[u8]) -> Vec<TranscriptResult> {
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Event {
        #[serde(default)]
        transcript: Option<Transcript>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Transcript {
        #[serde(default)]
        results: Vec<ResultJson>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct ResultJson {
        #[serde(default)]
        alternatives: Vec<Alternative>,
        #[serde(default)]
        is_partial: bool,
        #[serde(default)]
        language_code: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Alternative {
        #[serde(default)]
        transcript: String,
    }
    let Ok(event) = serde_json::from_slice::<Event>(payload) else {
        return Vec::new();
    };
    event
        .transcript
        .map(|transcript| transcript.results)
        .unwrap_or_default()
        .into_iter()
        .map(|result| TranscriptResult {
            text: result
                .alternatives
                .into_iter()
                .next()
                .map(|alternative| alternative.transcript.trim().to_string())
                .unwrap_or_default(),
            is_partial: result.is_partial,
            language: result.language_code,
        })
        .collect()
}

/// The human part of an AWS error body: `{"Message": …}` (event stream) or
/// `{"message": …}` (the upgrade response), else the text itself, shortened.
fn error_body_message(body: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) {
        for field in ["Message", "message"] {
            if let Some(text) = value.get(field).and_then(|v| v.as_str()) {
                return text.trim().to_string();
            }
        }
    }
    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    let mut short: String = text.chars().take(300).collect();
    if text.chars().count() > 300 {
        short.push('…');
    }
    short
}

/// An AWS failure, in words a user can act on.
///
/// The kinds that matter are the ones with a fix on the user's side: a key AWS
/// does not recognise, a key without permission to stream, a clock too far off
/// for the signature, an account over its stream quota. AWS's own message is
/// kept in parentheses, because it is the part that names the specific
/// parameter or permission.
pub(crate) fn describe_failure(kind: &str, message: &str) -> String {
    let message = message.trim();
    let detail = if message.is_empty() {
        String::new()
    } else {
        format!(" ({message})")
    };
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("signature expired") || lowered.contains("request has expired") {
        return format!(
            "Amazon Transcribe refused the request because this computer's clock is off{detail}. \
             AWS rejects signed requests made more than five minutes from its own time."
        );
    }
    match kind {
        "UnrecognizedClientException"
        | "InvalidSignatureException"
        | "IncompleteSignatureException"
        | "SignatureDoesNotMatch"
        | "InvalidClientTokenId"
        | "ExpiredTokenException"
        | "ExpiredToken" => format!(
            "Amazon Transcribe did not accept the access key{detail}. Check the key ID and \
             secret, or paste fresh temporary credentials."
        ),
        "AccessDeniedException" | "AccessDenied" => format!(
            "This access key may not use Amazon Transcribe streaming{detail}. Its IAM user or \
             role needs the transcribe:StartStreamTranscriptionWebSocket permission."
        ),
        "LimitExceededException" | "ThrottlingException" => format!(
            "Amazon Transcribe is limiting this account{detail}: too many streams at once, or \
             one ran too long."
        ),
        "BadRequestException" => format!("Amazon Transcribe rejected the request{detail}."),
        "InternalFailureException" | "ServiceUnavailableException" => {
            format!("Amazon Transcribe had a problem on its side{detail}. Try again shortly.")
        }
        other => format!("Amazon Transcribe returned {other}{detail}."),
    }
}

/// A refused WebSocket upgrade. A bad signature, an unknown key or a missing
/// permission can be answered here, with an HTTP status, before any event-stream
/// exception is sent.
pub(crate) fn describe_handshake_failure(status: u16, body: &[u8]) -> String {
    let message = error_body_message(body);
    let kind = match status {
        401 => "UnrecognizedClientException",
        403 if message.to_ascii_lowercase().contains("not authorized") => "AccessDeniedException",
        403 => "UnrecognizedClientException",
        400 => "BadRequestException",
        429 => "LimitExceededException",
        500..=599 => "ServiceUnavailableException",
        _ => {
            return format!(
                "Amazon Transcribe refused the connection (HTTP {status}){}",
                if message.is_empty() {
                    String::new()
                } else {
                    format!(": {message}")
                }
            )
        }
    };
    describe_failure(kind, &message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use chrono::TimeZone;

    fn credentials(id: &str, secret: &str, token: Option<&str>) -> AwsCredentials {
        AwsCredentials {
            access_key_id: id.to_string(),
            secret_access_key: secret.to_string(),
            session_token: token.map(str::to_string),
        }
    }

    /// AWS's own worked example of a presigned URL ("Authenticating Requests:
    /// Using Query Parameters (AWS Signature Version 4)", Amazon S3 User Guide).
    /// Every intermediate step is compared, so a regression points at the step
    /// that broke rather than only at a different signature.
    #[test]
    fn presigning_reproduces_the_published_aws_example() {
        let creds = credentials(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            None,
        );
        let signed = presign(&PresignRequest {
            method: "GET",
            host: "examplebucket.s3.amazonaws.com",
            path: "/test.txt",
            params: Vec::new(),
            region: "us-east-1",
            service: "s3",
            credentials: &creds,
            now: Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap(),
            expires_secs: 86400,
            payload_hash: "UNSIGNED-PAYLOAD",
        });
        assert_eq!(
            signed.canonical_request,
            "GET\n\
             /test.txt\n\
             X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host\n\
             host:examplebucket.s3.amazonaws.com\n\
             \n\
             host\n\
             UNSIGNED-PAYLOAD"
        );
        assert_eq!(
            signed.string_to_sign,
            "AWS4-HMAC-SHA256\n\
             20130524T000000Z\n\
             20130524/us-east-1/s3/aws4_request\n\
             3bfa292879f6447bbcda7001decf97f4a54dc650c8942174ae0a9121cf58ad04"
        );
        assert_eq!(
            signed.signature,
            "aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        );
        assert_eq!(
            signed.query,
            "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host&X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        );
    }

    fn config(language: StreamingLanguage, token: Option<&str>) -> AmazonTranscribeConfig {
        AmazonTranscribeConfig {
            region: "eu-west-1".to_string(),
            credentials: credentials(
                "AKIAIOSFODNN7EXAMPLE",
                "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                token,
            ),
            language,
        }
    }

    fn query_of(url: &str) -> Vec<(String, String)> {
        url.split_once('?')
            .map(|(_, query)| query)
            .unwrap_or_default()
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn the_streaming_url_is_signed_for_the_port_8443_endpoint() {
        let now = Utc.with_ymd_and_hms(2026, 10, 13, 6, 30, 0).unwrap();
        let url = streaming_url(&config(StreamingLanguage::Code("en-US"), None), 16_000, now);
        assert!(url.starts_with(
            "wss://transcribestreaming.eu-west-1.amazonaws.com:8443/stream-transcription-websocket?"
        ));
        let query = query_of(&url);
        let get = |name: &str| {
            query
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(get("language-code"), Some("en-US"));
        assert_eq!(get("media-encoding"), Some("pcm"));
        assert_eq!(get("sample-rate"), Some("16000"));
        assert_eq!(get("enable-partial-results-stabilization"), Some("true"));
        assert_eq!(get("partial-results-stability"), Some("low"));
        assert_eq!(get("X-Amz-Expires"), Some("300"));
        assert_eq!(get("X-Amz-SignedHeaders"), Some("host"));
        assert_eq!(
            get("X-Amz-Credential"),
            Some("AKIAIOSFODNN7EXAMPLE%2F20261013%2Feu-west-1%2Ftranscribe%2Faws4_request")
        );
        assert!(get("X-Amz-Security-Token").is_none());
        // The signature is last; everything before it is the sorted canonical
        // query, so what was signed is byte-for-byte what is sent.
        let (signed_part, signature) = url
            .split_once('?')
            .unwrap()
            .1
            .rsplit_once("&X-Amz-Signature=")
            .unwrap();
        assert_eq!(signature.len(), 64);
        let names: Vec<&str> = signed_part
            .split('&')
            .map(|pair| pair.split('=').next().unwrap())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn a_session_token_is_signed_into_the_url() {
        let now = Utc.with_ymd_and_hms(2026, 10, 13, 6, 30, 0).unwrap();
        let token = "IQoJb3JpZ2luX2VjEMv//////////wEaCXVz+abc=";
        let with = streaming_url(
            &config(StreamingLanguage::Code("en-US"), Some(token)),
            16_000,
            now,
        );
        let without = streaming_url(&config(StreamingLanguage::Code("en-US"), None), 16_000, now);
        let query = query_of(&with);
        let sent = query
            .iter()
            .find(|(name, _)| name == "X-Amz-Security-Token")
            .map(|(_, value)| value.clone());
        assert_eq!(
            sent.as_deref(),
            Some("IQoJb3JpZ2luX2VjEMv%2F%2F%2F%2F%2F%2F%2F%2F%2F%2FwEaCXVz%2Babc%3D")
        );
        // Part of the signature, not appended after it.
        assert_ne!(
            with.rsplit_once("X-Amz-Signature=").unwrap().1,
            without.rsplit_once("X-Amz-Signature=").unwrap().1
        );
    }

    #[test]
    fn auto_detection_sends_candidates_and_a_preferred_language() {
        let now = Utc.with_ymd_and_hms(2026, 10, 13, 6, 30, 0).unwrap();
        let language = StreamingLanguage::Identify(vec!["de-DE", "en-US", "es-US"]);
        let url = streaming_url(&config(language, None), 16_000, now);
        assert!(url.contains("identify-language=true"));
        assert!(url.contains("language-options=de-DE%2Cen-US%2Ces-US"));
        assert!(url.contains("preferred-language=de-DE"));
        assert!(!url.contains("language-code="));
    }

    #[test]
    fn uri_encoding_follows_sigv4_rules() {
        assert_eq!(uri_encode("a-z_A.Z~09"), "a-z_A.Z~09");
        assert_eq!(uri_encode("a b"), "a%20b");
        assert_eq!(uri_encode("a/b+c=d,e"), "a%2Fb%2Bc%3Dd%2Ce");
        assert_eq!(uri_encode("é"), "%C3%A9");
    }

    #[test]
    fn credentials_are_read_from_the_composed_secret() {
        let plain = AwsCredentials::parse(
            " AKIAIOSFODNN7EXAMPLE : wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY ",
        )
        .expect("a key ID and secret should parse");
        assert_eq!(plain.access_key_id, "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(
            plain.secret_access_key,
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
        );
        assert!(plain.session_token.is_none());

        let temporary =
            AwsCredentials::parse("ASIAIOSFODNN7EXAMPLE:secret+/=:IQoJb3JpZ2luX2VjE+/token==")
                .expect("temporary credentials should parse");
        assert_eq!(
            temporary.session_token.as_deref(),
            Some("IQoJb3JpZ2luX2VjE+/token==")
        );
        // A trailing separator with nothing after it is not a token.
        assert!(AwsCredentials::parse("AKIAIOSFODNN7EXAMPLE:secret:")
            .unwrap()
            .session_token
            .is_none());

        for bad in [
            "",
            "AKIAIOSFODNN7EXAMPLE",
            "AKIAIOSFODNN7EXAMPLE:",
            ":secret",
            "short:secret",
            "AKIA-IOSFODNN7EXAMPLE:secret",
            "AKIAIOSFODNN7EXAMPLE:sec ret",
            "AKIAIOSFODNN7EXAMPLE:secret:tok en",
            "AKIAIOSFODNN7EXAMPLE:secret:token:extra",
        ] {
            assert!(
                AwsCredentials::parse(bad).is_none(),
                "{bad:?} should not parse"
            );
        }
    }

    #[test]
    fn a_region_is_read_from_a_name_or_a_pasted_endpoint() {
        for (raw, expected) in [
            ("us-east-1", Some("us-east-1")),
            (" EU-West-2 ", Some("eu-west-2")),
            ("us-gov-west-1", Some("us-gov-west-1")),
            ("transcribestreaming.ap-southeast-2.amazonaws.com", Some("ap-southeast-2")),
            (
                "wss://transcribestreaming.ca-central-1.amazonaws.com:8443/stream-transcription-websocket",
                Some("ca-central-1"),
            ),
            ("https://transcribestreaming.sa-east-1.amazonaws.com", Some("sa-east-1")),
            ("", None),
            ("useast1", None),
            ("us-east", None),
            ("us-east-x", None),
            ("transcribestreaming-fips.us-east-1.amazonaws.com", None),
            ("https://example.com", None),
        ] {
            assert_eq!(region_from(raw).as_deref(), expected, "for {raw:?}");
        }
        // Every region the picker offers is one the resolver accepts.
        for region in STREAMING_REGIONS {
            assert_eq!(region_from(region).as_deref(), Some(*region));
        }
    }

    #[test]
    fn a_chosen_language_becomes_the_full_aws_locale() {
        let code = |selected: &str| streaming_language(selected, "en", "us-east-1");
        assert_eq!(code("en"), Some(StreamingLanguage::Code("en-US")));
        assert_eq!(code("es"), Some(StreamingLanguage::Code("es-US")));
        assert_eq!(code("pt"), Some(StreamingLanguage::Code("pt-BR")));
        assert_eq!(code("zh-Hans"), Some(StreamingLanguage::Code("zh-CN")));
        assert_eq!(code("zh-Hant"), Some(StreamingLanguage::Code("zh-TW")));
        assert_eq!(code("yue"), Some(StreamingLanguage::Code("zh-HK")));
        assert_eq!(code("jw"), Some(StreamingLanguage::Code("jv-ID")));
        assert_eq!(code("ne"), Some(StreamingLanguage::Code("ne-NP")));
    }

    #[test]
    fn a_language_aws_cannot_stream_fails_instead_of_guessing() {
        for unsupported in ["la", "mi", "ur", "mn", "az", "haw", "nn", "yi", "xx"] {
            assert_eq!(
                streaming_language(unsupported, "en", "us-east-1"),
                None,
                "{unsupported} has no streaming model at AWS"
            );
        }
    }

    #[test]
    fn starred_languages_are_missing_in_four_regions_only() {
        // Nepali is starred; English is not.
        assert_eq!(
            streaming_language("ne", "en", "ap-northeast-1"),
            None,
            "Tokyo has no streaming for the starred languages"
        );
        assert_eq!(
            streaming_language("ne", "en", "ap-south-1"),
            Some(StreamingLanguage::Code("ne-NP"))
        );
        assert_eq!(
            streaming_language("en", "en", "ap-northeast-1"),
            Some(StreamingLanguage::Code("en-US"))
        );
    }

    #[test]
    fn auto_detection_offers_five_languages_with_the_interface_language_first() {
        let Some(StreamingLanguage::Identify(english)) =
            streaming_language("auto", "en", "us-east-1")
        else {
            panic!("auto should identify the language");
        };
        assert_eq!(english, vec!["en-US", "es-US", "fr-FR", "de-DE", "pt-BR"]);

        let Some(StreamingLanguage::Identify(german)) =
            streaming_language("", "de-DE", "us-east-1")
        else {
            panic!("an empty selection is auto too");
        };
        assert_eq!(german, vec!["de-DE", "en-US", "es-US", "fr-FR", "pt-BR"]);

        let Some(StreamingLanguage::Identify(taiwan)) =
            streaming_language("auto", "zh-TW", "us-east-1")
        else {
            panic!("auto should identify the language");
        };
        assert_eq!(taiwan[0], "zh-TW");
        // One dialect per language: zh-CN must not join zh-TW.
        assert!(!taiwan.contains(&"zh-CN"));

        // An interface language AWS lacks in this region is simply left out.
        let Some(StreamingLanguage::Identify(tokyo)) =
            streaming_language("auto", "tr", "ap-northeast-1")
        else {
            panic!("auto should identify the language");
        };
        assert_eq!(tokyo, vec!["en-US", "es-US", "fr-FR", "de-DE", "pt-BR"]);
    }

    #[test]
    fn audio_events_have_the_exact_documented_layout() {
        let frame = audio_event(&[1, 2, 3, 4]);
        // 16 bytes of overhead, 88 bytes of the three headers, 4 of payload.
        let headers_len = (1 + 13 + 1 + 2 + 24) + (1 + 11 + 1 + 2 + 10) + (1 + 13 + 1 + 2 + 5);
        assert_eq!(headers_len, 88);
        assert_eq!(frame.len(), 16 + headers_len + 4);
        assert_eq!(&frame[0..4], &(frame.len() as u32).to_be_bytes());
        assert_eq!(&frame[4..8], &(headers_len as u32).to_be_bytes());
        assert_eq!(&frame[12..26], b"\x0d:content-type");

        let decoded = decode_messages(&frame).expect("own frames should decode");
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].payload, vec![1, 2, 3, 4]);
        assert_eq!(decoded[0].header_str(":event-type"), Some("AudioEvent"));
        assert_eq!(decoded[0].header_str(":message-type"), Some("event"));
        assert_eq!(
            decoded[0].header_str(":content-type"),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn the_end_of_stream_event_is_an_empty_audio_event() {
        let end = audio_event(&[]);
        assert_eq!(end.len(), 16 + 88);
        let decoded = decode_messages(&end).unwrap();
        assert!(decoded[0].payload.is_empty());
        assert_eq!(decoded[0].header_str(":event-type"), Some("AudioEvent"));
    }

    /// A frame AWS publishes in "Setting up a streaming transcription"
    /// (HTTP/2): a signed empty message with a timestamp and a byte-array
    /// header. Decoding it proves the CRC32 variant and the two non-string
    /// header types against bytes AWS produced, not bytes this module produced.
    #[test]
    fn a_frame_published_by_aws_decodes_and_its_checksums_hold() {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(
                "AAAAUwAAAEP1RHpYBTpkYXRlCAAAAWiXUkMLEDpjaHVuay1zaWduYXR1cmUGACCt6Zy+uymwEK2SrLp/zVBI5eGn83jdBwCaRUBJA+eaDafqjqI=",
            )
            .unwrap();
        let messages = decode_messages(&bytes).expect("AWS's own frame should decode");
        assert_eq!(messages.len(), 1);
        let message = &messages[0];
        assert!(message.payload.is_empty());
        assert_eq!(message.headers[0].0, ":date");
        assert!(
            matches!(message.headers[0].1, HeaderValue::Timestamp(ms) if ms > 1_500_000_000_000)
        );
        assert_eq!(message.headers[1].0, ":chunk-signature");
        assert!(matches!(&message.headers[1].1, HeaderValue::Bytes(sig) if sig.len() == 32));
    }

    #[test]
    fn corrupt_or_truncated_frames_are_rejected() {
        let frame = audio_event(b"pcm");
        let mut flipped = frame.clone();
        let last = flipped.len() - 6;
        flipped[last] ^= 0xFF;
        assert!(decode_messages(&flipped).unwrap_err().contains("checksum"));

        let mut bad_prelude = frame.clone();
        bad_prelude[3] ^= 0x01;
        assert!(decode_messages(&bad_prelude)
            .unwrap_err()
            .contains("prelude"));

        assert!(decode_messages(&frame[..frame.len() - 1]).is_err());
        assert!(decode_messages(&frame[..10]).is_err());

        // Two messages in one frame are both read.
        let mut two = audio_event(b"a");
        two.extend_from_slice(&audio_event(b"b"));
        let decoded = decode_messages(&two).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[1].payload, b"b".to_vec());
    }

    fn server_event(event_type: &str, payload: &str) -> EventMessage {
        let bytes = encode_message(
            &[
                (":content-type", "application/json"),
                (":event-type", event_type),
                (":message-type", "event"),
            ],
            payload.as_bytes(),
        );
        decode_messages(&bytes).unwrap().remove(0)
    }

    #[test]
    fn transcript_events_become_partial_and_final_results() {
        let partial = server_event(
            "TranscriptEvent",
            r#"{"Transcript":{"Results":[{"Alternatives":[{"Transcript":"The Amazon is ","Items":[]}],"EndTime":1.5,"IsPartial":true,"ResultId":"a","StartTime":0.0}]}}"#,
        );
        assert_eq!(
            interpret(&partial),
            ServerMessage::Results(vec![TranscriptResult {
                text: "The Amazon is".to_string(),
                is_partial: true,
                language: None,
            }])
        );

        let mixed = server_event(
            "TranscriptEvent",
            r#"{"Transcript":{"Results":[
                {"Alternatives":[{"Transcript":"The Amazon is the largest rainforest."}],"IsPartial":false,"ResultId":"a","LanguageCode":"en-US"},
                {"Alternatives":[{"Transcript":"On the"}],"IsPartial":true,"ResultId":"b"}
            ]}}"#,
        );
        let ServerMessage::Results(results) = interpret(&mixed) else {
            panic!("a transcript event should yield results");
        };
        assert_eq!(results.len(), 2);
        assert!(!results[0].is_partial);
        assert_eq!(results[0].text, "The Amazon is the largest rainforest.");
        assert_eq!(results[0].language.as_deref(), Some("en-US"));
        assert!(results[1].is_partial);

        // Silence: AWS keeps sending events with no results.
        let empty = server_event("TranscriptEvent", r#"{"Transcript":{"Results":[]}}"#);
        assert_eq!(interpret(&empty), ServerMessage::Results(Vec::new()));
        assert_eq!(
            interpret(&server_event("SomethingNew", "{}")),
            ServerMessage::Ignored
        );
    }

    #[test]
    fn exceptions_are_read_from_either_header_name() {
        let websocket = decode_messages(&encode_message(
            &[
                (":content-type", "application/json"),
                (":exception-type", "BadRequestException"),
                (":message-type", "exception"),
            ],
            br#"{"Message":"Your request timed out because no new audio was received for 15 seconds."}"#,
        ))
        .unwrap()
        .remove(0);
        assert_eq!(
            interpret(&websocket),
            ServerMessage::Exception {
                kind: "BadRequestException".to_string(),
                message: "Your request timed out because no new audio was received for 15 seconds."
                    .to_string(),
            }
        );

        let http2 = decode_messages(&encode_message(
            &[
                (":event-type", "LimitExceededException"),
                (":message-type", "exception"),
            ],
            b"You have reached your limit of concurrent streams.",
        ))
        .unwrap()
        .remove(0);
        assert_eq!(
            interpret(&http2),
            ServerMessage::Exception {
                kind: "LimitExceededException".to_string(),
                message: "You have reached your limit of concurrent streams.".to_string(),
            }
        );

        let error = decode_messages(&encode_message(
            &[
                (":error-code", "InternalError"),
                (":error-message", "boom"),
                (":message-type", "error"),
            ],
            b"",
        ))
        .unwrap()
        .remove(0);
        assert_eq!(
            interpret(&error),
            ServerMessage::Exception {
                kind: "InternalError".to_string(),
                message: "boom".to_string(),
            }
        );
    }

    #[test]
    fn failures_say_what_to_fix() {
        let bad_key = describe_failure(
            "UnrecognizedClientException",
            "The security token included in the request is invalid.",
        );
        assert!(bad_key.contains("access key"), "{bad_key}");
        assert!(bad_key.contains("security token included"), "{bad_key}");

        let denied = describe_handshake_failure(
            403,
            br#"{"message":"User: arn:aws:iam::123456789012:user/dictation is not authorized to perform: transcribe:StartStreamTranscriptionWebSocket"}"#,
        );
        assert!(
            denied.contains("transcribe:StartStreamTranscriptionWebSocket permission"),
            "{denied}"
        );

        let clock = describe_handshake_failure(
            403,
            br#"{"message":"Signature expired: 20261013T060000Z is now earlier than 20261013T061000Z"}"#,
        );
        assert!(clock.contains("clock"), "{clock}");

        let forged = describe_handshake_failure(
            403,
            br#"{"message":"The request signature we calculated does not match the signature you provided."}"#,
        );
        assert!(forged.contains("did not accept the access key"), "{forged}");

        assert!(describe_failure("LimitExceededException", "").contains("limiting"));
        assert!(describe_failure("BadRequestException", "bad language").contains("bad language"));
        assert!(describe_handshake_failure(418, b"").contains("HTTP 418"));
    }
}
