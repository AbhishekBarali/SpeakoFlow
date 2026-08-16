//! Which agent actions may be approved without asking a human.
//!
//! The point of the feature is that the user is not watching. A user who has to
//! say "yes" to every file read has not been freed from the terminal, they have
//! been given a worse terminal. So safe actions are answered automatically and
//! the rest still stop and wait.
//!
//! The hazard is obvious, so the design is deliberately paranoid:
//!
//! * **The permission request does not say what the tool does.** Verified
//!   against `kiro-cli acp` 2.18.1: `session/request_permission` carries only
//!   `toolCallId` and `title`. The `kind` (`edit`, `execute`, `delete`, …)
//!   arrived earlier, on a `session/update` `tool_call` event with the same id.
//!   A policy that reads only the permission frame is approving a title, not an
//!   action. So every tool call is recorded in a [`ToolTracker`] as it streams
//!   past, and the permission request is correlated back to it. The same
//!   omission is documented for `codex-acp`, so it is the norm, not a quirk.
//! * **Fail closed.** No correlation, no `kind`, an unrecognised `kind`, or a
//!   path we cannot resolve all mean "ask the human". Every unknown is a deny.
//! * **Destructive kinds are never auto-approved, and never voice-approvable**,
//!   regardless of settings. `delete` and `move` always reach a human, and
//!   [`super::is_high_risk`] additionally blocks a spoken "yes" for dangerous
//!   shell commands.
//! * **Writes are scoped to the project.** An `edit` inside the session's own
//!   working directory is the expected case and can be automatic; an edit
//!   anywhere else is not, and asks. Containment is checked on *canonicalised*
//!   paths, never on string prefixes, because `C:\work\..\Windows` is a prefix
//!   match and a directory escape. The reference filesystem MCP server shipped
//!   exactly that bug (CVE-2025-53109).

use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};

/// How many tool calls to remember per session.
///
/// Only needed long enough to answer a permission request that follows its own
/// `tool_call` event, which is immediate. The cap exists so a long session
/// cannot grow the map without bound.
const TRACKER_BUDGET: usize = 256;

/// The kind of thing a tool call does, as classified by the agent itself.
///
/// These are the `ToolCallKind` values in the ACP schema. `Other` covers both
/// the spec's own `other` and anything a future agent invents, and is treated as
/// unknown-and-therefore-unsafe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    Other,
}

impl ToolKind {
    /// Parse an ACP `kind` string. Unrecognised values become [`ToolKind::Other`]
    /// rather than an error, so a new kind degrades to "ask" instead of breaking
    /// the session.
    pub fn from_acp(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "read" => ToolKind::Read,
            "edit" => ToolKind::Edit,
            "delete" => ToolKind::Delete,
            "move" => ToolKind::Move,
            "search" => ToolKind::Search,
            "execute" => ToolKind::Execute,
            "think" => ToolKind::Think,
            "fetch" => ToolKind::Fetch,
            "switch_mode" | "switchmode" => ToolKind::SwitchMode,
            _ => ToolKind::Other,
        }
    }

    /// Whether this kind can destroy work that is not recoverable from git.
    fn is_destructive(self) -> bool {
        matches!(self, ToolKind::Delete | ToolKind::Move)
    }
}

/// What we know about one in-flight tool call, accumulated from every event that
/// mentioned it.
#[derive(Debug, Clone, Default)]
pub struct ToolCallInfo {
    pub title: Option<String>,
    pub kind: Option<ToolKind>,
    /// The tool's own arguments, when the agent sends them. Used to spot
    /// dangerous shell commands and to find the paths a write would touch.
    pub raw_input: Option<Value>,
    /// Paths reported via `locations`, plus anything recognisable in `raw_input`.
    pub paths: Vec<String>,
}

