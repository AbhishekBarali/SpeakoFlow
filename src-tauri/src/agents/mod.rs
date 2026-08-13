//! Managed coding-agent sessions.
//!
//! SpeakoFlow spawns a coding-agent CLI (Claude Code for now) and drives it over
//! its bidirectional `stream-json` control protocol, so the assistant can answer
//! "what's happening?" out loud, stop a session, and answer the agent's own
//! permission prompts without the user tabbing to a terminal.
//!
//! Why a protocol and not a terminal: the popular parallel-agent managers spawn
//! the agent's TUI in a pseudo-terminal and pattern-match the *rendered screen*
//! to guess whether it is busy or blocked. That breaks whenever the agent's UI
//! changes, and it needs tmux, which does not exist on Windows. The CLI already
//! emits structured events and accepts structured commands; we use those.
//!
//! Design notes that matter:
//!
//! * **The digest, not the transcript.** A status question must be answerable in
//!   about 200 tokens with zero file I/O, because the whole point is a fast
//!   spoken answer. Each session keeps a small rolling summary
//!   ([`SessionState`]) that the reader thread updates as events arrive; the
//!   full transcript is never handed to the assistant model.
//! * **Blocking I/O on a dedicated thread per session.** `std::process` plus one
//!   reader thread mirrors the validated spike exactly and needs no new
//!   dependencies. Sessions are counted in single digits, so a thread each is
//!   cheaper than the churn of adding tokio's process feature.
//! * **Permissions are opt-in.** Without `--permission-prompt-tool stdio` *and*
//!   an `initialize` control request, Claude Code silently auto-denies every
//!   prompt and only reports it after the fact. Both are sent below.
//! * **Nothing is auto-approved.** A pending approval parks the session in
//!   [`AgentStatus::WaitingApproval`] until a human answers, and commands that
//!   look destructive refuse to be approved by voice at all.

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter};

mod env;

/// Environment override for the model handed to the agent CLI.
///
/// Deliberately an environment variable rather than a setting for now: the agent
/// feature is pre-release, and a real settings field would ripple into the
/// generated frontend bindings before the shape has settled. When unset the
/// agent uses whatever its own config selects.
const MODEL_ENV: &str = "SPEAKOFLOW_AGENT_MODEL";

/// Cap on how much of an assistant line we keep for the spoken summary.
const LINE_BUDGET: usize = 220;

/// Cap on remembered file paths per session, newest wins.
const FILES_BUDGET: usize = 40;

/// Where a session is, in the only terms a user cares about out loud.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AgentStatus {
    /// Process spawned, handshake not finished.
    Starting,
    /// Actively thinking or running tools.
    Working,
    /// Blocked on a human decision. The one status worth interrupting someone for.
    WaitingApproval,
    /// Turn finished successfully; the session is still alive for follow-ups.
    Idle,
    /// The turn ended in an error reported by the agent.
    Failed,
    /// Stopped by the user.
    Cancelled,
    /// Handed to a terminal, which now owns it.
    HandedOff,
    /// The process exited.
    Ended,
}

impl AgentStatus {
    /// Wording used in spoken and on-screen summaries.
    fn label(self) -> &'static str {
        match self {
            AgentStatus::Starting => "starting",
            AgentStatus::Working => "working",
            AgentStatus::WaitingApproval => "waiting for your approval",
            AgentStatus::Idle => "done, waiting for you",
            AgentStatus::Failed => "failed",
            AgentStatus::Cancelled => "stopped",
            AgentStatus::HandedOff => "handed to a terminal",
            AgentStatus::Ended => "closed",
        }
    }

    /// Whether the session can still accept input.
    fn is_live(self) -> bool {
        !matches!(self, AgentStatus::Ended | AgentStatus::HandedOff)
    }
}

/// A permission prompt the agent is blocked on.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    /// Correlation id required on the control response.
    pub request_id: String,
    pub tool_name: String,
    /// One line describing what it wants to do, safe to read aloud.
    pub detail: String,
    /// True when the action looks destructive enough that a voice "yes" must not
    /// be enough on its own.
    pub high_risk: bool,
}

/// The rolling per-session digest. Everything the assistant needs to answer a
/// status question, and nothing else.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionView {
    /// Short stable handle used by voice ("stop session 2").
    pub id: String,
    /// The agent CLI's own session id, once known, for `--resume`.
    pub agent_session_id: Option<String>,
    /// Human name, defaulting to the working directory's folder name.
    pub label: String,
    pub cwd: String,
    pub model: Option<String>,
    pub status: AgentStatus,
    pub elapsed_secs: u64,
    pub last_tool: Option<String>,
    pub last_line: Option<String>,
    pub files_touched: Vec<String>,
    pub tool_calls: u32,
    pub cost_usd: f64,
    pub pending: Option<PendingApproval>,
    /// The last tool failure, cleared as soon as a tool succeeds again. Separate
    /// from `error`, which is about the turn as a whole: a model that malforms a
    /// tool call and retries is not a failed session, but a session that looks
    /// frozen for ten seconds with no explanation is a bad experience.
    pub tool_error: Option<String>,
    pub error: Option<String>,
    /// The prompt that started the session, trimmed.
    pub task: String,
}

/// Internal mutable state, kept behind a mutex and updated by the reader thread.
#[derive(Debug)]
struct SessionState {
    id: String,
    agent_session_id: Option<String>,
    label: String,
    cwd: String,
    model: Option<String>,
    status: AgentStatus,
    started: Instant,
    last_tool: Option<String>,
    last_line: Option<String>,
    files_touched: Vec<String>,
    tool_calls: u32,
    cost_usd: f64,
    pending: Option<PendingApproval>,
    tool_error: Option<String>,
    error: Option<String>,
    task: String,
}

impl SessionState {
    fn view(&self) -> AgentSessionView {
        AgentSessionView {
            id: self.id.clone(),
            agent_session_id: self.agent_session_id.clone(),
            label: self.label.clone(),
            cwd: self.cwd.clone(),
            model: self.model.clone(),
            status: self.status,
            elapsed_secs: self.started.elapsed().as_secs(),
            last_tool: self.last_tool.clone(),
            last_line: self.last_line.clone(),
            files_touched: self.files_touched.clone(),
            tool_calls: self.tool_calls,
            cost_usd: self.cost_usd,
            pending: self.pending.clone(),
            tool_error: self.tool_error.clone(),
            error: self.error.clone(),
            task: self.task.clone(),
        }
    }
}

