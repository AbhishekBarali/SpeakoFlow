//! An Agent Client Protocol client, so one implementation drives every agent.
//!
//! ACP is JSON-RPC 2.0 over the agent's stdin and stdout, one JSON object per
//! line. SpeakoFlow is the *client* (the editor, in ACP's terms) and the coding
//! agent is the server. That direction is the whole point: we are the remote
//! control, and the agent does the coding.
//!
//! ## Why this is hand-rolled
//!
//! The official `agent-client-protocol` crate exists and is stable at 1.0. It is
//! not used here for two concrete reasons:
//!
//! 1. **Paradigm.** This module is built on `std::process` plus one blocking
//!    reader thread per session, which is what the existing Claude driver does
//!    and what its header documents as a deliberate choice. The crate is async
//!    and would pull tokio's `process` and `io-util` features in to run a second
//!    concurrency model beside the first.
//! 2. **Tolerance.** Strict typed deserialisation is a liability against real
//!    agents. Kiro emits `_kiro.dev/`-prefixed methods and extra `_meta` objects
//!    that are not in the schema; a client that must parse every frame into a
//!    known type has to be perfect about optional fields to avoid dropping
//!    events. Reading `serde_json::Value` and looking only for the fields we act
//!    on degrades gracefully by construction, which is exactly what the spec's
//!    own extensibility rules ask for.
//!
//! Revisit this if we ever need the parts of ACP the crate does own well —
//! notably its MCP bridging. The wire format is simple; the vendor variance is
//! the hard part, and that is what this handles.
//!
//! ## Sequencing
//!
//! `initialize` → `session/new` → `session/prompt` must happen in order, and
//! each step needs the previous response. Rather than block the caller on three
//! round trips, the handshake runs as a state machine *on the reader thread*:
//! `start` writes `initialize` and returns immediately, and each response drives
//! the next request. Nothing blocks, nothing needs a timeout, and the session
//! shows as `Starting` until it can really accept work.
//!
//! Anything the user says before the session is ready is queued and sent when it
//! is, so talking to an agent that is still booting works instead of failing.

use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdin, Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

use super::policy::{pick_option, ApprovalPolicy, ToolKind, ToolTracker, Verdict};
use super::registry::AgentKind;
use super::{
    announce, emit_changed, env, truncate, write_line, AgentStatus, Delivery, PendingApproval,
    SessionState, FILES_BUDGET, LINE_BUDGET,
};

/// The ACP major version this client speaks.
const PROTOCOL_VERSION: u64 = 1;

/// Fixed ids for the two handshake calls, so responses are recognisable without
/// bookkeeping.
const INITIALIZE_ID: u64 = 1;
const NEW_SESSION_ID: u64 = 2;
/// Everything after the handshake counts up from here.
const FIRST_CALL_ID: u64 = 10;

/// Cap on how much of a tool title to keep.
const TITLE_BUDGET: usize = 120;

/// A permission request we are parked on.
///
/// The id is kept as the raw JSON value it arrived as. Verified against
/// `kiro-cli acp` 2.18.1, which sends a UUID *string*: JSON-RPC allows string or
/// number ids, and a client that assumes integers answers into the void.
struct Parked {
    request_id: Value,
    options: Vec<Value>,
}

/// Turn scheduling. Exactly one `session/prompt` may be in flight at a time.
#[derive(Default)]
struct Turns {
    /// Request id of the running turn.
    in_flight: Option<u64>,
    /// Work waiting for the current turn, or for the session to open.
    queued: VecDeque<String>,
    /// Text that a cancel was issued to make room for. Jumps the queue once the
    /// cancelled turn settles.
    interrupting: Option<String>,
}

/// The agent's session modes, which are how ACP exposes personas.
///
/// Kiro returns its whole agent list here — `kiro_default`, `kiro_planner`,
/// `research`, and any user-defined ones — so "switch to the planner" is a
/// protocol call rather than a feature.
#[derive(Default, Debug, Clone)]
pub struct Modes {
    pub current: Option<String>,
    /// `(id, description)` pairs.
    pub available: Vec<(String, String)>,
}

/// One switchable dimension of a live session.
///
/// This is ACP's `configOptions` surface, which supersedes `modes` and is the
/// **only** control the spec explicitly allows changing while a turn is running:
/// "The current value of a config option can be changed at any point during a
/// session, whether the Agent is idle or generating a response."
///
/// That sentence is the answer to "can you switch the model while it's
/// building?" — yes, and without throwing away the work in progress.
#[derive(Debug, Clone)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    /// `mode`, `model`, `model_config`, `thought_level`, or a vendor value
    /// beginning with `_`. The spec is explicit that categories are UX metadata
    /// and "MUST NOT be required for correctness", so this only ever *helps*
    /// find the right option — never gates it.
    pub category: Option<String>,
    pub is_boolean: bool,
    pub current: String,
    /// `(value, label)` pairs. Empty for a boolean option.
    pub values: Vec<(String, String)>,
}

/// A slash command the agent advertises for this session.
///
/// Commands are not a separate protocol method: per the spec they are "included
/// as regular user messages in prompt requests", so running one means sending
/// `/name args` as prompt text. Verified against `kiro-cli acp` 2.18.1, which
/// answered `/usage` with `stopReason: end_turn`.
#[derive(Debug, Clone)]
pub struct Command {
    /// Without the leading slash.
    pub name: String,
    pub description: String,
    /// What the command expects after its name, when the agent says.
    pub hint: Option<String>,
}

/// How much of a session's control surface a voice command may touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRisk {
    /// Read-only or additive: safe to run because the user asked.
    Safe,
    /// Throws away history, ends the session, or removes a safety control.
    /// Never run from the voice path.
    Refused,
}

/// Classify a slash command for voice use.
///
/// Fail closed on the small set that destroys context or disables approvals,
/// allow the rest. The deny-list is deliberately about *consequence*, not about
/// which agent is behind the session: `/clear` and `/rewind` discard work the
/// user cannot get back, `/quit` ends the session mid-task, and
/// `/tools trust-all` switches off the approval policy that makes the whole
/// feature safe — which is exactly what the design notes say never to ship as a
/// default.
pub fn command_risk(name: &str, args: Option<&str>) -> CommandRisk {
    let name = name.trim().trim_start_matches('/').to_lowercase();
    let args = args.unwrap_or_default().trim().to_lowercase();
    match name.as_str() {
        "quit" | "exit" | "clear" | "rewind" => CommandRisk::Refused,
        // `/tools trust-all` disables the approval policy; plain `/tools` lists.
        "tools" if args.starts_with("trust") => CommandRisk::Refused,
        // Loading another conversation over this one loses the current thread.
        "chat" if args.starts_with("load") || args.starts_with("new") => CommandRisk::Refused,
        // `/context clear` drops files the agent was told to keep in mind.
        "context" if args.starts_with("clear") || args.starts_with("remove") => {
            CommandRisk::Refused
        }
        "knowledge" if args.starts_with("clear") || args.starts_with("remove") => {
            CommandRisk::Refused
        }
        _ => CommandRisk::Safe,
    }
}

/// Everything about a session that can be inspected or changed while it runs.
#[derive(Default, Debug, Clone)]
pub struct Controls {
    /// The spec's preferred surface, when the agent offers it.
    pub config: Vec<ConfigOption>,
    /// Current model id, from `models.currentModelId` or the `model` config option.
    pub model: Option<String>,
    /// `(id, description)` pairs from `models.availableModels`.
    pub models: Vec<(String, String)>,
    pub commands: Vec<Command>,
    /// How full the context window is, when the agent volunteers it.
    pub context_percent: Option<f64>,
    /// Reasoning effort, when the agent volunteers it.
    pub effort: Option<String>,
}

impl Controls {
    /// The config option covering a semantic category, if the agent sent one.
    fn by_category(&self, category: &str) -> Option<&ConfigOption> {
        self.config
            .iter()
            .find(|option| option.category.as_deref() == Some(category))
    }

    /// Whether a command with this name is advertised.
    fn has_command(&self, name: &str) -> bool {
        let needle = name.trim().trim_start_matches('/').to_lowercase();
        self.commands
            .iter()
            .any(|command| command.name.to_lowercase() == needle)
    }

    /// Every model this session can switch to, from whichever surface has them.
    fn model_choices(&self) -> Vec<(String, String)> {
        if let Some(option) = self.by_category("model") {
            if !option.values.is_empty() {
                return option.values.clone();
            }
        }
        self.models.clone()
    }
}

/// What a control request expects back, so a reply can be applied or undone.
enum Ack {
    /// `session/set_config_option` answers with the complete config state.
    Config,
    /// `session/set_model` and `session/set_mode` answer with nothing useful, so
    /// the change is applied optimistically and rolled back if it errored.
    Model {
        previous: Option<String>,
    },
    Mode {
        previous: Option<String>,
    },
}