impl ToolCallInfo {
    /// The shell command this call would run, if it is that sort of call.
    pub fn command(&self) -> Option<&str> {
        self.raw_input
            .as_ref()
            .and_then(|v| v.get("command"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|c| !c.is_empty())
    }
}

/// Remembers tool calls by id so a later permission request can be understood.
///
/// Bounded and insertion-ordered: the oldest entry is dropped once the budget is
/// reached.
#[derive(Debug, Default)]
pub struct ToolTracker {
    calls: HashMap<String, ToolCallInfo>,
    order: VecDeque<String>,
}

impl ToolTracker {
    /// Fold one `tool_call`, `tool_call_chunk`, or `tool_call_update` event into
    /// what we know.
    ///
    /// Merging rather than replacing is required: the chunk event that arrives
    /// first often carries the `kind` while the later full event carries the
    /// human title, and `tool_call_update` carries neither. Whichever order they
    /// arrive in, nothing already learned is forgotten.
    pub fn observe(&mut self, id: &str, update: &Value) {
        if id.is_empty() {
            return;
        }
        if !self.calls.contains_key(id) {
            if self.order.len() >= TRACKER_BUDGET {
                if let Some(oldest) = self.order.pop_front() {
                    self.calls.remove(&oldest);
                }
            }
            self.order.push_back(id.to_string());
        }
        let info = self.calls.entry(id.to_string()).or_default();

        if let Some(title) = update.get("title").and_then(Value::as_str) {
            if !title.trim().is_empty() {
                info.title = Some(title.trim().to_string());
            }
        }
        if let Some(kind) = update.get("kind").and_then(Value::as_str) {
            info.kind = Some(ToolKind::from_acp(kind));
        }
        if let Some(raw) = update.get("rawInput") {
            if !raw.is_null() {
                info.raw_input = Some(raw.clone());
            }
        }
        for path in paths_in(update) {
            if !info.paths.contains(&path) {
                info.paths.push(path);
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&ToolCallInfo> {
        self.calls.get(id)
    }
}

/// Every path mentioned by a tool-call event: the ACP `locations` array, plus the
/// usual argument names agents use for a file.
fn paths_in(update: &Value) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(locations) = update.get("locations").and_then(Value::as_array) {
        for location in locations {
            if let Some(path) = location.get("path").and_then(Value::as_str) {
                if !path.trim().is_empty() {
                    found.push(path.to_string());
                }
            }
        }
    }
    if let Some(raw) = update.get("rawInput") {
        for key in [
            "path",
            "file_path",
            "filePath",
            "notebook_path",
            "target_file",
            "abs_path",
        ] {
            if let Some(path) = raw.get(key).and_then(Value::as_str) {
                if !path.trim().is_empty() && !found.iter().any(|p| p == path) {
                    found.push(path.to_string());
                }
            }
        }
    }
    found
}

/// The user's auto-approval settings for one session.
///
/// Everything is off by default. Auto-approval is a thing the user turns on
/// knowingly, per session, not a default that surprises them.
#[derive(Debug, Clone)]
pub struct ApprovalPolicy {
    /// Master switch. With this off nothing is ever auto-answered.
    pub enabled: bool,
    /// Auto-allow non-mutating work: reads, searches, thinking, fetches.
    pub allow_reads: bool,
    /// Auto-allow file writes, but only inside `project_root`.
    pub allow_edits: bool,
    /// The session's working directory. Writes outside it always ask.
    pub project_root: Option<PathBuf>,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_reads: true,
            allow_edits: true,
            project_root: None,
        }
    }
}

/// The outcome of a policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Answer "yes" on the user's behalf. The reason is logged and shown, so an
    /// auto-approval is never invisible.
    AutoAllow(&'static str),
    /// Park the session and wait for a human.
    Ask {
        /// True when even an explicit spoken "yes" must not be enough.
        high_risk: bool,
    },
}

impl ApprovalPolicy {
    /// Decide what to do about a permission request.
    ///
    /// `info` is the correlated tool call, or `None` when correlation failed —
    /// which is itself a reason to ask.
    pub fn decide(&self, info: Option<&ToolCallInfo>) -> Verdict {
        // No correlation means we do not know what we would be approving.
        let Some(info) = info else {
            return Verdict::Ask { high_risk: false };
        };
        // An agent that declines to say what kind of call this is gets no trust.
        let Some(kind) = info.kind else {
            return Verdict::Ask { high_risk: false };
        };

        if kind.is_destructive() {
            // Never auto-approved, and never voice-approvable, whatever the
            // settings say.
            return Verdict::Ask { high_risk: true };
        }

        if kind == ToolKind::Execute {
            // Running commands is always a human decision. Dangerous ones are
            // additionally blocked from voice approval.
            let command = info.command().unwrap_or_default();
            let dangerous = !command.is_empty()
                && super::is_high_risk("Bash", &serde_json::json!({ "command": command }));
            return Verdict::Ask {
                high_risk: dangerous,
            };
        }

        if !self.enabled {
            return Verdict::Ask { high_risk: false };
        }

        match kind {
            ToolKind::Read | ToolKind::Search | ToolKind::Think | ToolKind::Fetch => {
                if self.allow_reads {
                    Verdict::AutoAllow("read-only")
                } else {
                    Verdict::Ask { high_risk: false }
                }
            }
            ToolKind::Edit => {
                if !self.allow_edits {
                    return Verdict::Ask { high_risk: false };
                }
                let Some(root) = self.project_root.as_deref() else {
                    // Without a known project there is no "inside" to be inside.
                    return Verdict::Ask { high_risk: false };
                };
                if info.paths.is_empty() {
                    // An edit that will not say what it edits.
                    return Verdict::Ask { high_risk: false };
                }
                if info.paths.iter().all(|p| within(root, Path::new(p))) {
                    Verdict::AutoAllow("edit inside the project folder")
                } else {
                    Verdict::Ask { high_risk: false }
                }
            }
            // `switch_mode` changes the agent's own permissions, so it is a
            // human decision. `Other` is unknown by definition.
            ToolKind::SwitchMode | ToolKind::Other => Verdict::Ask { high_risk: false },
            // Handled above.
            ToolKind::Delete | ToolKind::Move | ToolKind::Execute => {
                Verdict::Ask { high_risk: true }
            }
        }
    }
}

/// Whether `candidate` resolves to something inside `root`.
///
/// Compares canonicalised paths, so `..` traversal and symlinks are resolved
/// before the comparison rather than after. A file that does not exist yet — the
/// common case for "create this file" — is checked via its nearest existing
/// ancestor, because a path cannot be canonicalised until it exists.
///
/// Returns `false` on anything it cannot resolve. This is a security check, so
/// "I don't know" and "no" are the same answer.
pub fn within(root: &Path, candidate: &Path) -> bool {
    let Some(root) = resolve(root) else {
        return false;
    };
    // A relative path is relative to the session's own cwd, which is the root.
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    // Reject traversal outright rather than relying on it cancelling out. A
    // symlinked ancestor plus `..` can escape a purely textual normalisation.
    if candidate
        .components()
        .any(|c| matches!(c, Component::ParentDir))
        && resolve(&candidate).is_none()
    {
        return false;
    }
    let Some(resolved) = resolve_or_ancestor(&candidate) else {
        return false;
    };
    resolved.starts_with(&root)
}

/// Canonicalise, dropping Windows' `\\?\` verbatim prefix so comparisons between
/// a canonicalised path and a canonicalised root behave the same on every OS.
fn resolve(path: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(path).ok()?;
    let text = canonical.to_string_lossy();
    Some(match text.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => canonical,
    })
}