/// One live session: shared state plus the pipes needed to talk to it.
struct Session {
    state: Arc<Mutex<SessionState>>,
    /// Held separately so the reader thread never blocks a writer.
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    child: Arc<Mutex<Child>>,
}

/// The write end and process handle of one session.
type SessionPipes = (Arc<Mutex<Option<ChildStdin>>>, Arc<Mutex<Child>>);

/// Tauri-managed state holding every session SpeakoFlow started.
pub struct AgentManager {
    sessions: Mutex<Vec<Session>>,
    counter: AtomicU64,
}

impl Default for AgentManager {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(Vec::new()),
            counter: AtomicU64::new(0),
        }
    }
}

impl AgentManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new agent session in `cwd` and send `prompt` as its first turn.
    ///
    /// Returns the short session id. The call does not wait for the agent to
    /// finish — that is the entire point.
    pub fn start(
        &self,
        app: &AppHandle,
        cwd: &str,
        prompt: &str,
        label: Option<String>,
        model: Option<String>,
    ) -> Result<String, String> {
        let dir = std::path::Path::new(cwd);
        if !dir.is_dir() {
            return Err(format!("`{}` is not a folder that exists.", cwd));
        }
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err("The agent needs a task to work on.".to_string());
        }

        let model = model.or_else(|| env::resolve_var(MODEL_ENV)).and_then(|m| {
            let m = m.trim().to_string();
            if m.is_empty() {
                None
            } else {
                Some(m)
            }
        });

        // Resolved to an absolute path, and handed an explicit environment. A
        // GUI process's inherited `PATH` and credentials are not dependable: the
        // app may have been launched before the user configured their provider,
        // and on Windows a running process never sees a later `setx` — closing
        // the window only hides SpeakoFlow to the tray, so a stale environment
        // can outlive several apparent restarts.
        let binary = env::resolve_claude()?;
        let forwarded = env::forwarded_vars();
        if !forwarded.is_empty() {
            // Names only — these are credentials.
            log::info!(
                "Forwarding {} provider variable(s) from the user environment: {}",
                forwarded.len(),
                forwarded
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        log::info!("Starting agent session with {}", binary.display());

        let mut command = Command::new(&binary);
        command
            .arg("-p")
            .args(["--input-format", "stream-json"])
            .args(["--output-format", "stream-json"])
            .arg("--verbose")
            .arg("--include-partial-messages")
            // Surfaces tool results — including failures — on stdout. Without it
            // a model that malforms a tool call and retries is indistinguishable
            // from one making progress.
            .arg("--replay-user-messages")
            // Route permission prompts to us instead of letting the CLI
            // auto-deny them. Undocumented in `--help`, but it is how the
            // official SDKs do it.
            .args(["--permission-prompt-tool", "stdio"])
            .args(["--permission-mode", "default"])
            .current_dir(dir)
            .env("PATH", env::effective_path())
            .envs(env::forwarded_vars())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(model) = &model {
            command.args(["--model", model.as_str()]);
        }
        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW: without it every session flashes a console.
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }

        let mut child = command.spawn().map_err(|e| {
            format!(
                "Could not start the Claude Code CLI ({}). Make sure `claude` is installed and on PATH.",
                e
            )
        })?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let short_id = format!("{}", self.counter.fetch_add(1, Ordering::Relaxed) + 1);
        let label = label
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| {
                dir.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "agent".to_string())
            });

        let state = Arc::new(Mutex::new(SessionState {
            id: short_id.clone(),
            agent_session_id: None,
            label,
            cwd: dir.to_string_lossy().to_string(),
            model,
            status: AgentStatus::Starting,
            started: Instant::now(),
            last_tool: None,
            last_line: None,
            files_touched: Vec::new(),
            tool_calls: 0,
            cost_usd: 0.0,
            pending: None,
            tool_error: None,
            error: None,
            task: truncate(prompt, LINE_BUDGET),
        }));
        let stdin = Arc::new(Mutex::new(stdin));

        // Handshake, then the first turn. Both go out before the reader starts
        // so the agent is never waiting on us.
        write_line(
            &stdin,
            &json!({
                "type": "control_request",
                "request_id": "init_1",
                "request": {
                    "subtype": "initialize",
                    "capabilities": { "canUseTool": true },
                    "canUseTool": true,
                    "hooks": {}
                }
            }),
        )?;
        write_line(&stdin, &user_message(prompt))?;

        // Reader thread: the only writer of session state.
        if let Some(stdout) = stdout {
            let state_c = Arc::clone(&state);
            let app_c = app.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let before = state_c.lock().unwrap().status;
                    let changed = match serde_json::from_str::<Value>(&line) {
                        Ok(event) => apply_event(&state_c, &event),
                        Err(_) => false,
                    };
                    if changed {
                        let view = state_c.lock().unwrap().view();
                        if view.status != before {
                            announce(&app_c, before, &view);
                        }
                        let _ = app_c.emit("agent-session-update", view);
                    }
                }
                // stdout closed: the process is gone.
                {
                    let mut s = state_c.lock().unwrap();
                    if s.status.is_live() {
                        s.status = AgentStatus::Ended;
                        s.pending = None;
                    }
                }
                emit_changed(&app_c, &state_c);
            });
        }

        // Drain stderr so a chatty CLI can never fill its pipe and stall.
        if let Some(stderr) = stderr {
            let state_c = Arc::clone(&state);
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    log::debug!("agent stderr: {}", line);
                    let mut s = state_c.lock().unwrap();
                    s.error = Some(truncate(&line, LINE_BUDGET));
                }
            });
        }

        self.sessions.lock().unwrap().push(Session {
            state,
            stdin,
            child: Arc::new(Mutex::new(child)),
        });
        let _ = app.emit("agent-sessions-changed", self.views());
        Ok(short_id)
    }

    /// Every session, newest last.
    pub fn views(&self) -> Vec<AgentSessionView> {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.state.lock().unwrap().view())
            .collect()
    }

    /// Resolve a spoken reference to one session.
    ///
    /// Voice never produces an exact id, so an exact match on the short id is
    /// tried first, then a case-insensitive substring of the label, folder, or
    /// the agent's own session id. Ambiguous input is an error rather than a
    /// guess, because the actions behind this include "stop it".
    fn resolve(&self, needle: &str) -> Result<Arc<Mutex<SessionState>>, String> {
        let needle = needle.trim().to_lowercase();
        let sessions = self.sessions.lock().unwrap();
        if sessions.is_empty() {
            return Err("There are no agent sessions running.".to_string());
        }
        if needle.is_empty() {
            // A single session needs no disambiguation.
            if sessions.len() == 1 {
                return Ok(Arc::clone(&sessions[0].state));
            }
            return Err("Which session? There is more than one.".to_string());
        }

        let mut matches: Vec<Arc<Mutex<SessionState>>> = Vec::new();
        for session in sessions.iter() {
            let s = session.state.lock().unwrap();
            if s.id == needle {
                return Ok(Arc::clone(&session.state));
            }
            let haystack = format!(
                "{} {} {}",
                s.label.to_lowercase(),
                s.cwd.to_lowercase(),
                s.agent_session_id
                    .clone()
                    .unwrap_or_default()
                    .to_lowercase()
            );
            if haystack.contains(&needle) {
                matches.push(Arc::clone(&session.state));
            }
        }
        match matches.len() {
            0 => Err(format!("No agent session matches \"{}\".", needle)),
            1 => Ok(matches.remove(0)),
            n => Err(format!(
                "\"{}\" matches {} sessions — say the number instead.",
                needle, n
            )),
        }
    }

    /// The pipes for a session, found by short id.
    fn pipes(&self, id: &str) -> Option<SessionPipes> {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.state.lock().unwrap().id == id)
            .map(|s| (Arc::clone(&s.stdin), Arc::clone(&s.child)))
    }

    /// Compact multi-session digest for the assistant. Pre-rendered on purpose:
    /// answering a status question must not cost a file read.
    pub fn summary_block(&self) -> String {
        let views = self.views();
        if views.is_empty() {
            return "No coding-agent sessions have been started.".to_string();
        }
        let mut out = String::new();
        let working = views
            .iter()
            .filter(|v| v.status == AgentStatus::Working)
            .count();
        let blocked = views
            .iter()
            .filter(|v| v.status == AgentStatus::WaitingApproval)
            .count();
        let done = views
            .iter()
            .filter(|v| matches!(v.status, AgentStatus::Idle))
            .count();
        out.push_str(&format!(
            "{} session(s): {} working, {} waiting on you, {} finished.\n",
            views.len(),
            working,
            blocked,
            done
        ));
        for v in &views {
            out.push_str(&format!(
                "[{}] {} — {} ({})",
                v.id,
                v.label,
                v.status.label(),
                format_duration(v.elapsed_secs)
            ));
            if let Some(pending) = &v.pending {
                out.push_str(&format!(
                    " · needs approval: {}{}",
                    pending.detail,
                    if pending.high_risk {
                        " [HIGH RISK]"
                    } else {
                        ""
                    }
                ));
            } else if let Some(tool) = &v.last_tool {
                out.push_str(&format!(" · last tool: {}", tool));
            }
            if let Some(tool_error) = &v.tool_error {
                out.push_str(&format!(" · a tool call failed: {}", tool_error));
            }
            if !v.files_touched.is_empty() {
                out.push_str(&format!(" · {} file(s) changed", v.files_touched.len()));
            }
            if let Some(line) = &v.last_line {
                out.push_str(&format!("\n    says: {}", line));
            }
            out.push('\n');
        }
        out
    }

    /// Longer digest for one session.
    pub fn detail_block(&self, needle: &str) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let v = state.lock().unwrap().view();
        let mut out = format!(
            "Session [{}] {}\nstatus: {} for {}\nfolder: {}\ntask: {}\n",
            v.id,
            v.label,
            v.status.label(),
            format_duration(v.elapsed_secs),
            v.cwd,
            v.task
        );
        if let Some(model) = &v.model {
            out.push_str(&format!("model: {}\n", model));
        }
        out.push_str(&format!(
            "tool calls: {} · cost so far: ${:.4}\n",
            v.tool_calls, v.cost_usd
        ));
        if !v.files_touched.is_empty() {
            out.push_str(&format!(
                "files touched ({}): {}\n",
                v.files_touched.len(),
                v.files_touched.join(", ")
            ));
        }
        if let Some(pending) = &v.pending {
            out.push_str(&format!(
                "WAITING FOR APPROVAL — {} wants to: {}{}\n",
                pending.tool_name,
                pending.detail,
                if pending.high_risk {
                    "\nThis looks destructive. It cannot be approved by voice; the user must confirm it in the app."
                } else {
                    ""
                }
            ));
        }
        if let Some(line) = &v.last_line {
            out.push_str(&format!("last message: {}\n", line));
        }
        if let Some(tool_error) = &v.tool_error {
            out.push_str(&format!("last tool failure: {}\n", tool_error));
        }
        if let Some(err) = &v.error {
            out.push_str(&format!("last error: {}\n", err));
        }
        Ok(out)
    }

    /// Send another instruction to a live session.
    pub fn send(&self, app: &AppHandle, needle: &str, text: &str) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label, live) = {
            let s = state.lock().unwrap();
            (s.id.clone(), s.label.clone(), s.status.is_live())
        };
        if !live {
            return Err(format!("Session [{}] {} has closed.", id, label));
        }
        let (stdin, _) = self
            .pipes(&id)
            .ok_or_else(|| "That session is no longer available.".to_string())?;
        write_line(&stdin, &user_message(text))?;
        {
            let mut s = state.lock().unwrap();
            s.status = AgentStatus::Working;
        }
        emit_changed(app, &state);
        Ok(format!("Sent to [{}] {}.", id, label))
    }

    /// Answer a pending permission prompt.
    ///
    /// `allow` alone is not enough for an action classified as high risk:
    /// approving a destructive command on a possibly-misheard "yes" is the one
    /// failure mode that would destroy trust in the feature. Those need
    /// `force`, which the voice path never sets.
    pub fn answer_permission(
        &self,
        app: &AppHandle,
        needle: &str,
        allow: bool,
        force: bool,
    ) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label, pending) = {
            let s = state.lock().unwrap();
            (s.id.clone(), s.label.clone(), s.pending.clone())
        };
        let pending =
            pending.ok_or_else(|| format!("Session [{}] {} is not waiting on you.", id, label))?;
        if allow && pending.high_risk && !force {
            return Err(format!(
                "That action looks destructive ({}). It can't be approved by voice — confirm it in the app instead.",
                pending.detail
            ));
        }

        let (stdin, _) = self
            .pipes(&id)
            .ok_or_else(|| "That session is no longer available.".to_string())?;
        let response = if allow {
            json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": pending.request_id,
                    "response": { "behavior": "allow" }
                }
            })
        } else {
            json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": pending.request_id,
                    "response": {
                        "behavior": "deny",
                        "message": "Denied by the user via SpeakoFlow."
                    }
                }
            })
        };
        write_line(&stdin, &response)?;
        {
            let mut s = state.lock().unwrap();
            s.pending = None;
            s.status = AgentStatus::Working;
        }
        emit_changed(app, &state);
        Ok(format!(
            "{} {} in [{}] {}.",
            if allow { "Approved" } else { "Denied" },
            pending.tool_name,
            id,
            label
        ))
    }

    /// Stop whatever a session is doing right now, leaving it alive for a new
    /// instruction.
    pub fn cancel(&self, app: &AppHandle, needle: &str) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label, pending, status) = {
            let s = state.lock().unwrap();
            (s.id.clone(), s.label.clone(), s.pending.clone(), s.status)
        };
        // Nothing is running, so an interrupt would just fail against a closed
        // pipe. Say so plainly instead.
        if !matches!(
            status,
            AgentStatus::Starting | AgentStatus::Working | AgentStatus::WaitingApproval
        ) {
            return Err(format!(
                "Session [{}] {} isn't running — it's already {}.",
                id,
                label,
                status.label()
            ));
        }
        let (stdin, _) = self
            .pipes(&id)
            .ok_or_else(|| "That session is no longer available.".to_string())?;
        // A session parked on a permission prompt has to be released before an
        // interrupt means anything: it is not running, it is waiting.
        if let Some(pending) = pending {
            let _ = write_line(
                &stdin,
                &json!({
                    "type": "control_response",
                    "response": {
                        "subtype": "success",
                        "request_id": pending.request_id,
                        "response": { "behavior": "deny", "message": "Stopped by the user." }
                    }
                }),
            );
        }
        write_line(
            &stdin,
            &json!({
                "type": "control_request",
                "request_id": format!("int_{}", now_millis()),
                "request": { "subtype": "interrupt" }
            }),
        )?;
        {
            let mut s = state.lock().unwrap();
            s.pending = None;
            s.status = AgentStatus::Cancelled;
        }
        emit_changed(app, &state);
        Ok(format!("Stopped [{}] {}.", id, label))
    }

    /// End a session for good and drop it from the list.
    ///
    /// Removal is the point: the previous version only marked the row "closed",
    /// which looked like the button had done nothing when the session had already
    /// exited on its own. A finished-or-failed row stays until the user dismisses
    /// it here, so its error text is still readable.
    pub fn close(&self, app: &AppHandle, needle: &str) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label) = {
            let s = state.lock().unwrap();
            (s.id.clone(), s.label.clone())
        };
        {
            let mut sessions = self.sessions.lock().unwrap();
            if let Some(index) = sessions
                .iter()
                .position(|s| s.state.lock().unwrap().id == id)
            {
                let session = sessions.remove(index);
                session.stdin.lock().unwrap().take();
                let _ = session.child.lock().unwrap().kill();
            }
        }
        {
            let mut s = state.lock().unwrap();
            s.pending = None;
            s.status = AgentStatus::Ended;
        }
        // The panel refetches the whole list on this event, so the row goes away.
        emit_changed(app, &state);
        Ok(format!("Closed [{}] {}.", id, label))
    }

    /// Open a session's working folder in the OS file manager.
    pub fn open_folder(&self, app: &AppHandle, needle: &str) -> Result<String, String> {
        use tauri_plugin_opener::OpenerExt;
        let state = self.resolve(needle)?;
        let (id, label, cwd) = {
            let s = state.lock().unwrap();
            (s.id.clone(), s.label.clone(), s.cwd.clone())
        };
        app.opener()
            .open_path(cwd.clone(), None::<String>)
            .map_err(|e| format!("Could not open {}: {}", cwd, e))?;
        Ok(format!("Opened {} for [{}] {}.", cwd, id, label))
    }

    /// Hand a session over to a real terminal, with its history intact.
    ///
    /// This exists because a protocol-driven session is headless: there is no
    /// terminal to look at, which is fine until the user wants to take over and
    /// keep typing. The agent CLI persists its own transcript, so
    /// `claude --resume <session id>` in the same folder continues exactly where
    /// SpeakoFlow left off.
    ///
    /// Our own child is closed first, deliberately. Two processes driving one
    /// session would race over the same transcript, and a half-owned session is
    /// worse than a clean handover.
    pub fn resume_in_terminal(&self, app: &AppHandle, needle: &str) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label, cwd, agent_session_id, pending) = {
            let s = state.lock().unwrap();
            (
                s.id.clone(),
                s.label.clone(),
                s.cwd.clone(),
                s.agent_session_id.clone(),
                s.pending.clone(),
            )
        };
        let agent_session_id = agent_session_id.ok_or_else(|| {
            format!(
                "Session [{}] {} never got far enough to have a resumable id.",
                id, label
            )
        })?;

        // Release a pending prompt before handing over, so the resumed session
        // does not inherit a decision nobody answered.
        let dropped_pending = pending.is_some();
        if let Some((stdin, _)) = self.pipes(&id) {
            if let Some(pending) = pending {
                let _ = write_line(
                    &stdin,
                    &json!({
                        "type": "control_response",
                        "response": {
                            "subtype": "success",
                            "request_id": pending.request_id,
                            "response": {
                                "behavior": "deny",
                                "message": "Handed over to a terminal; ask again there."
                            }
                        }
                    }),
                );
            }
        }
        // Kill our child but keep the row: the previous version removed it, which
        // looked like the session had vanished rather than moved.
        {
            let sessions = self.sessions.lock().unwrap();
            if let Some(session) = sessions.iter().find(|s| s.state.lock().unwrap().id == id) {
                session.stdin.lock().unwrap().take();
                let _ = session.child.lock().unwrap().kill();
            }
        }
        {
            let mut s = state.lock().unwrap();
            s.pending = None;
            s.status = AgentStatus::HandedOff;
        }
        if let Err(e) = spawn_resume_terminal(&cwd, &agent_session_id) {
            // The terminal is the whole point, so a failure here is worth
            // reporting even though the session is already released.
            emit_changed(app, &state);
            return Err(e);
        }
        emit_changed(app, &state);
        let mut message = format!(
            "Handed [{}] {} to a terminal — it resumed with its full history, and SpeakoFlow is no longer driving it.",
            id, label
        );
        if dropped_pending {
            message.push_str(
                " The action it was waiting on was not applied; it will ask again there.",
            );
        }
        Ok(message)
    }

    /// Kill every session. Called on app shutdown so we never orphan a CLI.
    pub fn shutdown(&self) {
        let sessions = self.sessions.lock().unwrap();
        for session in sessions.iter() {
            session.stdin.lock().unwrap().take();
            let _ = session.child.lock().unwrap().kill();
        }
    }
}