/// What a session should be configured to when it opens.
///
/// Applied after `session/new`, not as CLI flags: `--model` and `--effort` exist
/// on `kiro-cli acp` but not on every adapter, and the protocol route works the
/// same everywhere. One code path, no per-agent flag table.
#[derive(Default, Debug, Clone)]
pub struct Wanted {
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl Wanted {
    fn is_empty(&self) -> bool {
        self.model.is_none() && self.effort.is_none()
    }
}

/// Shared session machinery, owned jointly by the reader thread and the caller.
struct Acp {
    kind: AgentKind,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    next_id: AtomicU64,
    /// The agent's session id, once `session/new` has answered. `None` means the
    /// session cannot accept work yet.
    session: Mutex<Option<String>>,
    turns: Mutex<Turns>,
    tracker: Mutex<ToolTracker>,
    policy: Mutex<ApprovalPolicy>,
    parked: Mutex<Option<Parked>>,
    modes: Mutex<Modes>,
    /// Models, config options, slash commands and live usage.
    controls: Mutex<Controls>,
    /// Control requests waiting on a reply, so an error can be undone.
    acks: Mutex<HashMap<u64, Ack>>,
    /// Model and effort asked for at start, applied once a session exists.
    wanted: Mutex<Wanted>,
    /// Whether the user has already been told the context window is filling up.
    context_warned: AtomicBool,
    /// Accumulates the streaming assistant message for the digest.
    line_buf: Mutex<String>,
    /// Shared with the manager; the reader thread is its main writer.
    state: Arc<Mutex<SessionState>>,
    app: AppHandle,
}

/// Handle the manager keeps for a live ACP session.
pub struct AcpHandle {
    inner: Arc<Acp>,
}

impl AcpHandle {
    /// Spawn an agent and begin the handshake.
    ///
    /// Returns as soon as the process is running and `initialize` has been
    /// written. The session reports `Starting` until `session/new` answers.
    /// `task` is queued as the first turn.
    pub fn start(
        app: &AppHandle,
        kind: AgentKind,
        state: Arc<Mutex<SessionState>>,
        task: &str,
        policy: ApprovalPolicy,
        wanted: Wanted,
    ) -> Result<(Self, Child), String> {
        let (binary, args) = kind.command()?;
        let cwd = state.lock().unwrap().cwd.clone();

        let forwarded = env::forwarded_vars();
        if !forwarded.is_empty() {
            // Names only — these are credentials.
            log::info!(
                "acp: forwarding {} provider variable(s): {}",
                forwarded.len(),
                forwarded
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        log::info!(
            "acp: starting {} via {} {}",
            kind.label(),
            binary.display(),
            args.join(" ")
        );

        let mut command = ProcessCommand::new(&binary);
        command
            .args(&args)
            .current_dir(&cwd)
            .env("PATH", env::effective_path())
            .envs(forwarded)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW: without it every session flashes a console.
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }

        let mut child = command.spawn().map_err(|e| {
            format!(
                "Could not start {} ({}). {}",
                kind.label(),
                e,
                kind.install_hint()
            )
        })?;

        let stdin = Arc::new(Mutex::new(child.stdin.take()));
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let mut turns = Turns::default();
        let task = task.trim();
        if !task.is_empty() {
            turns.queued.push_back(task.to_string());
        }

        let inner = Arc::new(Acp {
            kind,
            stdin,
            next_id: AtomicU64::new(FIRST_CALL_ID),
            session: Mutex::new(None),
            turns: Mutex::new(turns),
            tracker: Mutex::new(ToolTracker::default()),
            policy: Mutex::new(policy),
            parked: Mutex::new(None),
            modes: Mutex::new(Modes::default()),
            controls: Mutex::new(Controls::default()),
            acks: Mutex::new(HashMap::new()),
            wanted: Mutex::new(wanted),
            context_warned: AtomicBool::new(false),
            line_buf: Mutex::new(String::new()),
            state,
            app: app.clone(),
        });

        // Client capabilities are declared honestly: we do not serve the agent's
        // filesystem or terminal requests, so it must use its own tools. Saying
        // otherwise would make the agent wait on methods we answer with errors.
        //
        // `session.configOptions.boolean` is the exception we *do* claim, because
        // we genuinely handle it: without advertising it an agent must hide its
        // boolean switches from us, per the spec ("Agents MUST NOT include
        // type: boolean options unless the Client advertised support").
        inner.request(
            INITIALIZE_ID,
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false,
                    "session": { "configOptions": { "boolean": {} } }
                },
                "clientInfo": { "name": "SpeakoFlow", "version": env!("CARGO_PKG_VERSION") }
            }),
        )?;