/// Canonicalise `path`, or its nearest existing ancestor with the remainder
/// re-appended, so a not-yet-created file can still be checked.
fn resolve_or_ancestor(path: &Path) -> Option<PathBuf> {
    if let Some(resolved) = resolve(path) {
        return Some(resolved);
    }
    let mut tail = Vec::new();
    let mut cursor = path;
    while let Some(parent) = cursor.parent() {
        let name = cursor.file_name()?;
        tail.push(name.to_os_string());
        if let Some(resolved) = resolve(parent) {
            let mut out = resolved;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return Some(out);
        }
        cursor = parent;
    }
    None
}

/// Pick the option to answer a permission request with.
///
/// Verified against `kiro-cli acp` 2.18.1, which offers `allow_once`,
/// `allow_always`, and `reject_once` — but **not** `reject_always`, which the
/// schema does define. Cursor omits it too. So the wanted kind is a preference,
/// not an assumption: we take the best available and never invent an id.
///
/// `options` is the array from the request. Returns the `optionId` to send back
/// verbatim; the ids are the agent's own strings and must not be normalised.
pub fn pick_option(options: &[Value], allow: bool) -> Option<String> {
    let preferred: &[&str] = if allow {
        &["allow_once", "allow_always"]
    } else {
        &["reject_once", "reject_always"]
    };
    for wanted in preferred {
        for option in options {
            if option.get("kind").and_then(Value::as_str) == Some(*wanted) {
                if let Some(id) = option.get("optionId").and_then(Value::as_str) {
                    return Some(id.to_string());
                }
            }
        }
    }
    // No recognised kind. Rather than guess at a custom option — which could be
    // anything, including "always allow everything" — give up and let the caller
    // treat it as unanswerable.
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tracked(update: Value) -> ToolTracker {
        let mut tracker = ToolTracker::default();
        tracker.observe("call_1", &update);
        tracker
    }

    fn policy_on(root: &Path) -> ApprovalPolicy {
        ApprovalPolicy {
            enabled: true,
            allow_reads: true,
            allow_edits: true,
            project_root: Some(root.to_path_buf()),
        }
    }

    #[test]
    fn kind_learned_from_a_chunk_survives_the_full_event() {
        // The exact ordering observed from kiro-cli: the chunk carries the kind,
        // the later event carries the title, and neither repeats the other.
        let mut tracker = ToolTracker::default();
        tracker.observe("call_1", &json!({ "kind": "edit" }));
        tracker.observe("call_1", &json!({ "title": "Creating ping.txt" }));
        let info = tracker.get("call_1").expect("call is tracked");
        assert_eq!(info.kind, Some(ToolKind::Edit));
        assert_eq!(info.title.as_deref(), Some("Creating ping.txt"));
    }

    #[test]
    fn unknown_kinds_do_not_become_trusted() {
        assert_eq!(ToolKind::from_acp("teleport"), ToolKind::Other);
        let tracker = tracked(json!({ "kind": "teleport", "title": "Teleporting" }));
        let verdict = ApprovalPolicy {
            enabled: true,
            ..ApprovalPolicy::default()
        }
        .decide(tracker.get("call_1"));
        assert_eq!(verdict, Verdict::Ask { high_risk: false });
    }

    #[test]
    fn a_permission_request_we_cannot_correlate_asks() {
        // The failure this protects against: approving a title because the
        // tool call it belongs to was never seen.
        let policy = ApprovalPolicy {
            enabled: true,
            ..ApprovalPolicy::default()
        };
        assert_eq!(policy.decide(None), Verdict::Ask { high_risk: false });
    }

    #[test]
    fn a_call_with_no_kind_asks() {
        let tracker = tracked(json!({ "title": "Doing something" }));
        let policy = ApprovalPolicy {
            enabled: true,
            ..ApprovalPolicy::default()
        };
        assert_eq!(
            policy.decide(tracker.get("call_1")),
            Verdict::Ask { high_risk: false }
        );
    }

    #[test]
    fn reads_are_auto_allowed_only_when_enabled() {
        let tracker = tracked(json!({ "kind": "read", "title": "Reading main.rs" }));
        let mut policy = ApprovalPolicy {
            enabled: true,
            ..ApprovalPolicy::default()
        };
        assert_eq!(
            policy.decide(tracker.get("call_1")),
            Verdict::AutoAllow("read-only")
        );
        policy.enabled = false;
        assert_eq!(
            policy.decide(tracker.get("call_1")),
            Verdict::Ask { high_risk: false }
        );
    }

    #[test]
    fn deletes_always_ask_and_are_never_voice_approvable() {
        let tracker = tracked(json!({ "kind": "delete", "title": "Removing build/" }));
        let policy = ApprovalPolicy {
            enabled: true,
            allow_reads: true,
            allow_edits: true,
            project_root: Some(PathBuf::from(".")),
        };
        assert_eq!(
            policy.decide(tracker.get("call_1")),
            Verdict::Ask { high_risk: true }
        );
    }

    #[test]
    fn moves_always_ask() {
        let tracker = tracked(json!({ "kind": "move", "title": "Renaming src" }));
        assert_eq!(
            ApprovalPolicy {
                enabled: true,
                ..ApprovalPolicy::default()
            }
            .decide(tracker.get("call_1")),
            Verdict::Ask { high_risk: true }
        );
    }

    #[test]
    fn commands_always_ask_and_dangerous_ones_block_voice() {
        let safe = tracked(json!({
            "kind": "execute",
            "title": "Running tests",
            "rawInput": { "command": "cargo test" }
        }));
        let policy = ApprovalPolicy {
            enabled: true,
            ..ApprovalPolicy::default()
        };
        assert_eq!(
            policy.decide(safe.get("call_1")),
            Verdict::Ask { high_risk: false }
        );

        let nasty = tracked(json!({
            "kind": "execute",
            "title": "Cleaning up",
            "rawInput": { "command": "rm -rf /" }
        }));
        assert_eq!(
            policy.decide(nasty.get("call_1")),
            Verdict::Ask { high_risk: true }
        );
    }

    #[test]
    fn edits_inside_the_project_are_allowed_and_outside_are_not() {
        let root = tempfile::tempdir().expect("temp dir");
        let inside = root.path().join("src").join("new.rs");
        std::fs::create_dir_all(inside.parent().unwrap()).expect("create src");

        let tracker = tracked(json!({
            "kind": "edit",
            "title": "Creating new.rs",
            "locations": [{ "path": inside.to_string_lossy() }]
        }));
        assert_eq!(
            policy_on(root.path()).decide(tracker.get("call_1")),
            Verdict::AutoAllow("edit inside the project folder")
        );

        let elsewhere = tempfile::tempdir().expect("second temp dir");
        let outside = tracked(json!({
            "kind": "edit",
            "title": "Creating stray.rs",
            "locations": [{ "path": elsewhere.path().join("stray.rs").to_string_lossy() }]
        }));
        assert_eq!(
            policy_on(root.path()).decide(outside.get("call_1")),
            Verdict::Ask { high_risk: false }
        );
    }

    #[test]
    fn an_edit_that_hides_its_target_asks() {
        let root = tempfile::tempdir().expect("temp dir");
        let tracker = tracked(json!({ "kind": "edit", "title": "Editing something" }));
        assert_eq!(
            policy_on(root.path()).decide(tracker.get("call_1")),
            Verdict::Ask { high_risk: false }
        );
    }

    #[test]
    fn traversal_out_of_the_project_is_not_inside_it() {
        let root = tempfile::tempdir().expect("temp dir");
        // Textually a prefix match, semantically an escape.
        let escape = root.path().join("..").join("escaped.txt");
        let tracker = tracked(json!({
            "kind": "edit",
            "title": "Creating escaped.txt",
            "locations": [{ "path": escape.to_string_lossy() }]
        }));
        assert_eq!(
            policy_on(root.path()).decide(tracker.get("call_1")),
            Verdict::Ask { high_risk: false }
        );
    }

    #[test]
    fn a_sibling_folder_sharing_a_name_prefix_is_not_inside() {
        // `C:\work\app` must not contain `C:\work\app-secrets`.
        let base = tempfile::tempdir().expect("temp dir");
        let project = base.path().join("app");
        let sibling = base.path().join("app-secrets");
        std::fs::create_dir_all(&project).expect("project");
        std::fs::create_dir_all(&sibling).expect("sibling");
        assert!(!within(&project, &sibling.join("keys.env")));
        assert!(within(&project, &project.join("main.rs")));
    }

    #[test]
    fn relative_paths_are_resolved_against_the_project() {
        let root = tempfile::tempdir().expect("temp dir");
        assert!(within(root.path(), Path::new("notes.md")));
        assert!(!within(root.path(), Path::new("../notes.md")));
    }

    #[test]
    fn new_files_in_existing_folders_resolve() {
        let root = tempfile::tempdir().expect("temp dir");
        let nested = root.path().join("a").join("b");
        std::fs::create_dir_all(&nested).expect("nested");
        assert!(within(root.path(), &nested.join("not-created-yet.txt")));
    }

    #[test]
    fn option_ids_are_taken_verbatim_from_the_agent() {
        // The real payload from kiro-cli 2.18.1, which has no reject_always.
        let options = vec![
            json!({ "optionId": "allow_once", "name": "Yes", "kind": "allow_once" }),
            json!({ "optionId": "allow_always", "name": "Always", "kind": "allow_always" }),
            json!({ "optionId": "reject_once", "name": "No", "kind": "reject_once" }),
        ];
        assert_eq!(pick_option(&options, true).as_deref(), Some("allow_once"));
        assert_eq!(pick_option(&options, false).as_deref(), Some("reject_once"));
    }

    #[test]
    fn denial_falls_back_to_reject_always_when_that_is_all_there_is() {
        let options = vec![json!({ "optionId": "no-forever", "kind": "reject_always" })];
        assert_eq!(pick_option(&options, false).as_deref(), Some("no-forever"));
        assert_eq!(pick_option(&options, true), None);
    }

    #[test]
    fn unrecognised_options_are_not_guessed_at() {
        let options = vec![json!({ "optionId": "_vendor_yolo", "kind": "_vendor_custom" })];
        assert_eq!(pick_option(&options, true), None);
        assert_eq!(pick_option(&options, false), None);
    }

    #[test]
    fn the_tracker_stays_bounded() {
        let mut tracker = ToolTracker::default();
        for i in 0..(TRACKER_BUDGET + 10) {
            tracker.observe(&format!("call_{i}"), &json!({ "kind": "read" }));
        }
        assert_eq!(tracker.calls.len(), TRACKER_BUDGET);
        assert!(tracker.get("call_0").is_none());
        assert!(tracker
            .get(&format!("call_{}", TRACKER_BUDGET + 9))
            .is_some());
    }
}