/// Open a visible terminal that resumes `session_id` in `cwd`.
///
/// Everything here is absolute and explicit. The first attempt at this feature
/// launched `claude` by name with an inherited environment, and the terminal that
/// opened reported both `'claude' is not recognized` *and* `'DOSKEY' is not
/// recognized` — the second of which lives in System32, so the child had no
/// usable `PATH` at all. A handed-off terminal has to work on the first try, so
/// the CLI path, the shell path, and `PATH` itself are all supplied here.
fn spawn_resume_terminal(cwd: &str, session_id: &str) -> Result<(), String> {
    let binary = env::resolve_claude()?;
    let claude = binary.display().to_string();
    let path = env::effective_path();
    let forwarded = env::forwarded_vars();
    // Quoted, because an install path can contain spaces.
    let manual = format!("\"{}\" --resume {}", claude, session_id);

    #[cfg(windows)]
    {
        use std::path::PathBuf;
        let root = std::env::var("SYSTEMROOT").unwrap_or_else(|_| "C:\\Windows".to_string());
        let cmd_exe = PathBuf::from(&root).join("System32").join("cmd.exe");
        let cmd_exe = if cmd_exe.is_file() {
            cmd_exe
        } else {
            PathBuf::from("cmd.exe")
        };

        // Windows Terminal when it is actually installed, resolved by path
        // rather than by name: the WindowsApps alias is not reliably on a GUI
        // process's `PATH`.
        let wt = std::env::var("LOCALAPPDATA").ok().map(|local| {
            PathBuf::from(local)
                .join("Microsoft")
                .join("WindowsApps")
                .join("wt.exe")
        });
        if let Some(wt) = wt.filter(|p| p.is_file()) {
            let mut command = Command::new(wt);
            command
                .args(["-d", cwd])
                .arg(&cmd_exe)
                .arg("/k")
                .arg(&manual)
                .env("PATH", &path)
                .envs(forwarded.clone());
            if command.spawn().is_ok() {
                return Ok(());
            }
        }

        // Otherwise a detached console. `start` hands it to whatever the user's
        // default terminal is, and detaching keeps it alive after we exit and
        // stops it inheriting our pipes.
        let mut command = Command::new(&cmd_exe);
        command
            .args(["/c", "start", "SpeakoFlow agent"])
            .arg(&cmd_exe)
            .arg("/k")
            .arg(&manual)
            .current_dir(cwd)
            .env("PATH", &path)
            .envs(forwarded.clone());
        if command.spawn().is_ok() {
            return Ok(());
        }
    }

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "tell application \"Terminal\" to do script \"cd {} && {}\"",
            shell_quote(cwd),
            manual
        );
        let mut command = Command::new("/usr/bin/osascript");
        command
            .args(["-e", script.as_str()])
            .env("PATH", &path)
            .envs(forwarded.clone());
        if command.spawn().is_ok() {
            return Ok(());
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // No standard terminal on Linux, so try the usual suspects in order.
        for terminal in [
            "x-terminal-emulator",
            "gnome-terminal",
            "konsole",
            "alacritty",
            "kitty",
            "xterm",
        ] {
            let mut command = Command::new(terminal);
            command
                .args(["-e", "sh", "-c", &format!("{}; exec $SHELL", manual)])
                .current_dir(cwd)
                .env("PATH", &path)
                .envs(forwarded.clone());
            if command.spawn().is_ok() {
                return Ok(());
            }
        }
    }

    Err(format!(
        "Couldn't open a terminal. Run this yourself in {}: {}",
        cwd, manual
    ))
}

