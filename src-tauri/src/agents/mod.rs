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

mod acp;
mod env;
mod policy;
mod registry;
mod workspace;

pub use policy::ApprovalPolicy;
pub use registry::{AgentKind, Transport};
pub use workspace::{create_file, create_folder, machine_context};

/// Spoken names of every coding agent installed on this machine.
pub fn installed_agent_labels() -> Vec<String> {
    registry::installed()
        .iter()
        .map(|a| a.label().to_string())
        .collect()
}

/// How a new instruction should reach a session that is already working.
///
/// ACP has no way to slip a message into a running turn — `session/cancel` is
/// the only interrupt — so the choice is real and it matters: interrupting
/// throws away whatever the agent was part-way through, and queueing makes the
/// user wait. Every other tool in this space makes the human pick with a
/// modifier key. We are listening to a sentence, so we can read the intent out
/// of how it was said, which is the one thing voice is genuinely better at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Deliver after the current turn finishes. The safe default.
    Queue,
    /// Cancel the running turn and deliver immediately.
    Interrupt,
}

impl Delivery {
    /// Read the intent out of what the user said.
    ///
    /// Deliberately conservative and deliberately deterministic. Conservative
    /// because the costs are lopsided: a wrongly queued message wastes a little
    /// time, while a wrongly interrupted turn destroys work in progress — so
    /// anything ambiguous queues. Deterministic because "stop" must not wait on
    /// a model round trip to be understood, and because a hard-coded list of
    /// stop words is auditable in a way an LLM's judgement is not.
    ///
    /// The assistant may still override this when a sentence needs real
    /// understanding; this is the floor, not the ceiling.
    pub fn from_spoken(text: &str) -> Self {
        let lowered = text.trim().to_lowercase();
        if lowered.is_empty() {
            return Delivery::Queue;
        }

        // "also", "after that", "and then" — explicitly about what comes next,
        // even when the sentence also contains a stop word ("after that, stop
        // the dev server"). Checked first for exactly that reason.
        const DEFERRING: [&str; 9] = [
            "after that",
            "after this",
            "afterwards",
            "when you're done",
            "when you are done",
            "when that's done",
            "and then",
            "next, ",
            "also ",
        ];
        if DEFERRING.iter().any(|marker| lowered.contains(marker)) {
            return Delivery::Queue;
        }

        // Spoken corrections front-load the stop word: people say "stop, do X",
        // not "do X, stop". So position carries the meaning, and matching these
        // anywhere in the sentence is what made "make it wait for the response"
        // read as an interrupt. Leading position only.
        let mut head = lowered.as_str();
        for filler in [
            "um,", "um ", "uh,", "uh ", "okay,", "okay ", "ok,", "ok ", "so,", "so ", "hey ",
            "yeah,", "yeah ", "hmm,", "hmm ",
        ] {
            if let Some(rest) = head.strip_prefix(filler) {
                head = rest.trim_start();
            }
        }
        const HALTING_LEAD: [&str; 20] = [
            "stop",
            "wait",
            "hold on",
            "hang on",
            "hold up",
            "no",
            "nope",
            "nah",
            "actually",
            "cancel",
            "forget",
            "scrap",
            "never mind",
            "nevermind",
            "abort",
            "halt",
            "don't",
            "dont",
            "do not",
            "instead",
        ];
        for marker in HALTING_LEAD {
            if head == marker {
                return Delivery::Interrupt;
            }
            if let Some(rest) = head.strip_prefix(marker) {
                // A real word boundary, so "not sure" and "now do X" are not
                // read as "no", and "nonstop" is not read as "stop".
                if rest.starts_with(|c: char| !c.is_alphanumeric() && c != '\'') {
                    return Delivery::Interrupt;
                }
            }
        }

        // A few phrases mean "abandon that" wherever they appear.
        const HALTING_ANYWHERE: [&str; 6] = [
            "cancel that",
            "forget that",
            "scrap that",
            "never mind that",
            "instead of",
            "stop doing",
        ];
        if HALTING_ANYWHERE
            .iter()
            .any(|marker| lowered.contains(marker))
        {
            return Delivery::Interrupt;
        }

        Delivery::Queue
    }
}

