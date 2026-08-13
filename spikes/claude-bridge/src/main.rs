//! P0 spike, Rust half: can the shipping language drive `claude.exe` over the
//! bidirectional stream-json control protocol on Windows?
//!
//! The Node probe (`probe.mjs`) answers "does the protocol behave as
//! documented". This answers the narrower question that actually matters for
//! SpeakoFlow: piped stdio to `claude.exe` from Rust on Windows, line-framed
//! JSON both directions, with no PTY, no tmux, and no extra runtime.
//!
//! Deliberately dependency-free (std only) so it compiles in seconds. The real
//! implementation would use tokio + serde_json, both already in the app.
//!
//! Usage:
//!   cargo run --quiet -- "<prompt>"

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    let prompt = std::env::args()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");
    let prompt = if prompt.trim().is_empty() {
        "Say hello in exactly five words. Do not use any tools.".to_string()
    } else {
        prompt
    };

    let cwd = std::env::current_dir().unwrap().join("sandbox");
    std::fs::create_dir_all(&cwd).unwrap();

    let started = Instant::now();
    let log = |t: &Instant, tag: &str, msg: &str| {
        println!("[{:>6}ms] {:<22} {}", t.elapsed().as_millis(), tag, msg);
    };

    log(&started, "SPAWN", "claude -p --input-format stream-json ...");

    let mut child = Command::new("claude")
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-mode",
            "default",
        ])
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn claude — is it on PATH?");

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // Line-framed reader on its own thread, mirroring how the real manager
    // would own one task per session.
    let (tx, rx) = mpsc::channel::<String>();
    let tx_err = tx.clone();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx_err.send(format!("__STDERR__{line}"));
        }
    });

    // Send one user turn. Hand-built JSON so the spike stays dependency-free;
    // the escaping below is only adequate for a probe.
    let escaped = prompt.replace('\\', "\\\\").replace('"', "\\\"");
    let msg = format!(
        r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"{escaped}"}}]}},"parent_tool_use_id":null,"session_id":""}}"#
    );
    log(&started, "-> user message", &prompt);
    stdin.write_all(msg.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();

    let mut saw_init = false;
    let mut saw_result = false;
    let mut session_id = String::new();
    let mut first_token_ms: Option<u128> = None;
    let mut deltas = 0usize;
    let mut control_requests = 0usize;

    // Crude field pluck; a probe does not need a JSON parser.
    fn field(line: &str, key: &str) -> Option<String> {
        let needle = format!("\"{key}\":\"");
        let start = line.find(&needle)? + needle.len();
        let rest = &line[start..];
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    }

    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(line) => {
                if let Some(err) = line.strip_prefix("__STDERR__") {
                    log(&started, "STDERR", err);
                    continue;
                }
                let ty = field(&line, "type").unwrap_or_default();
                match ty.as_str() {
                    "system" => {
                        if field(&line, "subtype").as_deref() == Some("init") {
                            saw_init = true;
                            session_id = field(&line, "session_id").unwrap_or_default();
                            let model = field(&line, "model").unwrap_or_default();
                            log(
                                &started,
                                "<= system/init",
                                &format!("session_id={session_id} model={model}"),
                            );
                        }
                    }
                    "stream_event" => {
                        if line.contains("content_block_delta") {
                            deltas += 1;
                            if first_token_ms.is_none() {
                                first_token_ms = Some(started.elapsed().as_millis());
                                log(
                                    &started,
                                    "<= FIRST TOKEN",
                                    &format!("{}ms", first_token_ms.unwrap()),
                                );
                            }
                        }
                    }
                    "assistant" => {
                        let preview: String = line.chars().take(200).collect();
                        log(&started, "<= assistant", &preview);
                    }
                    "control_request" => {
                        control_requests += 1;
                        log(&started, "<= CONTROL_REQUEST", &line);
                    }
                    "result" => {
                        saw_result = true;
                        let preview: String = line.chars().take(240).collect();
                        log(&started, "<= RESULT", &preview);
                        break;
                    }
                    other => log(&started, &format!("<= {other}"), ""),
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    drop(stdin);
    let status = child.wait().ok();

    println!("\n================ RUST SPIKE RESULT ================");
    println!("spawned claude.exe from Rust : YES");
    println!("piped stdio, line-framed JSON: {}", if saw_init { "YES" } else { "NO" });
    println!("session_id captured          : {}", if session_id.is_empty() { "NO".into() } else { session_id.clone() });
    println!("token deltas received        : {deltas}");
    println!("first token                  : {}", first_token_ms.map(|m| format!("{m}ms")).unwrap_or_else(|| "n/a".into()));
    println!("control_requests received    : {control_requests}");
    println!("turn completed (result)      : {}", if saw_result { "YES" } else { "NO" });
    println!("exit status                  : {status:?}");
    println!("===================================================");
}