/// Minimal quoting for the AppleScript path above.
#[cfg(target_os = "macos")]
fn shell_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// Serialize one JSON value as a protocol line.
fn write_line(stdin: &Arc<Mutex<Option<ChildStdin>>>, value: &Value) -> Result<(), String> {
    let mut guard = stdin.lock().unwrap();
    let pipe = guard
        .as_mut()
        .ok_or_else(|| "That session's input is closed.".to_string())?;
    let mut line = value.to_string();
    line.push('\n');
    pipe.write_all(line.as_bytes())
        .and_then(|_| pipe.flush())
        .map_err(|e| format!("Could not talk to the agent: {}", e))
}

/// One user turn in the shape the CLI expects.
fn user_message(text: &str) -> Value {
    json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
        "parent_tool_use_id": null,
        "session_id": ""
    })
}

fn emit_changed(app: &AppHandle, state: &Arc<Mutex<SessionState>>) {
    let view = state.lock().unwrap().view();
    let _ = app.emit("agent-session-update", view);
}

/// A status change the user asked to be told about, in one spoken sentence.
///
/// Only the transitions worth interrupting someone for produce a notice. A
/// session going from Starting to Working is noise; a session blocking on a
/// decision, or finishing, is the entire reason this feature exists.
fn notice_for(before: AgentStatus, view: &AgentSessionView) -> Option<String> {
    match view.status {
        AgentStatus::WaitingApproval => {
            let pending = view.pending.as_ref()?;
            Some(format!(
                "{} needs your approval to {}.",
                view.label, pending.detail
            ))
        }
        AgentStatus::Idle => Some(format!(
            "{} finished after {}.",
            view.label,
            format_duration(view.elapsed_secs)
        )),
        AgentStatus::Failed => Some(format!("{} failed.", view.label)),
        // Everything else is either noise or something the user just did
        // themselves, and being told about your own action is irritating.
        AgentStatus::Starting
        | AgentStatus::Working
        | AgentStatus::Cancelled
        | AgentStatus::HandedOff
        | AgentStatus::Ended => {
            let _ = before;
            None
        }
    }
}