        if let Some(stdout) = stdout {
            let acp = Arc::clone(&inner);
            std::thread::spawn(move || read_loop(acp, stdout));
        }
        if let Some(stderr) = stderr {
            let acp = Arc::clone(&inner);
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    // Agents log routine chatter here, so this is not an error
                    // channel. It is recorded but does not fail the session.
                    log::debug!("acp stderr: {}", truncate(&line, 300));
                    acp.state.lock().unwrap().error = Some(truncate(&line, LINE_BUDGET));
                }
            });
        }

        Ok((Self { inner }, child))
    }

    /// Send an instruction, either queued behind the current turn or interrupting it.
    pub fn submit(&self, text: &str, delivery: Delivery) -> Result<(), String> {
        self.inner.submit(text, delivery)
    }

    /// Ask the agent to stop the running turn.
    pub fn cancel(&self) -> Result<(), String> {
        self.inner.cancel()
    }

    /// Answer the permission request the session is parked on.
    pub fn answer(&self, allow: bool) -> Result<String, String> {
        let (request_id, option_id) = {
            let parked = self.inner.parked.lock().unwrap();
            let parked = parked.as_ref().ok_or_else(|| {
                "That session is not waiting on a permission request.".to_string()
            })?;
            let option = pick_option(&parked.options, allow).ok_or_else(|| {
                format!(
                    "{} didn't offer an option we recognise, so it has to be answered in its own window.",
                    self.inner.kind.label()
                )
            })?;
            (parked.request_id.clone(), option)
        };
        self.inner.respond(
            &request_id,
            json!({ "outcome": { "outcome": "selected", "optionId": option_id } }),
        )?;
        *self.inner.parked.lock().unwrap() = None;
        Ok(option_id_label(&option_id, allow))
    }

    /// Switch the agent's mode (its persona, in Kiro's case).
    ///
    /// The spec supersedes `modes` with a `mode`-category config option and says
    /// clients that understand config options should prefer it, so that route is
    /// tried first and `session/set_mode` is the fallback.
    pub fn set_mode(&self, mode: &str) -> Result<String, String> {
        let session = self.inner.session_id()?;
        let controls = self.inner.controls.lock().unwrap().clone();
        if let Some(option) = controls.by_category("mode") {
            if !option.values.is_empty() {
                let chosen = match_choice(&option.values, mode).ok_or_else(|| {
                    format!(
                        "No mode matches \"{}\". Available: {}.",
                        mode.trim(),
                        option
                            .values
                            .iter()
                            .map(|(id, _)| id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
                self.inner.set_config_option(&session, option, &chosen)?;
                self.inner.modes.lock().unwrap().current = Some(chosen.clone());
                return Ok(chosen);
            }
        }

        let available = self.inner.modes.lock().unwrap().available.clone();
        if available.is_empty() {
            return Err(format!(
                "{} didn't offer any modes.",
                self.inner.kind.label()
            ));
        }
        let chosen = match_choice(&available, mode).ok_or_else(|| {
            format!(
                "No mode matches \"{}\". Available: {}.",
                mode.trim(),
                available
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let previous = self.inner.modes.lock().unwrap().current.clone();
        self.inner
            .acks
            .lock()
            .unwrap()
            .insert(id, Ack::Mode { previous });
        self.inner.request(
            id,
            "session/set_mode",
            json!({ "sessionId": session, "modeId": chosen }),
        )?;
        self.inner.modes.lock().unwrap().current = Some(chosen.clone());
        Ok(chosen)
    }

    /// The modes this agent offers, for listing out loud.
    pub fn modes(&self) -> Modes {
        self.inner.modes.lock().unwrap().clone()
    }

    /// Everything switchable about this session right now.
    pub fn controls(&self) -> Controls {
        self.inner.controls.lock().unwrap().clone()
    }

    /// Switch the model this session is using, without disturbing its work.
    ///
    /// See [`Acp::route_model`] for the order the routes are tried in. When the
    /// only route is the agent's own `/model` command, the switch queues like any
    /// other instruction, because a command is a prompt.
    pub fn set_model(&self, wanted: &str) -> Result<String, String> {
        let session = self.inner.session_id()?;
        let resolved = {
            let controls = self.inner.controls.lock().unwrap();
            let choices = controls.model_choices();
            match_choice(&choices, wanted).unwrap_or_else(|| wanted.trim().to_string())
        };
        if let Some(command) = self.inner.route_model(&session, wanted)? {
            self.submit(&command, Delivery::Queue)?;
        }
        Ok(resolved)
    }

    /// Set how hard the model should think, where the agent exposes it.
    ///
    /// Returns the level and whether it had to go through the agent's own
    /// `/effort` command. That distinction is reported rather than hidden because
    /// the command route is **unconfirmed**: `kiro-cli acp` accepts
    /// `/effort low` with `stopReason: end_turn`, but its metadata then stops
    /// reporting an effort value instead of reporting the new one, and `/effort`
    /// is declared `inputType: selection` — a picker in the terminal, which may
    /// ignore an inline argument. Saying "done" on that evidence would be a
    /// guess dressed as a fact.
    pub fn set_effort(&self, level: &str) -> Result<(String, bool), String> {
        let session = self.inner.session_id()?;
        let via_command = match self.inner.route_effort(&session, level)? {
            Some(command) => {
                self.submit(&command, Delivery::Queue)?;
                true
            }
            None => false,
        };
        Ok((level.trim().to_lowercase(), via_command))
    }

    /// Run one of the agent's own slash commands.
    ///
    /// Commands travel as prompt text, so they take a turn slot like any other
    /// instruction. That is why `delivery` matters: `/compact` said while the
    /// agent is working should wait for the current turn rather than cancel it.
    pub fn run_command(
        &self,
        name: &str,
        args: Option<&str>,
        delivery: Delivery,
    ) -> Result<(), String> {
        let name = name.trim().trim_start_matches('/');
        if name.is_empty() {
            return Err("Which command?".to_string());
        }
        if command_risk(name, args) == CommandRisk::Refused {
            return Err(format!(
                "/{} discards work or switches off an approval, so it can't be run by voice.",
                name
            ));
        }
        let controls = self.inner.controls.lock().unwrap().clone();
        // An empty list means the agent never advertised any, not that it has
        // none — so only reject when we actually know better.
        if !controls.commands.is_empty() && !controls.has_command(name) {
            return Err(format!(
                "{} has no /{} command. It offers: {}.",
                self.inner.kind.label(),
                name,
                controls
                    .commands
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let text = match args.map(str::trim).filter(|a| !a.is_empty()) {
            Some(args) => format!("/{} {}", name, args),
            None => format!("/{}", name),
        };
        self.submit(&text, delivery)
    }

    /// Replace the auto-approval settings for this session.
    pub fn set_policy(&self, policy: ApprovalPolicy) {
        *self.inner.policy.lock().unwrap() = policy;
    }

    /// Whether the session can accept work yet.
    pub fn is_ready(&self) -> bool {
        self.inner.session.lock().unwrap().is_some()
    }
}

/// The parts of a session the event mapper touches.
///
/// Deliberately excludes the Tauri handle: mapping protocol events onto the
/// digest is the logic most worth testing, and it should not require a running
/// application to exercise. Emitting is the caller's job.
struct Digest<'a> {
    state: &'a Arc<Mutex<SessionState>>,
    tracker: &'a Mutex<ToolTracker>,
    line_buf: &'a Mutex<String>,
    modes: &'a Mutex<Modes>,
    controls: &'a Mutex<Controls>,
}

impl Acp {
    fn digest(&self) -> Digest<'_> {
        Digest {
            state: &self.state,
            tracker: &self.tracker,
            line_buf: &self.line_buf,
            modes: &self.modes,
            controls: &self.controls,
        }
    }

    /// Write a JSON-RPC request.
    fn request(&self, id: u64, method: &str, params: Value) -> Result<(), String> {
        write_line(
            &self.stdin,
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
    }

    /// Write a JSON-RPC notification, which expects no answer.
    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        write_line(
            &self.stdin,
            &json!({ "jsonrpc": "2.0", "method": method, "params": params }),
        )
    }

    /// Answer a request the agent made of us.
    fn respond(&self, id: &Value, result: Value) -> Result<(), String> {
        write_line(
            &self.stdin,
            &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        )
    }

    /// The agent's session id, or a sentence explaining why there isn't one yet.
    fn session_id(&self) -> Result<String, String> {
        self.session
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "That session is still starting up.".to_string())
    }

    /// Whether the context window is far enough along to be worth mentioning,
    /// and only the first time it happens.
    ///
    /// A latch rather than a threshold check at the call site, because the
    /// metadata event that carries the percentage arrives many times a turn and
    /// nobody wants to be told twice a second.
    fn take_context_warning(&self, threshold: f64) -> Option<f64> {
        let percent = self.controls.lock().unwrap().context_percent?;
        if percent < threshold {
            return None;
        }
        if self.context_warned.swap(true, Ordering::Relaxed) {
            return None;
        }
        Some(percent)
    }

    /// Send an instruction, either queued behind the current turn or interrupting it.
    fn submit(&self, text: &str, delivery: Delivery) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("There was nothing to send.".to_string());
        }
        let interrupt_now = {
            let mut turns = self.turns.lock().unwrap();
            match delivery {
                Delivery::Queue => {
                    turns.queued.push_back(text.to_string());
                    false
                }
                Delivery::Interrupt => {
                    if turns.in_flight.is_some() {
                        // Cancel first; the text goes in once the turn settles,
                        // because a prompt sent into a running turn is only
                        // queued by the agent anyway.
                        turns.interrupting = Some(text.to_string());
                        true
                    } else {
                        turns.queued.push_front(text.to_string());
                        false
                    }
                }
            }
        };
        if interrupt_now {
            self.cancel()?;
            return Ok(());
        }
        self.pump();
        Ok(())
    }

    /// Apply the model and effort the session was asked to start with.
    ///
    /// Runs on the reader thread, the moment a session id exists and before the
    /// first turn is sent, so the work starts on the model the user asked for
    /// rather than switching part-way through it.
    fn apply_wanted(&self, session: &str) {
        let wanted = std::mem::take(&mut *self.wanted.lock().unwrap());
        if wanted.is_empty() {
            return;
        }
        // Commands are prompts, so anything that has to go that route is put in
        // front of the task rather than sent as a request. Built in order and
        // inserted at the front together, so the task still runs last.
        let mut prefix: Vec<String> = Vec::new();
        if let Some(model) = wanted.model.as_deref() {
            match self.route_model(session, model) {
                Ok(Some(command)) => prefix.push(command),
                Ok(None) => {}
                Err(e) => log::warn!("acp: couldn't start on model {}: {}", model, e),
            }
        }
        if let Some(effort) = wanted.effort.as_deref() {
            match self.route_effort(session, effort) {
                Ok(Some(command)) => prefix.push(command),
                Ok(None) => {}
                Err(e) => log::warn!("acp: couldn't start at effort {}: {}", effort, e),
            }
        }
        if !prefix.is_empty() {
            let mut turns = self.turns.lock().unwrap();
            for text in prefix.into_iter().rev() {
                turns.queued.push_front(text);
            }
        }
    }

    /// Switch the model, returning a slash command to send if that is the only
    /// route this agent offers.
    ///
    /// Three routes, in order of how well the protocol supports them:
    ///
    /// 1. `session/set_config_option` on the option whose category is `model` —
    ///    the spec's preferred surface, and explicitly legal mid-turn.
    /// 2. `session/set_model` — the older dedicated method. Verified working
    ///    against `kiro-cli acp` 2.18.1: the call returned `{}` and a following
    ///    `session/load` reported the new `currentModelId`, so the switch is real
    ///    and not merely accepted.
    /// 3. The agent's own `/model` command, for agents that expose the choice
    ///    only to a human.
    fn route_model(&self, session: &str, wanted: &str) -> Result<Option<String>, String> {
        let wanted = wanted.trim();
        if wanted.is_empty() {
            return Err("Which model should it use?".to_string());
        }
        let controls = self.controls.lock().unwrap().clone();
        let choices = controls.model_choices();

        // With a list, only a listed id may be sent: inventing one gets an error
        // from the agent at best and a silently wrong model at worst.
        let chosen = if choices.is_empty() {
            wanted.to_string()
        } else {
            match_choice(&choices, wanted).ok_or_else(|| {
                format!(
                    "No model matches \"{}\". This session offers: {}.",
                    wanted,
                    choices
                        .iter()
                        .map(|(id, _)| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?
        };

        if let Some(option) = controls.by_category("model") {
            self.set_config_option(session, option, &chosen)?;
            return Ok(None);
        }
        if !controls.models.is_empty() {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            self.acks.lock().unwrap().insert(
                id,
                Ack::Model {
                    previous: controls.model.clone(),
                },
            );
            self.request(
                id,
                "session/set_model",
                json!({ "sessionId": session, "modelId": chosen }),
            )?;
            self.controls.lock().unwrap().model = Some(chosen);
            return Ok(None);
        }
        if controls.has_command("model") {
            return Ok(Some(format!("/model {chosen}")));
        }
        Err(format!(
            "{} didn't offer a way to change its model.",
            self.kind.label()
        ))
    }

    /// Set how hard the model should think, where the agent exposes it.
    ///
    /// Kiro calls this "effort" and takes `low` … `max`; the spec's category for
    /// the same idea is `thought_level`. Both are handled, and the slash command
    /// is the fallback.
    fn route_effort(&self, session: &str, level: &str) -> Result<Option<String>, String> {
        let level = level.trim().to_lowercase();
        if level.is_empty() {
            return Err("Which effort level?".to_string());
        }
        let controls = self.controls.lock().unwrap().clone();
        if let Some(option) = controls.by_category("thought_level") {
            let chosen = if option.values.is_empty() {
                level
            } else {
                match_choice(&option.values, &level).ok_or_else(|| {
                    format!(
                        "No effort level matches \"{}\". This session offers: {}.",
                        level,
                        option
                            .values
                            .iter()
                            .map(|(id, _)| id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?
            };
            self.set_config_option(session, option, &chosen)?;
            return Ok(None);
        }
        if controls.has_command("effort") {
            self.controls.lock().unwrap().effort = Some(level.clone());
            return Ok(Some(format!("/effort {level}")));
        }
        Err(format!(
            "{} didn't offer a thinking-effort setting.",
            self.kind.label()
        ))
    }

    /// Change one config option, optimistically and immediately.
    ///
    /// Sent whether or not a turn is running, which the spec allows explicitly.
    /// The reply carries the complete config state, so [`Ack::Config`] replaces
    /// what we hold rather than patching it — that is how an agent tells us a
    /// dependent option changed too (picking a model can change which reasoning
    /// levels exist).
    fn set_config_option(
        &self,
        session: &str,
        option: &ConfigOption,
        value: &str,
    ) -> Result<(), String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut params = json!({
            "sessionId": session,
            "configId": option.id,
            "value": value,
        });
        if option.is_boolean {
            let truthy = matches!(
                value.trim().to_lowercase().as_str(),
                "true" | "on" | "yes" | "1"
            );
            params["type"] = json!("boolean");
            params["value"] = json!(truthy);
        }
        self.acks.lock().unwrap().insert(id, Ack::Config);
        self.request(id, "session/set_config_option", params)?;
        // Optimistic, so a spoken confirmation is not gated on a round trip. The
        // reply replaces this wholesale, and an error rolls the session's own
        // value back into view on the next update.
        if let Some(held) = self
            .controls
            .lock()
            .unwrap()
            .config
            .iter_mut()
            .find(|held| held.id == option.id)
        {
            held.current = value.to_string();
        }
        if option.category.as_deref() == Some("model") {
            self.controls.lock().unwrap().model = Some(value.to_string());
        }
        if option.category.as_deref() == Some("thought_level") {
            self.controls.lock().unwrap().effort = Some(value.to_string());
        }
        Ok(())
    }

    /// Decline a request we do not implement. Required rather than optional: an
    /// unanswered request leaves the agent waiting forever.
    fn respond_error(&self, id: &Value, message: &str) {
        let _ = write_line(
            &self.stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": message }
            }),
        );
    }

    /// `session/cancel` is a notification: the running turn then settles with a
    /// `cancelled` stop reason, which is where the interrupt is picked up.
    fn cancel(&self) -> Result<(), String> {
        let session = self
            .session
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "That session is still starting up.".to_string())?;
        self.notify("session/cancel", json!({ "sessionId": session }))
    }

    /// Send the next queued turn, if the session is idle and ready.
    fn pump(&self) {
        // A handed-off or closed session keeps its queue but sends nothing: the
        // process behind it is gone.
        if !self.state.lock().unwrap().status.is_live() {
            return;
        }
        let session = { self.session.lock().unwrap().clone() };
        let Some(session) = session else {
            // Not ready. Whatever is queued goes out when session/new answers.
            return;
        };
        let (id, text) = {
            let mut turns = self.turns.lock().unwrap();
            if turns.in_flight.is_some() {
                return;
            }
            let Some(text) = turns.queued.pop_front() else {
                return;
            };
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            turns.in_flight = Some(id);
            (id, text)
        };

        self.line_buf.lock().unwrap().clear();
        let sent = self.request(
            id,
            "session/prompt",
            json!({
                "sessionId": session,
                "prompt": [{ "type": "text", "text": text }]
            }),
        );
        match sent {
            Ok(()) => {
                let mut s = self.state.lock().unwrap();
                s.status = AgentStatus::Working;
                s.error = None;
                drop(s);
                emit_changed(&self.app, &self.state);
            }
            Err(e) => {
                self.turns.lock().unwrap().in_flight = None;
                self.fail(&format!("Couldn't send to {}: {}", self.kind.label(), e));
            }
        }
    }

    /// Mark the session failed with a message safe to read aloud.
    fn fail(&self, message: &str) {
        {
            let mut s = self.state.lock().unwrap();
            if !s.status.is_live() {
                return;
            }
            s.status = AgentStatus::Failed;
            s.error = Some(truncate(message, LINE_BUDGET));
            s.pending = None;
        }
        emit_changed(&self.app, &self.state);
    }
}

/// The reader thread: the only writer of session state, and the driver of the
/// handshake state machine.
fn read_loop(acp: Arc<Acp>, stdout: std::process::ChildStdout) {
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let message = match serde_json::from_str::<Value>(&line) {
            Ok(value) => value,
            Err(_) => {
                // Some agents print a banner before speaking protocol.
                log::debug!("acp: ignoring non-JSON line: {}", truncate(&line, 200));
                continue;
            }
        };

        let before = acp.state.lock().unwrap().status;
        let changed = dispatch(&acp, &message);
        warn_about_context(&acp);
        if changed {
            let view = acp.state.lock().unwrap().view();
            if view.status != before {
                announce(&acp.app, before, &view);
            }
            let _ = acp.app.emit("agent-session-update", view);
        }
    }

    // stdout closed: the process is gone.
    {
        let mut s = acp.state.lock().unwrap();
        if s.status.is_live() {
            s.status = AgentStatus::Ended;
            s.pending = None;
        }
    }
    emit_changed(&acp.app, &acp.state);
}

/// Route one frame. Returns whether the session digest changed.
fn dispatch(acp: &Arc<Acp>, message: &Value) -> bool {
    if let Some(method) = message.get("method").and_then(Value::as_str) {
        return handle_incoming(acp, method, message);
    }
    if message.get("result").is_some() || message.get("error").is_some() {
        return handle_response(acp, message);
    }
    false
}

/// Whether `method` is `tail`, allowing for a vendor prefix.
///
/// Kiro sends `_kiro.dev/session/update` carrying chunk-level events *alongside*
/// the standard `session/update`. Both are real events, so both are handled.
fn is_method(method: &str, tail: &str) -> bool {
    method == tail || method.ends_with(&format!("/{tail}"))
}

fn handle_incoming(acp: &Arc<Acp>, method: &str, message: &Value) -> bool {
    if is_method(method, "session/update") {
        return match message.pointer("/params/update") {
            Some(update) => apply_update(&acp.digest(), update),
            None => false,
        };
    }
    if is_method(method, "session/request_permission") {
        return handle_permission(acp, message);
    }
    // Vendor telemetry. Read opportunistically, never depended on.
    if method.starts_with('_') && method.ends_with("/metadata") {
        return apply_metadata(&acp.digest(), message.pointer("/params"));
    }
    // Kiro's richer equivalent of `available_commands_update`. Same treatment:
    // useful, optional, and ignored if its shape ever changes.
    if method.starts_with('_') && method.contains("/commands/") {
        let commands = parse_commands(message.pointer("/params"));
        if !commands.is_empty() {
            log::debug!(
                "acp: {} advertises {} command(s)",
                acp.kind.label(),
                commands.len()
            );
            acp.controls.lock().unwrap().commands = commands;
        }
        return false;
    }
    // Notifications need no answer; requests do, or the agent stalls waiting.
    if let Some(id) = message.get("id") {
        log::debug!("acp: declining unsupported request {}", method);
        acp.respond_error(id, "SpeakoFlow does not implement this method");
    } else {
        log::trace!("acp: ignoring notification {}", method);
    }
    false
}

/// Map one `session/update` onto the digest.
fn apply_update(digest: &Digest<'_>, update: &Value) -> bool {
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or_default();

    match kind {
        "agent_message_chunk" => {
            let text = update
                .pointer("/content/text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if text.trim().is_empty() {
                return false;
            }
            let line = {
                let mut buffer = digest.line_buf.lock().unwrap();
                buffer.push_str(text);
                truncate(&buffer, LINE_BUDGET)
            };
            let mut s = digest.state.lock().unwrap();
            s.last_line = Some(line);
            if matches!(s.status, AgentStatus::Starting | AgentStatus::Idle) {
                s.status = AgentStatus::Working;
            }
            true
        }
        "agent_thought_chunk" => {
            // Thinking is activity, not content: it keeps the session looking
            // alive but never becomes the spoken summary.
            let mut s = digest.state.lock().unwrap();
            if matches!(s.status, AgentStatus::Starting) {
                s.status = AgentStatus::Working;
                return true;
            }
            false
        }
        "tool_call" | "tool_call_chunk" | "tool_call_update" => {
            let call_id = update
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let status = update.get("status").and_then(Value::as_str);
            let (title, paths) = {
                let mut tracker = digest.tracker.lock().unwrap();
                if !call_id.is_empty() {
                    tracker.observe(&call_id, update);
                }
                match tracker.get(&call_id) {
                    // Paths are only harvested once the call has actually
                    // completed. They arrive on the first `tool_call` event,
                    // while the call is still `pending` and may yet be refused,
                    // fail, or be cancelled — counting them then is what made a
                    // session report four files created into an empty folder.
                    // The tracker keeps accumulating either way, so a
                    // `tool_call_update` that only carries `status: completed`
                    // still finds the paths recorded earlier.
                    Some(info) => (
                        info.title.clone(),
                        if status == Some("completed") {
                            info.paths.clone()
                        } else {
                            Vec::new()
                        },
                    ),
                    None => (None, Vec::new()),
                }
            };

            let mut s = digest.state.lock().unwrap();
            if kind == "tool_call" {
                // Counted once per call: the chunk and the update refer to the
                // same call and would treble the number.
                s.tool_calls += 1;
                s.tool_error = None;
                digest.line_buf.lock().unwrap().clear();
            }
            if let Some(title) = title {
                s.last_tool = Some(truncate(&title, TITLE_BUDGET));
            }
            for path in paths {
                let name = display_path(&path, &s.cwd);
                if !s.files_touched.contains(&name) {
                    if s.files_touched.len() >= FILES_BUDGET {
                        s.files_touched.remove(0);
                    }
                    s.files_touched.push(name);
                }
            }
            match status {
                Some("failed") => {
                    s.tool_error = Some(truncate(
                        &tool_failure_text(update)
                            .unwrap_or_else(|| s.last_tool.clone().unwrap_or_default()),
                        LINE_BUDGET,
                    ));
                }
                Some("completed") => s.tool_error = None,
                _ => {}
            }
            if matches!(s.status, AgentStatus::Starting | AgentStatus::Idle) {
                s.status = AgentStatus::Working;
            }
            true
        }
        "current_mode_update" => {
            let mode = update
                .get("currentModeId")
                .and_then(Value::as_str)
                .map(str::to_string);
            if mode.is_some() {
                digest.modes.lock().unwrap().current = mode;
            }
            false
        }
        // What this session can be told to do. Kept so "what can you change?"
        // has a real answer, and so a command is never invented.
        "available_commands_update" => {
            let commands = parse_commands(Some(update));
            if !commands.is_empty() {
                digest.controls.lock().unwrap().commands = commands;
            }
            false
        }
        // The agent changed its own configuration — a rate-limit fallback to a
        // different model, or a mode switch after finishing a plan. Worth
        // knowing: otherwise a status report keeps naming the old model.
        "config_option_update" => {
            let config = parse_config_options(update.get("configOptions"));
            if config.is_empty() {
                return false;
            }
            let mut controls = digest.controls.lock().unwrap();
            controls.config = config;
            if let Some(model) = controls.by_category("model") {
                controls.model = Some(model.current.clone());
            }
            if let Some(effort) = controls.by_category("thought_level") {
                controls.effort = Some(effort.current.clone());
            }
            let mode = controls
                .by_category("mode")
                .map(|option| option.current.clone());
            drop(controls);
            if let Some(mode) = mode {
                digest.modes.lock().unwrap().current = Some(mode);
            }
            false
        }
        // The standard counterpart to Kiro's `_kiro.dev/metadata`: context window
        // size and cost for the session.
        "usage_update" => {
            let mut parts: Vec<String> = Vec::new();
            if let Some(percent) = usage_percent(update) {
                digest.controls.lock().unwrap().context_percent = Some(percent);
                parts.push(format!("{percent:.0}% context used"));
            }
            if let Some(cost) = update
                .pointer("/cost/amount")
                .or_else(|| update.get("cost"))
                .and_then(Value::as_f64)
                .filter(|amount| *amount > 0.0)
            {
                let currency = update
                    .pointer("/cost/currency")
                    .and_then(Value::as_str)
                    .unwrap_or("USD");
                parts.push(format!("{cost:.2} {currency}"));
            }
            if parts.is_empty() {
                return false;
            }
            digest.state.lock().unwrap().usage = Some(parts.join(", "));
            true
        }
        // `plan` and our own echoed `user_message_chunk` carry nothing the spoken
        // digest needs.
        _ => false,
    }
}

/// Percentage of the context window in use, from a `usage_update`.
///
/// The spec sends `size` (tokens used) and agents vary on whether they send a
/// maximum alongside it, so a percentage is only reported when both are present
/// or when the agent gives one directly. Guessing a window size would put a
/// wrong number into a spoken warning.
fn usage_percent(update: &Value) -> Option<f64> {
    if let Some(percent) = update
        .get("contextUsagePercentage")
        .and_then(Value::as_f64)
        .filter(|p| *p > 0.0)
    {
        return Some(percent);
    }
    let used = update.get("size").and_then(Value::as_f64)?;
    let total = update
        .get("maxSize")
        .or_else(|| update.get("contextWindow"))
        .or_else(|| update.pointer("/context/maxSize"))
        .and_then(Value::as_f64)
        .filter(|total| *total > 0.0)?;
    Some((used / total) * 100.0)
}

/// Pull a readable reason out of a failed tool call.
fn tool_failure_text(update: &Value) -> Option<String> {
    let content = update.get("content").and_then(Value::as_array)?;
    for entry in content {
        for pointer in ["/content/text", "/text"] {
            if let Some(text) = entry.pointer(pointer).and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    return Some(text.trim().to_string());
                }
            }
        }
    }
    None
}

/// Shorten a path for the digest: inside the project, the relative part is
/// enough, and it is what a person would say out loud.
fn display_path(path: &str, cwd: &str) -> String {
    let candidate = std::path::Path::new(path);
    let root = std::path::Path::new(cwd);
    if let Ok(relative) = candidate.strip_prefix(root) {
        let text = relative.to_string_lossy().to_string();
        if !text.is_empty() {
            return text;
        }
    }
    candidate
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

/// Vendor telemetry: cost and context usage, when an agent volunteers them.
///
/// Kiro reports credits rather than dollars, so this deliberately does not feed
/// `cost_usd` — mixing units in one number is how a status report starts lying.
/// It becomes a phrase instead, which is all a spoken summary needs.
fn apply_metadata(digest: &Digest<'_>, params: Option<&Value>) -> bool {
    let Some(params) = params else {
        return false;
    };
    let mut parts: Vec<String> = Vec::new();

    if let Some(usage) = params.get("meteringUsage").and_then(Value::as_array) {
        let mut total = 0.0;
        let mut unit = String::new();
        for entry in usage {
            if let Some(value) = entry.get("value").and_then(Value::as_f64) {
                total += value;
            }
            if let Some(found) = entry.get("unit").and_then(Value::as_str) {
                unit = found.to_string();
            }
        }
        if total > 0.0 {
            let unit = if unit.is_empty() {
                "units".to_string()
            } else if total == 1.0 {
                unit
            } else {
                format!("{unit}s")
            };
            parts.push(format!("{total:.2} {unit}"));
        }
    }
    if let Some(context) = params
        .get("contextUsagePercentage")
        .and_then(Value::as_f64)
        .filter(|p| *p > 0.0)
    {
        // Kept as a number as well as a phrase: a warning at 80% needs to compare
        // it, and re-parsing English to get a float back would be absurd.
        digest.controls.lock().unwrap().context_percent = Some(context);
        parts.push(format!("{context:.0}% context used"));
    }
    // Kiro reports the effort level here rather than as a config option, so this
    // is the only place we learn what it is actually set to.
    if let Some(effort) = params
        .get("effort")
        .and_then(Value::as_str)
        .filter(|effort| !effort.trim().is_empty())
    {
        digest.controls.lock().unwrap().effort = Some(effort.to_string());
    }

    if parts.is_empty() {
        return false;
    }
    digest.state.lock().unwrap().usage = Some(parts.join(", "));
    true
}

/// Decide and answer, or park the session for a human.
fn handle_permission(acp: &Arc<Acp>, message: &Value) -> bool {
    let Some(request_id) = message.get("id").cloned() else {
        // A permission request with no id cannot be answered.
        log::warn!("acp: permission request had no id; ignoring");
        return false;
    };
    let tool_call = message
        .pointer("/params/toolCall")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let call_id = tool_call
        .get("toolCallId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let options: Vec<Value> = message
        .pointer("/params/options")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // The request itself usually adds only the title, but fold it in anyway.
    if !call_id.is_empty() {
        acp.tracker.lock().unwrap().observe(&call_id, &tool_call);
    }

    // Correlate. This is the step that makes the decision meaningful: the
    // request does not say what the tool does, the earlier tool_call event did.
    let (verdict, detail, tool_name) = {
        let tracker = acp.tracker.lock().unwrap();
        let info = tracker.get(&call_id);
        let verdict = acp.policy.lock().unwrap().decide(info);
        let detail = info
            .and_then(|i| i.title.clone())
            .or_else(|| {
                tool_call
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "do something it hasn't described".to_string());
        let name = info
            .and_then(|i| i.kind)
            .map(kind_name)
            .unwrap_or("a tool")
            .to_string();
        (verdict, detail, name)
    };

    if let Verdict::AutoAllow(reason) = verdict {
        if let Some(option_id) = pick_option(&options, true) {
            match acp.respond(
                &request_id,
                json!({ "outcome": { "outcome": "selected", "optionId": option_id } }),
            ) {
                Ok(()) => {
                    log::info!("acp: auto-approved ({}): {}", reason, detail);
                    let mut s = acp.state.lock().unwrap();
                    s.auto_approvals += 1;
                    if matches!(s.status, AgentStatus::Starting | AgentStatus::Idle) {
                        s.status = AgentStatus::Working;
                    }
                    return true;
                }
                Err(e) => {
                    log::warn!("acp: couldn't send auto-approval, asking instead: {}", e);
                }
            }
        } else {
            log::warn!("acp: no recognised allow option; asking instead");
        }
    }

    let high_risk = matches!(verdict, Verdict::Ask { high_risk: true });
    *acp.parked.lock().unwrap() = Some(Parked {
        request_id: request_id.clone(),
        options,
    });
    let mut s = acp.state.lock().unwrap();
    s.pending = Some(PendingApproval {
        request_id: id_text(&request_id),
        tool_name,
        detail: truncate(&detail, LINE_BUDGET),
        high_risk,
    });
    s.status = AgentStatus::WaitingApproval;
    true
}

/// Handle a response to one of our own requests, driving the handshake forward.
fn handle_response(acp: &Arc<Acp>, message: &Value) -> bool {
    let id = message.get("id").and_then(Value::as_u64);
    let error = message.get("error");

    match id {
        Some(INITIALIZE_ID) => {
            if let Some(error) = error {
                acp.fail(&format!(
                    "{} refused to start: {}",
                    acp.kind.label(),
                    error_text(error)
                ));
                return true;
            }
            if let Some(name) = message
                .pointer("/result/agentInfo/name")
                .and_then(Value::as_str)
            {
                log::info!(
                    "acp: connected to {} {}",
                    name,
                    message
                        .pointer("/result/agentInfo/version")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                );
            }
            // Straight on to opening a session in the project folder.
            let cwd = acp.state.lock().unwrap().cwd.clone();
            if let Err(e) = acp.request(
                NEW_SESSION_ID,
                "session/new",
                json!({ "cwd": cwd, "mcpServers": [] }),
            ) {
                acp.fail(&format!("Couldn't open a session: {e}"));
                return true;
            }
            false
        }
        Some(NEW_SESSION_ID) => {
            if let Some(error) = error {
                acp.fail(&format!(
                    "{} couldn't open a session here: {}",
                    acp.kind.label(),
                    error_text(error)
                ));
                return true;
            }
            let Some(session_id) = message
                .pointer("/result/sessionId")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                acp.fail("The agent opened a session but didn't say which one.");
                return true;
            };

            if let Some(modes) = message.pointer("/result/modes") {
                let mut store = acp.modes.lock().unwrap();
                store.current = modes
                    .get("currentModeId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                store.available = modes
                    .get("availableModes")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(|mode| {
                                let id = mode.get("id").and_then(Value::as_str)?;
                                let description = mode
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                Some((id.to_string(), description.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }

            *acp.session.lock().unwrap() = Some(session_id.clone());
            let session_id_for_config = session_id.clone();
            acp.state.lock().unwrap().agent_session_id = Some(session_id);

            // What this session can be told to change, mid-flight. Both surfaces
            // are read: `configOptions` is the spec's direction of travel, and
            // `models` is what agents ship today (Kiro 2.18.1 sends `models` and
            // `modes`, no `configOptions`).
            {
                let (model, models) = parse_models(message.pointer("/result/models"));
                let config = parse_config_options(message.pointer("/result/configOptions"));
                let mut controls = acp.controls.lock().unwrap();
                controls.config = config;
                controls.models = models;
                controls.model = model.or_else(|| {
                    controls
                        .by_category("model")
                        .map(|option| option.current.clone())
                });
                if let Some(effort) = controls.by_category("thought_level") {
                    controls.effort = Some(effort.current.clone());
                }
                if !controls.models.is_empty() || !controls.config.is_empty() {
                    log::info!(
                        "acp: {} offers {} model(s), {} config option(s); current model {}",
                        acp.kind.label(),
                        controls.models.len(),
                        controls.config.len(),
                        controls.model.as_deref().unwrap_or("unreported")
                    );
                }
            }

            // Configure before the first turn, so the work starts on the model
            // that was asked for instead of switching part-way through it.
            acp.apply_wanted(&session_id_for_config);
            // Anything said while it was booting goes now.
            acp.pump();
            true
        }
        Some(other) => {
            // A control request we sent. Handled before the turn check because a
            // rejected model switch must not be mistaken for a finished turn.
            // Taken out of the map first: an `if let` on the guard would hold the
            // lock across the whole handler, which then takes others.
            let ack = acp.acks.lock().unwrap().remove(&other);
            if let Some(ack) = ack {
                return apply_ack(acp, ack, message, error);
            }
            let is_turn = { acp.turns.lock().unwrap().in_flight == Some(other) };
            if !is_turn {
                // A response to something we no longer track. Errors are worth
                // logging but do not change what the session is doing.
                if let Some(error) = error {
                    log::warn!("acp: request {} failed: {}", other, error_text(error));
                }
                return false;
            }
            finish_turn(acp, message, error);
            true
        }
        None => false,
    }
}

/// Apply the reply to a control request.
///
/// An agent is allowed to refuse — a model may be unavailable, a mode may be
/// gone. The optimistic value is rolled back in that case, because a status
/// report that says "now on Sonnet" when the switch failed is worse than no
/// report at all.
fn apply_ack(acp: &Arc<Acp>, ack: Ack, message: &Value, error: Option<&Value>) -> bool {
    match ack {
        Ack::Config => {
            if let Some(error) = error {
                log::warn!("acp: config option refused: {}", error_text(error));
                return false;
            }
            // The reply carries the complete state, which is the point: a model
            // change can alter which reasoning levels exist.
            let config = parse_config_options(message.pointer("/result/configOptions"));
            if config.is_empty() {
                return false;
            }
            let mut controls = acp.controls.lock().unwrap();
            controls.config = config;
            if let Some(model) = controls.by_category("model") {
                controls.model = Some(model.current.clone());
            }
            if let Some(effort) = controls.by_category("thought_level") {
                controls.effort = Some(effort.current.clone());
            }
            if let Some(mode) = controls.by_category("mode") {
                acp.modes.lock().unwrap().current = Some(mode.current.clone());
            }
            false
        }
        Ack::Model { previous } => {
            if let Some(error) = error {
                log::warn!("acp: model switch refused: {}", error_text(error));
                acp.controls.lock().unwrap().model = previous;
                let mut s = acp.state.lock().unwrap();
                s.error = Some(truncate(
                    &format!("Couldn't switch model: {}", error_text(error)),
                    LINE_BUDGET,
                ));
                return true;
            }
            false
        }
        Ack::Mode { previous } => {
            if let Some(error) = error {
                log::warn!("acp: mode switch refused: {}", error_text(error));
                acp.modes.lock().unwrap().current = previous;
                let mut s = acp.state.lock().unwrap();
                s.error = Some(truncate(
                    &format!("Couldn't switch mode: {}", error_text(error)),
                    LINE_BUDGET,
                ));
                return true;
            }
            false
        }
    }
}

/// Settle a finished turn and start whatever is next.
fn finish_turn(acp: &Arc<Acp>, message: &Value, error: Option<&Value>) {
    let stop_reason = message
        .pointer("/result/stopReason")
        .and_then(Value::as_str)
        .unwrap_or("end_turn")
        .to_string();

    {
        let mut turns = acp.turns.lock().unwrap();
        turns.in_flight = None;
        // An interrupt was waiting for this turn to settle. It jumps the queue,
        // because the user said it to change what is happening right now.
        if let Some(text) = turns.interrupting.take() {
            turns.queued.push_front(text);
        }
    }

    {
        let mut s = acp.state.lock().unwrap();
        // A session handed to a terminal, or already closed, must not be brought
        // back to life by a late frame from the process we just killed. Without
        // this the row flips from "in terminal" back to "working", which is the
        // opposite of what happened.
        if !s.status.is_live() {
            return;
        }
        s.pending = None;
        // The turn is over, so "Running: python hello.py" is no longer true.
        // Leaving it there made finished sessions look permanently busy.
        s.last_tool = None;
        if let Some(error) = error {
            s.status = AgentStatus::Failed;
            s.error = Some(truncate(&error_text(error), LINE_BUDGET));
        } else {
            match stop_reason.as_str() {
                "cancelled" => s.status = AgentStatus::Cancelled,
                "refusal" => {
                    // Not a broken session: the agent chose not to do one thing
                    // and is still there to be asked something else.
                    s.status = AgentStatus::Idle;
                    s.last_line = Some("The agent declined that request.".to_string());
                }
                "max_tokens" => {
                    s.status = AgentStatus::Failed;
                    s.error = Some("The agent ran out of context.".to_string());
                }
                "max_turn_requests" => {
                    s.status = AgentStatus::Failed;
                    s.error = Some("The agent hit its step limit for one turn.".to_string());
                }
                // `end_turn` and anything unrecognised: the turn is simply over.
                _ => s.status = AgentStatus::Idle,
            }
        }
    }
    *acp.parked.lock().unwrap() = None;
    // Sends the next queued turn if there is one, which also flips the status
    // back to Working.
    acp.pump();
}

/// Pick the value a user meant from a list of `(id, label)` pairs.
///
/// Spoken input is lossy, so this widens deliberately: exact id, then a
/// case-insensitive containment either way ("sonnet" → `claude-sonnet-5`), then
/// a match against the label. Dictation also inserts spaces and hyphens where
/// the id has neither, so both are stripped before comparing — "GPT 5.6 Luna"
/// has to reach `gpt-5.6-luna`.
fn match_choice(choices: &[(String, String)], wanted: &str) -> Option<String> {
    let needle = wanted.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    let squash = |text: &str| {
        text.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '.')
            .collect::<String>()
    };
    let squashed = squash(&needle);

    choices
        .iter()
        .find(|(id, _)| id.to_lowercase() == needle)
        .or_else(|| choices.iter().find(|(id, _)| squash(id) == squashed))
        .or_else(|| {
            choices
                .iter()
                .find(|(id, _)| squash(id).contains(&squashed) || squashed.contains(&squash(id)))
        })
        .or_else(|| {
            choices
                .iter()
                .find(|(_, label)| squash(label).contains(&squashed))
        })
        .map(|(id, _)| id.clone())
}

/// Read ACP's `configOptions` array into our own shape.
fn parse_config_options(value: Option<&Value>) -> Vec<ConfigOption> {
    let Some(list) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|option| {
            let id = option.get("id").and_then(Value::as_str)?.to_string();
            let kind = option
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("select")
                .to_lowercase();
            // An unrecognised type is skipped rather than guessed at, which is
            // what the spec asks for: the agent keeps using its default.
            if kind != "select" && kind != "boolean" {
                return None;
            }
            let current = match option.get("currentValue") {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Bool(flag)) => flag.to_string(),
                Some(other) => other.to_string(),
                None => String::new(),
            };
            Some(ConfigOption {
                name: option
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string(),
                category: option
                    .get("category")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                is_boolean: kind == "boolean",
                current,
                values: option
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|entry| {
                                let value = entry.get("value").and_then(Value::as_str)?;
                                let label = entry
                                    .get("name")
                                    .or_else(|| entry.get("description"))
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                Some((value.to_string(), label.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                id,
            })
        })
        .collect()
}

/// Read the older `models` (`SessionModelState`) block.
fn parse_models(value: Option<&Value>) -> (Option<String>, Vec<(String, String)>) {
    let Some(models) = value else {
        return (None, Vec::new());
    };
    let current = models
        .get("currentModelId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let available = models
        .get("availableModels")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|model| {
                    // `modelId` per the schema; `id` tolerated because agents
                    // have shipped both.
                    let id = model
                        .get("modelId")
                        .or_else(|| model.get("id"))
                        .and_then(Value::as_str)?;
                    let description = model
                        .get("description")
                        .or_else(|| model.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    Some((id.to_string(), description.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    (current, available)
}

/// Read a slash-command list.
///
/// Handles both the standard `availableCommands` array and Kiro's
/// `_kiro.dev/commands/available` payload, which uses `commands` and puts the
/// hint inside `meta`. Leading slashes are stripped so one name shape is stored.
fn parse_commands(params: Option<&Value>) -> Vec<Command> {
    let Some(params) = params else {
        return Vec::new();
    };
    let list = params
        .get("availableCommands")
        .or_else(|| params.get("commands"))
        .and_then(Value::as_array);
    let Some(list) = list else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|entry| {
            let name = entry.get("name").and_then(Value::as_str)?;
            let name = name.trim().trim_start_matches('/');
            if name.is_empty() {
                return None;
            }
            let hint = entry
                .pointer("/input/hint")
                .or_else(|| entry.pointer("/meta/hint"))
                .or_else(|| entry.pointer("/_meta/hint"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|hint| !hint.trim().is_empty());
            Some(Command {
                name: name.to_string(),
                description: entry
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                hint,
            })
        })
        .collect()
}

/// Percentage of context used at which the user is told, once per session.
///
/// Chosen to leave room to act: an agent that hits its window mid-task fails the
/// turn with `max_tokens` and loses the thread, and "you're nearly out" is only
/// useful while compacting is still an option.
const CONTEXT_WARN_PERCENT: f64 = 80.0;

/// Say something, once, when a session is running out of context window.
///
/// The whole premise is that the user is not watching, so a silent slide into
/// `max_tokens` is exactly the failure they cannot see coming. The notice names
/// the remedy rather than just the number, because "82% context" is not an
/// instruction.
fn warn_about_context(acp: &Arc<Acp>) {
    let Some(percent) = acp.take_context_warning(CONTEXT_WARN_PERCENT) else {
        return;
    };
    let (label, session_id) = {
        let s = acp.state.lock().unwrap();
        (s.label.clone(), s.id.clone())
    };
    let message = format!(
        "{} has used {:.0}% of its context. Say compact it and I'll have it summarise and carry on.",
        label, percent
    );
    let _ = acp.app.emit(
        "agent-notification",
        json!({
            "sessionId": session_id,
            "label": label,
            "message": message,
            "highRisk": false,
        }),
    );
    super::speak_notice(&acp.app, message);
}

/// Human name for a tool kind, for the spoken prompt.
fn kind_name(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "a file read",
        ToolKind::Edit => "a file edit",
        ToolKind::Delete => "a delete",
        ToolKind::Move => "a move",
        ToolKind::Search => "a search",
        ToolKind::Execute => "a command",
        ToolKind::Think => "thinking",
        ToolKind::Fetch => "a network fetch",
        ToolKind::SwitchMode => "a mode change",
        ToolKind::Other => "a tool",
    }
}

/// A JSON-RPC id as text, for storing in [`PendingApproval`].
///
/// Numbers and strings are both legal; strings arrive without quotes so the
/// value reads naturally in a log line.
fn id_text(id: &Value) -> String {
    match id {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn error_text(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error");
    match error.get("data").and_then(Value::as_str) {
        Some(data) if !data.trim().is_empty() => format!("{message} — {data}"),
        _ => message.to_string(),
    }
}

/// Wording for what we just answered, since "always" is worth repeating back.
fn option_id_label(option_id: &str, allow: bool) -> String {
    match (allow, option_id) {
        (true, "allow_always") => "Approved, and allowed from now on".to_string(),
        (true, _) => "Approved".to_string(),
        (false, "reject_always") => "Denied, and blocked from now on".to_string(),
        (false, _) => "Denied".to_string(),
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;

    /// The exact `session/new` result captured from `kiro-cli acp` 2.18.1 on
    /// 2026-08-16, trimmed to three models and two modes. Kept verbatim in shape
    /// so a vendor change to the payload breaks a test rather than a session.
    fn kiro_session_new() -> Value {
        json!({
            "sessionId": "fd6989da-d314-4bed-9c38-2e4479f18e46",
            "modes": {
                "currentModeId": "kiro_default",
                "availableModes": [
                    { "id": "kiro_default", "name": "kiro_default", "description": "The default agent for Kiro CLI" },
                    { "id": "kiro_planner", "name": "kiro_planner", "description": "Specialized planning agent",
                      "_meta": { "welcomeMessage": "Transform any idea into fully working code." } }
                ]
            },
            "models": {
                "currentModelId": "claude-opus-5",
                "availableModels": [
                    { "modelId": "auto", "name": "auto", "description": "Models chosen by task" },
                    { "modelId": "claude-opus-5", "name": "claude-opus-5", "description": "Claude Opus 5 with 1M context" },
                    { "modelId": "gpt-5.6-luna", "name": "gpt-5.6-luna", "description": "OpenAI GPT 5.6 Luna" }
                ]
            }
        })
    }

    #[test]
    fn the_live_session_payload_yields_switchable_models() {
        let (current, available) = parse_models(kiro_session_new().get("models"));
        assert_eq!(current.as_deref(), Some("claude-opus-5"));
        assert_eq!(available.len(), 3);
        assert_eq!(available[2].0, "gpt-5.6-luna");
    }

    #[test]
    fn spoken_model_names_reach_the_right_id() {
        let (_, available) = parse_models(kiro_session_new().get("models"));
        // Every one of these is how dictation renders a model name out loud.
        for spoken in ["gpt 5.6 luna", "GPT-5.6 Luna", "luna", "gpt5.6luna"] {
            assert_eq!(
                match_choice(&available, spoken).as_deref(),
                Some("gpt-5.6-luna"),
                "{spoken}"
            );
        }
        assert_eq!(
            match_choice(&available, "opus").as_deref(),
            Some("claude-opus-5")
        );
        // A model this session does not have must not be guessed at.
        assert_eq!(match_choice(&available, "gemini 3 pro"), None);
    }

    #[test]
    fn config_options_are_preferred_and_categorised() {
        let config = parse_config_options(Some(&json!([
            { "id": "mode", "name": "Session Mode", "category": "mode", "type": "select",
              "currentValue": "ask",
              "options": [ { "value": "ask", "name": "Ask" }, { "value": "code", "name": "Code" } ] },
            { "id": "model", "name": "Model", "category": "model", "type": "select",
              "currentValue": "model-1",
              "options": [ { "value": "model-1", "name": "Model 1" }, { "value": "model-2", "name": "Model 2" } ] },
            { "id": "brave", "name": "Brave Mode", "type": "boolean", "currentValue": true },
            // An unrecognised type is skipped, per the spec: the agent keeps
            // using its own default rather than us guessing at a control.
            { "id": "temperature", "name": "Temperature", "type": "slider", "currentValue": "0.7" }
        ])));
        assert_eq!(config.len(), 3);
        let controls = Controls {
            config,
            ..Controls::default()
        };
        assert_eq!(controls.by_category("model").unwrap().current, "model-1");
        assert_eq!(controls.model_choices().len(), 2);
        assert!(controls.config.iter().any(|o| o.is_boolean));
    }

    #[test]
    fn both_command_payload_shapes_are_read() {
        // The standard notification.
        let standard = parse_commands(Some(&json!({
            "availableCommands": [
                { "name": "web", "description": "Search the web", "input": { "hint": "query" } }
            ]
        })));
        assert_eq!(standard[0].name, "web");
        assert_eq!(standard[0].hint.as_deref(), Some("query"));

        // Kiro's vendor payload, which uses `commands`, leading slashes, and puts
        // the hint under `meta`.
        let kiro = parse_commands(Some(&json!({
            "commands": [
                { "name": "/compact", "description": "Compact conversation history" },
                { "name": "/context", "description": "Manage context files",
                  "meta": { "hint": "add <path>, remove <path>, clear" } }
            ]
        })));
        assert_eq!(kiro.len(), 2);
        assert_eq!(kiro[0].name, "compact", "the slash must be stripped");
        assert!(kiro[1].hint.is_some());
    }

    #[test]
    fn commands_that_destroy_work_are_refused() {
        // Not a style preference: each of these loses something the user cannot
        // get back by saying "undo".
        for (name, args) in [
            ("clear", None),
            ("/quit", None),
            ("rewind", None),
            ("tools", Some("trust-all")),
            ("chat", Some("load ./other.json")),
            ("context", Some("clear")),
        ] {
            assert_eq!(
                command_risk(name, args),
                CommandRisk::Refused,
                "{name} {args:?}"
            );
        }
        for (name, args) in [
            ("compact", None),
            ("usage", None),
            ("context", Some("add src/main.rs")),
            ("tools", None),
            ("effort", Some("high")),
        ] {
            assert_eq!(command_risk(name, args), CommandRisk::Safe, "{name}");
        }
    }

    #[test]
    fn usage_updates_become_a_percentage_only_when_it_is_knowable() {
        assert_eq!(
            usage_percent(&json!({ "size": 40_000, "maxSize": 200_000 })),
            Some(20.0)
        );
        assert_eq!(
            usage_percent(&json!({ "contextUsagePercentage": 82.4 })),
            Some(82.4)
        );
        // A token count with no window size cannot be turned into a percentage,
        // and inventing a window would put a wrong number into a spoken warning.
        assert_eq!(usage_percent(&json!({ "size": 40_000 })), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A digest and its backing pieces, standing in for a live session.
    struct Fixture {
        state: Arc<Mutex<SessionState>>,
        tracker: Mutex<ToolTracker>,
        line_buf: Mutex<String>,
        modes: Mutex<Modes>,
        controls: Mutex<Controls>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                state: Arc::new(Mutex::new(SessionState {
                    id: "1".into(),
                    agent_session_id: None,
                    label: "tmp".into(),
                    cwd: "C:/tmp".into(),
                    model: None,
                    status: AgentStatus::Starting,
                    started: std::time::Instant::now(),
                    last_tool: None,
                    last_line: None,
                    files_touched: Vec::new(),
                    tool_calls: 0,
                    cost_usd: 0.0,
                    pending: None,
                    tool_error: None,
                    error: None,
                    task: "make a file".into(),
                    agent: AgentKind::KiroCli,
                    usage: None,
                    auto_approvals: 0,
                })),
                tracker: Mutex::new(ToolTracker::default()),
                line_buf: Mutex::new(String::new()),
                modes: Mutex::new(Modes::default()),
                controls: Mutex::new(Controls::default()),
            }
        }

        fn digest(&self) -> Digest<'_> {
            Digest {
                state: &self.state,
                tracker: &self.tracker,
                line_buf: &self.line_buf,
                modes: &self.modes,
                controls: &self.controls,
            }
        }

        fn feed(&self, update: Value) -> bool {
            apply_update(&self.digest(), &update)
        }

        fn view(&self) -> super::super::AgentSessionView {
            self.state.lock().unwrap().view()
        }
    }

    #[test]
    fn the_real_kiro_turn_produces_a_sensible_digest() {
        // Replays the exact event sequence captured from kiro-cli acp 2.18.1 by
        // scripts/acp-probe.mjs, including the vendor chunk event that arrives
        // before the standard one. If Kiro changes shape, this test is where it
        // shows up.
        let f = Fixture::new();

        // 1. The chunk event: carries the kind, no title.
        assert!(f.feed(json!({
            "sessionUpdate": "tool_call_chunk",
            "toolCallId": "toolu_bdrk_014f",
            "kind": "edit"
        })));
        // 2. The standard event: carries the title.
        assert!(f.feed(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "toolu_bdrk_014f",
            "title": "Creating ping.txt",
            "kind": "edit",
            "status": "pending",
            "locations": [{ "path": "C:/tmp/ping.txt" }]
        })));
        // 3. Completion.
        assert!(f.feed(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "toolu_bdrk_014f",
            "status": "completed"
        })));
        // 4. The streamed reply, in the several chunks it really arrives in.
        for piece in [
            "Created `ping.txt`",
            " in the current directory containing `",
            "pong`;",
            " verified by reading it back.",
        ] {
            f.feed(json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": piece }
            }));
        }

        let view = f.view();
        assert_eq!(view.status, AgentStatus::Working);
        // Counted once, not once per event shape.
        assert_eq!(view.tool_calls, 1);
        assert_eq!(view.last_tool.as_deref(), Some("Creating ping.txt"));
        assert_eq!(view.files_touched, vec!["ping.txt".to_string()]);
        assert!(view.tool_error.is_none());
        // The chunks are reassembled into one readable line, not four.
        assert_eq!(
            view.last_line.as_deref(),
            Some("Created `ping.txt` in the current directory containing `pong`; verified by reading it back.")
        );
    }

    #[test]
    fn the_correlated_kind_reaches_the_policy() {
        // The end-to-end version of the correlation requirement: the kind only
        // ever appeared on the chunk event, and the policy still sees it.
        let f = Fixture::new();
        f.feed(json!({
            "sessionUpdate": "tool_call_chunk",
            "toolCallId": "call_a",
            "kind": "edit"
        }));
        f.feed(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call_a",
            "title": "Creating notes.md",
            "locations": [{ "path": "C:/tmp/notes.md" }]
        }));
        let tracker = f.tracker.lock().unwrap();
        let info = tracker.get("call_a").expect("tracked");
        assert_eq!(info.kind, Some(ToolKind::Edit));
        assert_eq!(info.title.as_deref(), Some("Creating notes.md"));
    }

    #[test]
    fn an_unfinished_write_is_never_reported_as_a_changed_file() {
        // The bug this pins down: a session reported four files created into a
        // folder that was empty. `locations` arrives on the first `tool_call`
        // event, while the call is still `pending` and may yet be refused or
        // fail — so a digest that harvests paths there is describing the agent's
        // intentions as if they were results.
        let f = Fixture::new();
        f.feed(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call_pending",
            "title": "Creating index.html",
            "kind": "edit",
            "status": "pending",
            "locations": [{ "path": "C:/tmp/index.html" }]
        }));
        assert!(
            f.view().files_touched.is_empty(),
            "a pending write is not a changed file"
        );
        // Still nothing while it runs.
        f.feed(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_pending",
            "status": "in_progress"
        }));
        assert!(f.view().files_touched.is_empty());
        // A refusal or crash ends it without a file.
        f.feed(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_pending",
            "status": "failed"
        }));
        assert!(
            f.view().files_touched.is_empty(),
            "a failed write must not be counted"
        );
        assert!(f.view().tool_error.is_some());

        // A different call that really does complete is counted — and the path
        // came from the earlier event, so completion alone is enough.
        f.feed(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call_done",
            "title": "Creating styles.css",
            "kind": "edit",
            "status": "pending",
            "locations": [{ "path": "C:/tmp/styles.css" }]
        }));
        f.feed(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_done",
            "status": "completed"
        }));
        assert_eq!(f.view().files_touched, vec!["styles.css".to_string()]);
    }

    #[test]
    fn the_agent_changing_its_own_model_is_noticed() {
        // Agents fall back to another model under rate limits. A status report
        // that keeps naming the old one is lying by omission.
        let f = Fixture::new();
        assert!(!f.feed(json!({
            "sessionUpdate": "config_option_update",
            "configOptions": [
                { "id": "model", "name": "Model", "category": "model", "type": "select",
                  "currentValue": "claude-haiku-4.5",
                  "options": [ { "value": "claude-haiku-4.5", "name": "Haiku" } ] },
                { "id": "effort", "name": "Thinking", "category": "thought_level", "type": "select",
                  "currentValue": "low", "options": [ { "value": "low", "name": "Low" } ] }
            ]
        })));
        let controls = f.controls.lock().unwrap();
        assert_eq!(controls.model.as_deref(), Some("claude-haiku-4.5"));
        assert_eq!(controls.effort.as_deref(), Some("low"));
    }

    #[test]
    fn advertised_commands_are_remembered_for_later_use() {
        let f = Fixture::new();
        f.feed(json!({
            "sessionUpdate": "available_commands_update",
            "availableCommands": [
                { "name": "compact", "description": "Compact conversation history" }
            ]
        }));
        assert!(f.controls.lock().unwrap().has_command("/compact"));
        assert!(!f.controls.lock().unwrap().has_command("teleport"));
    }

    #[test]
    fn a_failed_tool_records_a_readable_reason_then_clears() {
        let f = Fixture::new();
        f.feed(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call_b",
            "title": "Running tests",
            "kind": "execute"
        }));
        f.feed(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_b",
            "status": "failed",
            "content": [{ "type": "content", "content": { "type": "text", "text": "cargo not found" } }]
        }));
        assert_eq!(f.view().tool_error.as_deref(), Some("cargo not found"));

        // A later successful call clears it, so a recovered session stops
        // reporting an error it has moved past.
        f.feed(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call_c",
            "title": "Reading Cargo.toml",
            "kind": "read"
        }));
        assert!(f.view().tool_error.is_none());
    }

    #[test]
    fn files_are_deduplicated_and_bounded() {
        let f = Fixture::new();
        for i in 0..(FILES_BUDGET + 5) {
            f.feed(json!({
                "sessionUpdate": "tool_call",
                "toolCallId": format!("call_{i}"),
                "title": format!("Editing file{i}.rs"),
                "kind": "edit",
                "locations": [{ "path": format!("C:/tmp/file{i}.rs") }]
            }));
            // Completion is what makes it a changed file rather than an
            // intention, so every edit here finishes.
            f.feed(json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": format!("call_{i}"),
                "status": "completed"
            }));
        }
        // Repeating a path must not add a second entry.
        f.feed(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "call_repeat",
            "title": "Editing file1.rs again",
            "kind": "edit",
            "locations": [{ "path": "C:/tmp/file1.rs" }]
        }));
        f.feed(json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_repeat",
            "status": "completed"
        }));
        let view = f.view();
        assert_eq!(view.files_touched.len(), FILES_BUDGET);
        let unique: std::collections::HashSet<_> = view.files_touched.iter().collect();
        assert_eq!(unique.len(), view.files_touched.len());
    }

    #[test]
    fn kiro_metadata_becomes_a_spoken_phrase_not_a_dollar_amount() {
        // Real payload shape. Credits must never be written into cost_usd.
        let f = Fixture::new();
        assert!(apply_metadata(
            &f.digest(),
            Some(&json!({
                "contextUsagePercentage": 12.4,
                "effort": "medium",
                "meteringUsage": [{ "value": 0.49, "unit": "credit" }],
                "turnDurationMs": 10600
            }))
        ));
        let view = f.view();
        assert_eq!(
            view.usage.as_deref(),
            Some("0.49 credits, 12% context used")
        );
        assert_eq!(
            view.cost_usd, 0.0,
            "credits must not be reported as dollars"
        );
    }

    #[test]
    fn metadata_without_numbers_changes_nothing() {
        let f = Fixture::new();
        assert!(!apply_metadata(
            &f.digest(),
            Some(&json!({ "effort": "high" }))
        ));
        assert!(!apply_metadata(&f.digest(), None));
        assert!(f.view().usage.is_none());
    }

    #[test]
    fn mode_updates_are_tracked() {
        let f = Fixture::new();
        f.feed(json!({
            "sessionUpdate": "current_mode_update",
            "currentModeId": "kiro_planner"
        }));
        assert_eq!(
            f.modes.lock().unwrap().current.as_deref(),
            Some("kiro_planner")
        );
    }

    #[test]
    fn unknown_update_kinds_are_ignored_without_panicking() {
        // Forward compatibility: a new sessionUpdate variant must be inert.
        let f = Fixture::new();
        assert!(!f.feed(json!({ "sessionUpdate": "plan", "entries": [] })));
        assert!(!f.feed(json!({ "sessionUpdate": "something_new_in_2027" })));
        assert!(!f.feed(json!({})));
        assert_eq!(f.view().tool_calls, 0);
    }

    #[test]
    fn vendor_prefixed_methods_are_recognised() {
        // Verified from kiro-cli 2.18.1, which sends both spellings.
        assert!(is_method("session/update", "session/update"));
        assert!(is_method("_kiro.dev/session/update", "session/update"));
        assert!(is_method(
            "session/request_permission",
            "session/request_permission"
        ));
        assert!(!is_method("session/update", "session/request_permission"));
        assert!(!is_method("_kiro.dev/metadata", "session/update"));
    }

    #[test]
    fn ids_survive_being_strings_or_numbers() {
        assert_eq!(id_text(&json!("fd7bf01b-4c8c")), "fd7bf01b-4c8c");
        assert_eq!(id_text(&json!(7)), "7");
    }

    #[test]
    fn paths_are_shortened_against_the_project() {
        let cwd = if cfg!(windows) {
            r"C:\work\app"
        } else {
            "/work/app"
        };
        let nested = if cfg!(windows) {
            r"C:\work\app\src\main.rs"
        } else {
            "/work/app/src/main.rs"
        };
        assert_eq!(
            display_path(nested, cwd),
            if cfg!(windows) {
                r"src\main.rs"
            } else {
                "src/main.rs"
            }
        );
        // Outside the project, the bare name is still better than a long path.
        let outside = if cfg!(windows) {
            r"C:\other\notes.md"
        } else {
            "/other/notes.md"
        };
        assert_eq!(display_path(outside, cwd), "notes.md");
    }

    #[test]
    fn errors_read_as_sentences() {
        assert_eq!(
            error_text(&json!({ "code": -32000, "message": "boom", "data": "no such folder" })),
            "boom — no such folder"
        );
        assert_eq!(error_text(&json!({ "message": "boom" })), "boom");
        assert_eq!(error_text(&json!({})), "unknown error");
    }

    #[test]
    fn always_answers_are_worded_differently() {
        assert_eq!(
            option_id_label("allow_always", true),
            "Approved, and allowed from now on"
        );
        assert_eq!(option_id_label("allow_once", true), "Approved");
        assert_eq!(option_id_label("reject_once", false), "Denied");
    }

    #[test]
    fn tool_failure_text_reads_the_nested_shape() {
        let update = json!({
            "status": "failed",
            "content": [{ "type": "content", "content": { "type": "text", "text": "permission denied" } }]
        });
        assert_eq!(
            tool_failure_text(&update).as_deref(),
            Some("permission denied")
        );
        assert!(tool_failure_text(&json!({ "status": "failed" })).is_none());
    }
}