/// Environment override for the model handed to the agent CLI.
///
/// Deliberately an environment variable rather than a setting for now: the agent
/// feature is pre-release, and a real settings field would ripple into the
/// generated frontend bindings before the shape has settled. When unset the
/// agent uses whatever its own config selects.
const MODEL_ENV: &str = "SPEAKOFLOW_AGENT_MODEL";

/// Default reasoning effort for new sessions, when the agent supports one.
///
/// Same reasoning as [`MODEL_ENV`]: an override for now, not a settings field.
/// Kiro's ladder is `low`, `medium`, `high`, `xhigh`, `max`; the spec's
/// equivalent is a `thought_level` config option, whose values are the agent's
/// to name.
const EFFORT_ENV: &str = "SPEAKOFLOW_AGENT_EFFORT";

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
    /// Which agent is behind this session, for the spoken summary.
    pub agent: String,
    /// Cost and context usage as a ready-made phrase, when the agent volunteers
    /// them. A phrase rather than a number because agents do not agree on units:
    /// Kiro meters in credits, Claude in dollars, and a single `cost_usd` field
    /// holding either is a status report that lies.
    pub usage: Option<String>,
    /// How many permission requests were answered automatically by policy.
    /// Surfaced so auto-approval is never silent.
    pub auto_approvals: u32,
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
    agent: AgentKind,
    usage: Option<String>,
    auto_approvals: u32,
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
            agent: self.agent.label().to_string(),
            usage: self.usage.clone(),
            auto_approvals: self.auto_approvals,
        }
    }
}

/// One live session: shared state plus the pipes needed to talk to it.
struct Session {
    state: Arc<Mutex<SessionState>>,
    /// Held separately so the reader thread never blocks a writer.
    ///
    /// Empty for ACP sessions, whose writes all go through [`acp::AcpHandle`].
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    child: Arc<Mutex<Child>>,
    /// The protocol client, for every agent except native Claude Code.
    acp: Option<Arc<acp::AcpHandle>>,
}