/// Tell the user about a transition: an event for the UI, and a spoken line when
/// spoken answers are switched on.
fn announce(app: &AppHandle, before: AgentStatus, view: &AgentSessionView) {
    let Some(message) = notice_for(before, view) else {
        return;
    };
    let _ = app.emit(
        "agent-notification",
        json!({
            "sessionId": view.id,
            "label": view.label,
            "status": view.status,
            "message": message,
            "highRisk": view.pending.as_ref().map(|p| p.high_risk).unwrap_or(false),
        }),
    );
    speak_notice(app, message);
}

/// Speak one short line through whichever TTS engine the assistant is using.
///
/// Reuses the assistant's own split: the local Kokoro engine renders in the
/// webview, everything else is fetched and played in the backend. Silent unless
/// the user has already turned spoken answers on, so this can never surprise
/// someone with audio they did not ask for.
fn speak_notice(app: &AppHandle, message: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let settings = crate::settings::get_settings(&app);
        if !settings.assistant_tts_enabled {
            return;
        }
        let text = crate::tts::sanitize_for_speech(&message);
        if text.trim().is_empty() {
            return;
        }
        if settings.assistant_tts_engine == "kokoro" {
            let _ = app.emit("assistant-tts", text);
        } else {
            crate::tts::speak_remote(&app, &settings, text).await;
        }
    });
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

/// Fold one protocol event into the digest. Returns whether anything the user
/// would notice changed, so we only emit on real transitions.
fn apply_event(state: &Arc<Mutex<SessionState>>, event: &Value) -> bool {
    let ty = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut s = state.lock().unwrap();
    match ty {
        "system" => match event.get("subtype").and_then(Value::as_str).unwrap_or("") {
            "init" => {
                s.agent_session_id = event
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(model) = event.get("model").and_then(Value::as_str) {
                    s.model = Some(model.to_string());
                }
                s.status = AgentStatus::Working;
                true
            }
            // The CLI auto-denied because permission routing was unavailable.
            // Surfaced as an error rather than silently looking idle.
            "permission_denied" => {
                let tool = event
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or("a tool");
                s.error = Some(format!("{} was blocked by the permission prompt.", tool));
                true
            }
            _ => false,
        },
        "assistant" => {
            let blocks = event
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut changed = false;
            for block in blocks {
                match block.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text" => {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            let text = text.trim();
                            if !text.is_empty() {
                                s.last_line = Some(truncate(text, LINE_BUDGET));
                                changed = true;
                            }
                        }
                    }
                    "tool_use" => {
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .to_string();
                        s.tool_calls = s.tool_calls.saturating_add(1);
                        s.last_tool = Some(name);
                        if let Some(path) = block.get("input").and_then(file_path_of) {
                            if !s.files_touched.iter().any(|p| p == &path) {
                                if s.files_touched.len() >= FILES_BUDGET {
                                    s.files_touched.remove(0);
                                }
                                s.files_touched.push(path);
                            }
                        }
                        s.status = AgentStatus::Working;
                        changed = true;
                    }
                    _ => {}
                }
            }
            changed
        }
        // Tool results arrive as user messages. Only failures are worth
        // recording: the model malforming a tool call and retrying looks
        // identical to steady progress from outside, which is exactly how a
        // session appeared "Working" for ten seconds while getting nowhere.
        "user" => {
            let blocks = event
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut changed = false;
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                    continue;
                }
                let failed = block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if failed {
                    let text = tool_result_text(block.get("content"));
                    s.tool_error = Some(truncate(&clean_tool_error(&text), LINE_BUDGET));
                    changed = true;
                } else if s.tool_error.is_some() {
                    // It recovered; a stale error is worse than none.
                    s.tool_error = None;
                    changed = true;
                }
            }
            changed
        }
        "control_request" => {
            let request = event.get("request").cloned().unwrap_or(Value::Null);
            if request.get("subtype").and_then(Value::as_str) != Some("can_use_tool") {
                return false;
            }
            let tool_name = request
                .get("tool_name")
                .and_then(Value::as_str)
                .unwrap_or("a tool")
                .to_string();
            let input = request.get("input").cloned().unwrap_or(Value::Null);
            let detail = describe_action(&tool_name, &input, request.get("description"));
            let high_risk = is_high_risk(&tool_name, &input);
            s.pending = Some(PendingApproval {
                request_id: event
                    .get("request_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                tool_name,
                detail,
                high_risk,
            });
            s.status = AgentStatus::WaitingApproval;
            true
        }
        "result" => {
            if let Some(cost) = event.get("total_cost_usd").and_then(Value::as_f64) {
                s.cost_usd = cost;
            }
            let subtype = event
                .get("subtype")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let is_error = event
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(text) = event.get("result").and_then(Value::as_str) {
                let text = text.trim();
                if !text.is_empty() {
                    s.last_line = Some(truncate(text, LINE_BUDGET));
                }
            }
            s.status = if subtype == "success" && !is_error {
                AgentStatus::Idle
            } else if s.status == AgentStatus::Cancelled {
                // An interrupt reports as an execution error; keep the honest label.
                AgentStatus::Cancelled
            } else {
                // The agent's own message is the useful part; `subtype` can be
                // "success" even on a failed turn, which reads as nonsense on
                // its own. Only fall back to it when there is nothing better.
                if s.error.is_none() {
                    s.error = Some(match &s.last_line {
                        Some(line) => line.clone(),
                        None => format!("The turn ended with {}.", subtype),
                    });
                }
                AgentStatus::Failed
            };
            s.pending = None;
            true
        }
        _ => false,
    }
}