/// What to start, and how.
///
/// A struct rather than eight positional arguments, because the list grew a
/// "which agent" and a "create the folder" and will grow more.
pub struct StartRequest<'a> {
    /// Working directory for the agent.
    pub cwd: &'a str,
    /// The first turn.
    pub prompt: &'a str,
    /// Spoken name for the session. Defaults to the folder name.
    pub label: Option<String>,
    pub model: Option<String>,
    /// Reasoning effort for this session, where the agent has one.
    pub effort: Option<String>,
    /// Which agent to use. `None` picks the configured or first installed one.
    pub agent: Option<&'a str>,
    /// Create `cwd` if it does not exist. Only ever set from an explicit request.
    pub create_if_missing: bool,
    /// Answer safe permission requests automatically for this session.
    pub auto_approve: bool,
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

    /// Start a new agent session and send its first turn.
    ///
    /// Returns the short session id. The call does not wait for the agent to
    /// finish — that is the entire point.
    pub fn start(&self, app: &AppHandle, req: StartRequest<'_>) -> Result<String, String> {
        let prompt = req.prompt.trim();
        if prompt.is_empty() {
            return Err("The agent needs a task to work on.".to_string());
        }

        // A folder that does not exist yet is the normal case for "start a new
        // project", so it can be created — but only when asked, and only
        // somewhere allowed. Creating directories from a possibly-misheard path
        // without being asked is how you end up with a folder called "the
        // desktop".
        let cwd = req.cwd.trim();
        if cwd.is_empty() {
            return Err("Which folder should it work in?".to_string());
        }
        let resolved = if std::path::Path::new(cwd).is_dir() {
            std::path::PathBuf::from(cwd)
        } else if req.create_if_missing {
            workspace::create_folder(cwd)?
        } else {
            return Err(format!(
                "`{}` is not a folder that exists. Ask me to create it and I will.",
                cwd
            ));
        };
        let dir = resolved.as_path();

        let kind = match req.agent {
            Some(name) if !name.trim().is_empty() => {
                AgentKind::from_spoken(name).ok_or_else(|| {
                    format!(
                        "I don't know an agent called \"{}\". I can use: {}.",
                        name.trim(),
                        registry::installed()
                            .iter()
                            .map(|a| a.label())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?
            }
            _ => registry::default_agent().ok_or_else(|| {
                "No coding agent is installed. Install the Kiro CLI or Claude Code first."
                    .to_string()
            })?,
        };

        // Everything except Claude's own protocol goes over ACP.
        if kind.transport() == Transport::Acp {
            return self.start_acp(app, kind, dir, prompt, req);
        }

        let model = req.model;
        let label = req.label;

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

        let (short_id, state) = self.new_state(dir, prompt, label, model, AgentKind::ClaudeCode);
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
            acp: None,
        });
        let _ = app.emit("agent-sessions-changed", self.views());
        Ok(short_id)
    }

    /// Start a session over the Agent Client Protocol.
    ///
    /// Short, because the protocol client does the work. That is the point of
    /// the exercise: adding an agent should not add a code path.
    fn start_acp(
        &self,
        app: &AppHandle,
        kind: AgentKind,
        dir: &std::path::Path,
        prompt: &str,
        req: StartRequest<'_>,
    ) -> Result<String, String> {
        let model = req
            .model
            .or_else(|| env::resolve_var(MODEL_ENV))
            .filter(|m| !m.trim().is_empty());
        let wanted = acp::Wanted {
            model: model.clone(),
            effort: req
                .effort
                .or_else(|| env::resolve_var(EFFORT_ENV))
                .filter(|e| !e.trim().is_empty()),
        };
        let (short_id, state) = self.new_state(dir, prompt, req.label, model, kind);

        let policy = ApprovalPolicy {
            enabled: req.auto_approve,
            project_root: Some(dir.to_path_buf()),
            ..ApprovalPolicy::default()
        };
        let (handle, child) =
            match acp::AcpHandle::start(app, kind, Arc::clone(&state), prompt, policy, wanted) {
                Ok(started) => started,
                Err(e) => {
                    // Nothing was registered yet, so there is no row to leave behind.
                    return Err(e);
                }
            };

        self.sessions.lock().unwrap().push(Session {
            state,
            // The real pipe lives inside the ACP handle; every write for this
            // session goes through the protocol client, never through here.
            stdin: Arc::new(Mutex::new(None)),
            child: Arc::new(Mutex::new(child)),
            acp: Some(Arc::new(handle)),
        });
        let _ = app.emit("agent-sessions-changed", self.views());
        Ok(short_id)
    }

    /// Build the shared digest for a new session and allocate its short id.
    fn new_state(
        &self,
        dir: &std::path::Path,
        prompt: &str,
        label: Option<String>,
        model: Option<String>,
        agent: AgentKind,
    ) -> (String, Arc<Mutex<SessionState>>) {
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
            agent,
            usage: None,
            auto_approvals: 0,
        }));
        (short_id, state)
    }

    /// The ACP driver for a session, if it has one.
    fn acp_for(&self, id: &str) -> Option<Arc<acp::AcpHandle>> {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.state.lock().unwrap().id == id)
            .and_then(|s| s.acp.clone())
    }

    /// Switch a session's mode, which is how ACP exposes personas.
    ///
    /// Kiro returns its whole agent list as modes — `kiro_planner`, `research`,
    /// and any the user has defined — so this is "switch to the planner" without
    /// a feature behind it.
    pub fn set_mode(&self, app: &AppHandle, needle: &str, mode: &str) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label) = {
            let s = state.lock().unwrap();
            (s.id.clone(), s.label.clone())
        };
        let acp = self.acp_for(&id).ok_or_else(|| {
            format!(
                "[{}] {} doesn't support modes — only agents driven over ACP do.",
                id, label
            )
        })?;
        let chosen = acp.set_mode(mode)?;
        emit_changed(app, &state);
        Ok(format!("[{}] {} is now in {} mode.", id, label, chosen))
    }

    /// The modes a session offers, worded for reading aloud.
    pub fn modes_block(&self, needle: &str) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label) = {
            let s = state.lock().unwrap();
            (s.id.clone(), s.label.clone())
        };
        let Some(acp) = self.acp_for(&id) else {
            return Ok(format!(
                "[{}] {} has no modes to switch between.",
                id, label
            ));
        };
        let modes = acp.modes();
        if modes.available.is_empty() {
            return Ok(format!("[{}] {} didn't report any modes.", id, label));
        }
        let mut out = format!(
            "[{}] {} is in {} mode. Available:\n",
            id,
            label,
            modes.current.as_deref().unwrap_or("its default")
        );
        for (mode_id, description) in modes.available.iter().take(12) {
            if description.is_empty() {
                out.push_str(&format!("- {}\n", mode_id));
            } else {
                out.push_str(&format!("- {}: {}\n", mode_id, truncate(description, 140)));
            }
        }
        Ok(out)
    }

    /// Switch the model a running session is using.
    ///
    /// This is the thing the feature was accused of not being able to do. It can:
    /// ACP's config-option surface is explicitly changeable "at any point during
    /// a session, whether the Agent is idle or generating a response", and
    /// `session/set_model` was verified to take effect on a live Kiro session.
    /// Nothing is cancelled and no work is lost — the next request the agent
    /// makes uses the new model.
    pub fn set_model(&self, app: &AppHandle, needle: &str, model: &str) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label, working) = {
            let s = state.lock().unwrap();
            (
                s.id.clone(),
                s.label.clone(),
                matches!(s.status, AgentStatus::Working),
            )
        };
        let acp = self.acp_for(&id).ok_or_else(|| {
            format!(
                "[{}] {} can't change model mid-session — only agents driven over ACP can.",
                id, label
            )
        })?;
        let chosen = acp.set_model(model)?;
        emit_changed(app, &state);
        Ok(if working {
            format!(
                "[{}] {} is switching to {} and carrying on with what it was doing.",
                id, label, chosen
            )
        } else {
            format!("[{}] {} is now using {}.", id, label, chosen)
        })
    }

    /// Set how hard a session's model should think.
    pub fn set_effort(&self, app: &AppHandle, needle: &str, level: &str) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label) = {
            let s = state.lock().unwrap();
            (s.id.clone(), s.label.clone())
        };
        let acp = self.acp_for(&id).ok_or_else(|| {
            format!(
                "[{}] {} has no thinking-effort setting to change.",
                id, label
            )
        })?;
        let (chosen, via_command) = acp.set_effort(level)?;
        emit_changed(app, &state);
        Ok(if via_command {
            // Hedged deliberately: this route goes through the agent's own
            // command, which accepts the request without reporting the result
            // back, so a flat "done" would be unverifiable.
            format!(
                "Asked [{}] {} to switch to {} effort. It applies from its next step, and it doesn't report the level back, so I can't confirm more than that.",
                id, label, chosen
            )
        } else {
            format!("[{}] {} is set to {} effort.", id, label, chosen)
        })
    }

    /// Run one of the agent's own slash commands on a session.
    ///
    /// The point is that a coding CLI is more than a chat box: `/compact` when
    /// the context fills, `/context add` to put a file in front of it, `/usage`
    /// to see what it has spent. Those are the controls a person at a terminal
    /// reaches for, and none of them were reachable by voice.
    pub fn run_command(
        &self,
        app: &AppHandle,
        needle: &str,
        command: &str,
        args: Option<&str>,
        delivery: Delivery,
    ) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label, live) = {
            let s = state.lock().unwrap();
            (s.id.clone(), s.label.clone(), s.status.is_live())
        };
        if !live {
            return Err(format!("Session [{}] {} has closed.", id, label));
        }
        let acp = self.acp_for(&id).ok_or_else(|| {
            format!(
                "[{}] {} doesn't expose commands — only agents driven over ACP do.",
                id, label
            )
        })?;
        let name = command.trim().trim_start_matches('/');
        acp.run_command(name, args, delivery)?;
        emit_changed(app, &state);
        Ok(format!("Ran /{} on [{}] {}.", name, id, label))
    }

    /// What a session can be told to change, worded for reading aloud.
    ///
    /// Deliberately not a dump of everything: a spoken list of nineteen model ids
    /// is unusable, so the current values lead and the choices are trimmed.
    pub fn controls_block(&self, needle: &str) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label) = {
            let s = state.lock().unwrap();
            (s.id.clone(), s.label.clone())
        };
        let Some(acp) = self.acp_for(&id) else {
            return Ok(format!(
                "[{}] {} runs on its own protocol, so its model and effort can't be changed from here.",
                id, label
            ));
        };
        let controls = acp.controls();
        let modes = acp.modes();
        let mut out = format!("[{}] {}\n", id, label);
        out.push_str(&format!(
            "model: {}\n",
            controls.model.as_deref().unwrap_or("not reported")
        ));
        if let Some(effort) = &controls.effort {
            out.push_str(&format!("effort: {}\n", effort));
        }
        if let Some(mode) = &modes.current {
            out.push_str(&format!("mode: {}\n", mode));
        }
        if let Some(percent) = controls.context_percent {
            out.push_str(&format!("context used: {:.0}%\n", percent));
        }
        let choices = controls.models;
        if !choices.is_empty() {
            out.push_str(&format!(
                "models available ({}): {}\n",
                choices.len(),
                choices
                    .iter()
                    .take(12)
                    .map(|(model_id, _)| model_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for option in &controls.config {
            out.push_str(&format!(
                "{} ({}): {}\n",
                option.name,
                option.category.as_deref().unwrap_or("setting"),
                option.current
            ));
        }
        if !controls.commands.is_empty() {
            // Descriptions matter here even though this reads back as speech: the
            // block goes to the model first, and "/compact — Compact conversation
            // history" is what lets it pick the right command instead of guessing
            // at a name it half-remembers.
            out.push_str("commands it accepts:\n");
            for command in controls.commands.iter().take(20) {
                out.push_str(&format!("- /{}", command.name));
                if let Some(hint) = &command.hint {
                    out.push_str(&format!(" {}", hint));
                }
                if !command.description.is_empty() {
                    out.push_str(&format!(": {}", truncate(&command.description, 90)));
                }
                out.push('\n');
            }
        }
        Ok(out)
    }

    /// Turn auto-approval of safe actions on or off for one session.
    ///
    /// Deliberately per-session rather than global: trusting an agent to write
    /// files unattended in a scratch project is not the same decision as
    /// trusting it in the repository that pays the bills.
    pub fn set_auto_approve(
        &self,
        app: &AppHandle,
        needle: &str,
        enabled: bool,
    ) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label, cwd) = {
            let s = state.lock().unwrap();
            (s.id.clone(), s.label.clone(), s.cwd.clone())
        };
        let acp = self.acp_for(&id).ok_or_else(|| {
            format!(
                "Auto-approval isn't available for [{}] {} yet — it only works over ACP.",
                id, label
            )
        })?;
        acp.set_policy(ApprovalPolicy {
            enabled,
            project_root: Some(std::path::PathBuf::from(&cwd)),
            ..ApprovalPolicy::default()
        });

        // "Approve it for me" almost always means the thing on screen right now
        // as well as everything after it. Turning the policy on and then leaving
        // the session still parked would be technically correct and useless.
        // Destructive actions are excluded, as everywhere else.
        let mut released = None;
        if enabled {
            let pending = state.lock().unwrap().pending.clone();
            if let Some(pending) = pending {
                if pending.high_risk {
                    released = Some(format!(
                        " It's still waiting on {}, which is destructive — that one needs you to confirm it in the app.",
                        pending.detail
                    ));
                } else if acp.answer(true).is_ok() {
                    let mut s = state.lock().unwrap();
                    s.pending = None;
                    s.status = AgentStatus::Working;
                    s.auto_approvals += 1;
                    released = Some(format!(" Approved {} for you.", pending.detail));
                }
            }
        }

        emit_changed(app, &state);
        Ok(if enabled {
            format!(
                "Auto-approval on for [{}] {}: reads and edits inside {} go ahead without asking. \
Commands, deletes, and anything outside the folder still stop for you.{}",
                id,
                label,
                cwd,
                released.unwrap_or_default()
            )
        } else {
            format!("Auto-approval off for [{}] {}.", id, label)
        })
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

    /// Send another instruction to a live session, queued behind whatever it is
    /// already doing.
    pub fn send(&self, app: &AppHandle, needle: &str, text: &str) -> Result<String, String> {
        self.steer(app, needle, text, Delivery::Queue)
    }

    /// Send an instruction, choosing whether it waits or interrupts.
    ///
    /// Interrupting is the reason this exists: "no, stop, use JWT instead" has to
    /// land now, and a queued message would arrive after the agent had finished
    /// doing the wrong thing.
    pub fn steer(
        &self,
        app: &AppHandle,
        needle: &str,
        text: &str,
        delivery: Delivery,
    ) -> Result<String, String> {
        let state = self.resolve(needle)?;
        let (id, label, live) = {
            let s = state.lock().unwrap();
            (s.id.clone(), s.label.clone(), s.status.is_live())
        };
        if !live {
            return Err(format!("Session [{}] {} has closed.", id, label));
        }

        if let Some(acp) = self.acp_for(&id) {
            acp.submit(text, delivery)?;
            emit_changed(app, &state);
            return Ok(match delivery {
                Delivery::Interrupt => {
                    format!("Stopping [{}] {} and switching to that.", id, label)
                }
                Delivery::Queue if acp.is_ready() => format!("Sent to [{}] {}.", id, label),
                // Worth saying: the message is safe, it just has not gone yet.
                Delivery::Queue => format!("[{}] {} is still starting — queued it.", id, label),
            });
        }

        // Native Claude transport: stdin is a queue, so an interrupt has to be
        // an interrupt control request followed by the new instruction.
        let (stdin, _) = self
            .pipes(&id)
            .ok_or_else(|| "That session is no longer available.".to_string())?;
        if delivery == Delivery::Interrupt {
            let _ = write_line(
                &stdin,
                &json!({
                    "type": "control_request",
                    "request_id": format!("int_{}", now_millis()),
                    "request": { "subtype": "interrupt" }
                }),
            );
        }
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

        if let Some(acp) = self.acp_for(&id) {
            let outcome = acp.answer(allow)?;
            {
                let mut s = state.lock().unwrap();
                s.pending = None;
                s.status = AgentStatus::Working;
            }
            emit_changed(app, &state);
            return Ok(format!(
                "{} {} in [{}] {}.",
                outcome, pending.tool_name, id, label
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
        if let Some(acp) = self.acp_for(&id) {
            // A session parked on a permission prompt is waiting, not running, so
            // the prompt has to be answered before a cancel means anything.
            if pending.is_some() {
                let _ = acp.answer(false);
            }
            acp.cancel()?;
            // The status flips to Cancelled when the turn actually settles with a
            // `cancelled` stop reason, not here: the agent decides when it has
            // stopped, and claiming otherwise would make the digest lie.
            emit_changed(app, &state);
            return Ok(format!("Stopping [{}] {}.", id, label));
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
        let (id, label, cwd, agent_session_id, pending, kind, status) = {
            let s = state.lock().unwrap();
            (
                s.id.clone(),
                s.label.clone(),
                s.cwd.clone(),
                s.agent_session_id.clone(),
                s.pending.clone(),
                s.agent,
                s.status,
            )
        };

        // Work out what to run before touching the session, so a missing CLI
        // fails without having killed anything.
        let mut handoff = kind.handoff(agent_session_id.as_deref().unwrap_or_default())?;
        if handoff.resumes_history && agent_session_id.is_none() {
            return Err(format!(
                "Session [{}] {} never got far enough to have a resumable id.",
                id, label
            ));
        }

        // The transcript cannot follow a Kiro session into a terminal, so the
        // situation does instead: a brief on disk, and a first question that makes
        // the new session read it before doing anything.
        let brief = if handoff.resumes_history {
            None
        } else {
            let view = state.lock().unwrap().view();
            match workspace::write_handoff_brief(
                &cwd,
                &format!("handoff-{}", id),
                &Self::handoff_brief(&view),
            ) {
                Ok(path) => Some(path),
                Err(e) => {
                    // A read-only or unusual folder must not block the handover;
                    // the terminal is still better than nothing.
                    log::warn!("agents: couldn't write a handoff brief: {}", e);
                    None
                }
            }
        };
        if let (Some(brief), true) = (brief.as_ref(), handoff.accepts_first_prompt) {
            let relative = brief
                .strip_prefix(std::path::Path::new(&cwd))
                .unwrap_or(brief.as_path());
            handoff.command.push_str(&format!(
                " \"Read {} — it is the brief from the session you are taking over. Summarise where things stand in two or three sentences, then wait for instructions.\"",
                relative.display().to_string().replace('\\', "/")
            ));
        }

        // Already handed over: just open another terminal. The previous version
        // left the button dead, which made a failed handoff unrecoverable.
        let already_handed_off = matches!(status, AgentStatus::HandedOff);

        let dropped_pending = pending.is_some();
        if !already_handed_off {
            // Release a pending prompt before handing over, so nothing is left
            // waiting on a decision no one will answer.
            if let Some(pending) = pending {
                if let Some(acp) = self.acp_for(&id) {
                    let _ = acp.answer(false);
                } else if let Some((stdin, _)) = self.pipes(&id) {
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
            // Kill our child but keep the row: removing it looked like the
            // session had vanished rather than moved.
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
                s.last_tool = None;
                s.error = None;
                s.status = AgentStatus::HandedOff;
            }
        }

        if let Err(e) = spawn_agent_terminal(&cwd, &handoff.command) {
            // The terminal is the whole point, so a failure here is worth
            // reporting even though the session is already released.
            emit_changed(app, &state);
            return Err(e);
        }
        emit_changed(app, &state);

        let mut message = if handoff.resumes_history {
            format!(
                "Handed [{}] {} to a terminal — it resumed with its full history, and SpeakoFlow is no longer driving it.",
                id, label
            )
        } else {
            // Said plainly, because the alternative is the user discovering it
            // from a confusing error in the terminal.
            let mut text = format!(
                "Opened {} in a terminal at {}. {} keeps coding sessions separate from terminal ones, so the conversation itself doesn't carry over",
                kind.label(),
                cwd,
                kind.label(),
            );
            match (&brief, handoff.accepts_first_prompt) {
                (Some(_), true) => text.push_str(
                    " — I left it a brief of the task, the files it changed and where it got to, and it opens reading that.",
                ),
                (Some(path), false) => {
                    text.push_str(&format!(" — the brief is at {}.", path.display()))
                }
                (None, _) => text.push('.'),
            }
            text.push_str(&format!(
                " SpeakoFlow is no longer driving [{}] {}.",
                id, label
            ));
            text
        };
        if dropped_pending {
            message.push_str(
                " The action it was waiting on was not applied; it will ask again there.",
            );
        }
        Ok(message)
    }

    /// The takeover brief written into a project when a session is handed to a
    /// terminal that cannot inherit the conversation.
    ///
    /// Written for the *next agent* to read, not for a human to admire: what was
    /// asked, what happened, what is unfinished. Deliberately excludes the rolling
    /// spoken digest's chatter and any tool ids — the incoming agent should re-derive
    /// state from the repository, and a brief that reads like a transcript invites it
    /// to trust stale detail instead.
    fn handoff_brief(view: &AgentSessionView) -> String {
        let mut out = String::from("# Session handoff\n\n");
        out.push_str(&format!(
            "Written by SpeakoFlow when this session moved to a terminal.\n\n\
         - Project: {}\n\
         - Agent: {}\n",
            view.cwd, view.agent
        ));
        if let Some(model) = &view.model {
            out.push_str(&format!("- Model: {}\n", model));
        }
        out.push_str(&format!(
            "- Status when handed over: {}\n- Ran for: {}\n- Tool calls: {}\n",
            view.status.label(),
            format_duration(view.elapsed_secs),
            view.tool_calls
        ));
        if let Some(usage) = &view.usage {
            out.push_str(&format!("- Usage: {}\n", usage));
        }
        out.push_str(&format!("\n## The task as given\n\n{}\n", view.task));
        if !view.files_touched.is_empty() {
            out.push_str("\n## Files it finished changing\n\n");
            for file in &view.files_touched {
                out.push_str(&format!("- {}\n", file));
            }
            out.push_str(
            "\nOnly completed changes are listed. Anything it was part-way through is not here — check the working tree.\n",
        );
        }
        if let Some(pending) = &view.pending {
            out.push_str(&format!(
            "\n## Left waiting for approval\n\n{} ({}). It was refused on the way out, so it will ask again.\n",
            pending.detail, pending.tool_name
        ));
        }
        if let Some(error) = &view.tool_error {
            out.push_str(&format!("\n## Last tool failure\n\n{}\n", error));
        }
        if let Some(error) = &view.error {
            out.push_str(&format!("\n## Last error\n\n{}\n", error));
        }
        if let Some(line) = &view.last_line {
            out.push_str(&format!("\n## Its last message\n\n{}\n", line));
        }
        out.push_str(
            "\n## Taking over\n\nThe previous conversation is gone; this file is all of it. \
         Verify the current state from the files before continuing, and do not assume the \
         work above is complete.\n",
        );
        out
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
fn spawn_agent_terminal(cwd: &str, command_line: &str) -> Result<(), String> {
    let path = env::effective_path();
    let forwarded = env::forwarded_vars();
    let manual = command_line.to_string();

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
            agent: AgentKind::ClaudeCode,
            usage: None,
            auto_approvals: 0,
        }))
    }

    #[test]
    fn corrections_interrupt_and_additions_queue() {
        // The distinction the whole steering feature rests on.
        for spoken in [
            "no, stop",
            "stop, that's wrong",
            "wait, use JWT instead",
            "hold on",
            "actually, do it the other way",
            "don't use sockets",
            "never mind that approach",
            "cancel that and start over",
            "instead of Redis use Postgres",
        ] {
            assert_eq!(
                Delivery::from_spoken(spoken),
                Delivery::Interrupt,
                "{spoken} should interrupt"
            );
        }
        for spoken in [
            "also add tests",
            "after that, update the docs",
            "and then deploy it",
            "when you're done, run the linter",
            "add a dark mode toggle too",
            "make the button blue",
        ] {
            assert_eq!(
                Delivery::from_spoken(spoken),
                Delivery::Queue,
                "{spoken} should queue"
            );
        }
    }

    #[test]
    fn ambiguity_queues_rather_than_destroying_work() {
        // The lopsided-cost rule: unclear input must never interrupt.
        assert_eq!(Delivery::from_spoken(""), Delivery::Queue);
        assert_eq!(Delivery::from_spoken("update the readme"), Delivery::Queue);
        // "stop" inside a word is not a command to stop.
        assert_eq!(
            Delivery::from_spoken("add a nonstop scrolling banner"),
            Delivery::Queue
        );
        assert_eq!(
            Delivery::from_spoken("make it wait for the response"),
            Delivery::Queue
        );
        // A deferring phrase wins even when a stop word is present, because the
        // sentence is explicitly about what happens next.
        assert_eq!(
            Delivery::from_spoken("after that, stop the dev server"),
            Delivery::Queue
        );
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