/// A tool result's text, whether it came as a plain string or as content blocks.
fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// Strip the CLI's machine-oriented wrapper so the message reads like a sentence
/// a person can act on.
fn clean_tool_error(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .trim_start_matches("<tool_use_error>")
        .trim_end_matches("</tool_use_error>")
        .trim();
    if trimmed.is_empty() {
        "A tool call failed.".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Pull a file path out of a tool input, whatever the tool calls it.
fn file_path_of(input: &Value) -> Option<String> {
    for key in ["file_path", "path", "notebook_path", "filePath"] {
        if let Some(p) = input.get(key).and_then(Value::as_str) {
            if !p.trim().is_empty() {
                return Some(p.to_string());
            }
        }
    }
    None
}

/// One short line describing a requested action, suitable for reading aloud.
fn describe_action(tool: &str, input: &Value, description: Option<&Value>) -> String {
    if let Some(command) = input.get("command").and_then(Value::as_str) {
        return format!("run `{}`", truncate(command.trim(), 160));
    }
    if let Some(path) = file_path_of(input) {
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or(path);
        return format!("{} {}", verb_for(tool), name);
    }
    if let Some(url) = input.get("url").and_then(Value::as_str) {
        return format!("fetch {}", truncate(url, 120));
    }
    if let Some(desc) = description.and_then(Value::as_str) {
        if !desc.trim().is_empty() {
            return truncate(desc.trim(), 160);
        }
    }
    format!("use {}", tool)
}

fn verb_for(tool: &str) -> &'static str {
    match tool {
        "Write" => "create",
        "Edit" | "MultiEdit" | "NotebookEdit" => "edit",
        "Read" => "read",
        _ => "touch",
    }
}

/// Whether an action is destructive enough that a spoken "yes" must not be
/// sufficient. Deliberately blunt and deliberately over-inclusive: a false
/// positive costs one tap in the app, a false negative can cost a repository.
fn is_high_risk(tool: &str, input: &Value) -> bool {
    let command = input
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    if command.is_empty() {
        // Non-command tools are scoped to files; edits are recoverable via git
        // and are the common case we do not want to make annoying.
        return false;
    }
    const DANGER: [&str; 18] = [
        "rm -rf",
        "rm -r",
        "rmdir /s",
        "del /f",
        "del /q",
        "format ",
        "mkfs",
        "dd if=",
        "git reset --hard",
        "git clean -f",
        "git push --force",
        "git push -f",
        "branch -d",
        "drop table",
        "drop database",
        "truncate table",
        "shutdown",
        "reg delete",
    ];
    if DANGER.iter().any(|needle| command.contains(needle)) {
        return true;
    }
    // Elevation, and the classic pipe-from-the-internet-into-a-shell.
    if command.starts_with("sudo ") || command.contains(" sudo ") {
        return true;
    }
    let piped_to_shell = (command.contains("curl ") || command.contains("wget "))
        && (command.contains("| sh") || command.contains("| bash") || command.contains("|sh"));
    piped_to_shell || tool.eq_ignore_ascii_case("KillShell") && command.contains("-9")
}

fn truncate(text: &str, budget: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= budget {
        return flat;
    }
    let cut: String = flat.chars().take(budget.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Durations a person would say out loud.
fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Parse the JSON arguments shared by the agent tools.
pub fn parse_session_ref(raw: &str) -> String {
    let value: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    for key in ["session", "session_id", "id", "which", "label"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return text.trim().to_string();
            }
        }
    }
    // A number is a perfectly good reference.
    if let Some(n) = value.get("session").and_then(Value::as_u64) {
        return n.to_string();
    }
    String::new()
}

/// Read a string argument by any of several plausible names, since different
/// models name the same field differently.
pub fn arg_str(raw: &str, keys: &[&str]) -> Option<String> {
    let value: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let map: HashMap<String, Value> = match value {
        Value::Object(map) => map.into_iter().collect(),
        _ => return None,
    };
    for key in keys {
        if let Some(text) = map.get(*key).and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return Some(text.trim().to_string());
            }
        }
    }
    None
}

/// Read a boolean argument, tolerating the string forms models emit.
pub fn arg_bool(raw: &str, keys: &[&str]) -> Option<bool> {
    let value: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    for key in keys {
        match value.get(*key) {
            Some(Value::Bool(b)) => return Some(*b),
            Some(Value::String(s)) => match s.trim().to_lowercase().as_str() {
                "true" | "yes" | "allow" | "approve" => return Some(true),
                "false" | "no" | "deny" | "reject" => return Some(false),
                _ => {}
            },
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Arc<Mutex<SessionState>> {
        Arc::new(Mutex::new(SessionState {
            id: "1".into(),
            agent_session_id: None,
            label: "handy".into(),
            cwd: "C:/tmp".into(),
            model: None,
            status: AgentStatus::Starting,
            started: Instant::now(),
            last_tool: None,
            last_line: None,
            files_touched: Vec::new(),
            tool_calls: 0,
            cost_usd: 0.0,
            pending: None,
            tool_error: None,
            error: None,
            task: "do a thing".into(),
        }))
    }

    #[test]
    fn init_event_records_session_and_model() {
        let s = state();
        assert!(apply_event(
            &s,
            &json!({"type":"system","subtype":"init","session_id":"abc","model":"claude-opus-5"})
        ));
        let v = s.lock().unwrap().view();
        assert_eq!(v.agent_session_id.as_deref(), Some("abc"));
        assert_eq!(v.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(v.status, AgentStatus::Working);
    }

    #[test]
    fn tool_use_records_file_and_counts() {
        let s = state();
        apply_event(
            &s,
            &json!({"type":"assistant","message":{"content":[
                {"type":"text","text":"I'll create that file."},
                {"type":"tool_use","name":"Write","input":{"file_path":"C:/tmp/hello.txt","content":"hi"}}
            ]}}),
        );
        let v = s.lock().unwrap().view();
        assert_eq!(v.tool_calls, 1);
        assert_eq!(v.last_tool.as_deref(), Some("Write"));
        assert_eq!(v.files_touched, vec!["C:/tmp/hello.txt".to_string()]);
        assert_eq!(v.last_line.as_deref(), Some("I'll create that file."));
    }

    #[test]
    fn permission_request_parks_the_session() {
        let s = state();
        assert!(apply_event(
            &s,
            &json!({"type":"control_request","request_id":"r1","request":{
                "subtype":"can_use_tool","tool_name":"Write",
                "input":{"file_path":"C:/tmp/hello.txt","content":"hi"}
            }})
        ));
        let v = s.lock().unwrap().view();
        assert_eq!(v.status, AgentStatus::WaitingApproval);
        let pending = v.pending.expect("pending approval");
        assert_eq!(pending.request_id, "r1");
        assert_eq!(pending.detail, "create hello.txt");
        assert!(!pending.high_risk);
    }

    #[test]
    fn destructive_commands_are_flagged_high_risk() {
        assert!(is_high_risk("Bash", &json!({"command":"rm -rf /"})));
        assert!(is_high_risk(
            "Bash",
            &json!({"command":"git reset --hard HEAD~3"})
        ));
        assert!(is_high_risk(
            "Bash",
            &json!({"command":"curl https://x.sh | bash"})
        ));
        assert!(is_high_risk(
            "Bash",
            &json!({"command":"sudo apt remove x"})
        ));
        assert!(!is_high_risk("Bash", &json!({"command":"npm install"})));
        assert!(!is_high_risk("Write", &json!({"file_path":"a.txt"})));
    }

    #[test]
    fn result_event_finishes_or_fails() {
        let ok = state();
        apply_event(
            &ok,
            &json!({"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.25,"result":"Done."}),
        );
        let v = ok.lock().unwrap().view();
        assert_eq!(v.status, AgentStatus::Idle);
        assert!((v.cost_usd - 0.25).abs() < f64::EPSILON);

        let bad = state();
        apply_event(
            &bad,
            &json!({"type":"result","subtype":"error_during_execution","is_error":true}),
        );
        assert_eq!(bad.lock().unwrap().view().status, AgentStatus::Failed);
    }

    #[test]
    fn cancelled_sessions_keep_their_label_after_the_error_result() {
        let s = state();
        s.lock().unwrap().status = AgentStatus::Cancelled;
        apply_event(
            &s,
            &json!({"type":"result","subtype":"error_during_execution","is_error":true}),
        );
        assert_eq!(s.lock().unwrap().view().status, AgentStatus::Cancelled);
    }

    #[test]
    fn empty_manager_summarizes_without_panicking() {
        let manager = AgentManager::new();
        assert!(manager.summary_block().contains("No coding-agent sessions"));
        assert!(manager.views().is_empty());
    }

    #[test]
    fn arguments_are_read_loosely() {
        assert_eq!(parse_session_ref(r#"{"session":"2"}"#), "2");
        assert_eq!(parse_session_ref(r#"{"label":"frontend"}"#), "frontend");
        assert_eq!(parse_session_ref("not json"), "");
        assert_eq!(
            arg_str(r#"{"folder":"C:/x"}"#, &["cwd", "folder"]).as_deref(),
            Some("C:/x")
        );
        assert_eq!(arg_bool(r#"{"allow":"yes"}"#, &["allow"]), Some(true));
        assert_eq!(arg_bool(r#"{"allow":false}"#, &["allow"]), Some(false));
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(9), "9s");
        assert_eq!(format_duration(75), "1m 15s");
        assert_eq!(format_duration(3700), "1h 1m");
    }

    #[test]
    fn tool_failures_are_recorded_and_then_cleared() {
        // The real failure that started this: the model malformed a Write call,
        // got a validation error, and the session looked like it was working.
        let s = state();
        assert!(apply_event(
            &s,
            &json!({"type":"user","message":{"content":[{
                "type":"tool_result",
                "is_error":true,
                "tool_use_id":"toolu_1",
                "content":"<tool_use_error>InputValidationError: Write failed due to the following issue:\nAn unexpected parameter `command` was provided</tool_use_error>"
            }]}})
        ));
        let v = s.lock().unwrap().view();
        let recorded = v.tool_error.expect("the failure should be visible");
        assert!(recorded.contains("An unexpected parameter"));
        assert!(
            !recorded.contains("tool_use_error"),
            "wrapper should be stripped"
        );
        // Not a failed session — it can still recover.
        assert_ne!(v.status, AgentStatus::Failed);

        // The retry succeeds, so the stale error must go.
        assert!(apply_event(
            &s,
            &json!({"type":"user","message":{"content":[{
                "type":"tool_result",
                "tool_use_id":"toolu_2",
                "content":"File created successfully at: C:\\tmp\\hello.txt"
            }]}})
        ));
        assert!(s.lock().unwrap().view().tool_error.is_none());
    }

    #[test]
    fn tool_result_text_handles_both_shapes() {
        assert_eq!(tool_result_text(Some(&json!("plain"))), "plain");
        assert_eq!(
            tool_result_text(Some(
                &json!([{ "type": "text", "text": "a" }, { "type": "text", "text": "b" }])
            )),
            "a b"
        );
        assert_eq!(tool_result_text(None), "");
        assert_eq!(
            clean_tool_error("  <tool_use_error></tool_use_error> "),
            "A tool call failed."
        );
    }

    #[test]
    fn a_handed_off_session_is_no_longer_live_and_says_nothing() {
        let s = state();
        s.lock().unwrap().status = AgentStatus::HandedOff;
        let v = s.lock().unwrap().view();
        assert!(!v.status.is_live());
        // The user just did this themselves; announcing it would be noise.
        assert!(notice_for(AgentStatus::Working, &v).is_none());
    }

    #[test]
    fn a_failed_turn_reports_the_agents_own_message_not_the_subtype() {
        // `subtype` can be "success" on a failed turn, which reads as nonsense
        // ("the turn ended with success" on a red Failed row).
        let s = state();
        apply_event(
            &s,
            &json!({"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}),
        );
        let v = s.lock().unwrap().view();
        assert_eq!(v.status, AgentStatus::Failed);
        assert_eq!(
            v.error.as_deref(),
            Some("Not logged in · Please run /login")
        );

        // With nothing better to say, the subtype is still the fallback.
        let bare = state();
        apply_event(
            &bare,
            &json!({"type":"result","subtype":"error_during_execution","is_error":true}),
        );
        assert_eq!(
            bare.lock().unwrap().view().error.as_deref(),
            Some("The turn ended with error_during_execution.")
        );
    }

    #[test]
    fn only_transitions_worth_interrupting_produce_a_notice() {
        let s = state();
        // Blocked on a decision: the one case worth speaking over your work.
        apply_event(
            &s,
            &json!({"type":"control_request","request_id":"r1","request":{
                "subtype":"can_use_tool","tool_name":"Bash","input":{"command":"npm install"}
            }}),
        );
        let view = s.lock().unwrap().view();
        let notice = notice_for(AgentStatus::Working, &view).expect("should notify");
        assert!(notice.contains("handy"));
        assert!(notice.contains("npm install"));

        // Finished.
        let done = state();
        apply_event(
            &done,
            &json!({"type":"result","subtype":"success","is_error":false}),
        );
        let view = done.lock().unwrap().view();
        assert!(notice_for(AgentStatus::Working, &view)
            .expect("should notify")
            .contains("finished"));

        // Failed.
        let failed = state();
        apply_event(
            &failed,
            &json!({"type":"result","subtype":"error_during_execution","is_error":true}),
        );
        let view = failed.lock().unwrap().view();
        assert!(notice_for(AgentStatus::Working, &view)
            .expect("should notify")
            .contains("failed"));
    }

    #[test]
    fn routine_and_self_inflicted_transitions_stay_silent() {
        let s = state();
        // Starting to working is noise.
        apply_event(
            &s,
            &json!({"type":"system","subtype":"init","session_id":"abc"}),
        );
        let view = s.lock().unwrap().view();
        assert!(notice_for(AgentStatus::Starting, &view).is_none());

        // The user stopped it themselves; telling them is irritating.
        let stopped = state();
        stopped.lock().unwrap().status = AgentStatus::Cancelled;
        let view = stopped.lock().unwrap().view();
        assert!(notice_for(AgentStatus::Working, &view).is_none());

        // A closed process is not an event worth speaking either.
        let ended = state();
        ended.lock().unwrap().status = AgentStatus::Ended;
        let view = ended.lock().unwrap().view();
        assert!(notice_for(AgentStatus::Working, &view).is_none());
    }
}
