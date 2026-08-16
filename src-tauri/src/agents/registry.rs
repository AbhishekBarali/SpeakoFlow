//! Which coding agents SpeakoFlow can drive, and how to launch each one.
//!
//! Adding an agent should be a table entry, not a feature. That is the whole
//! reason this speaks the Agent Client Protocol: the vendors write and maintain
//! their own ACP adapters, so a new agent is a name, a command, and nothing else.
//!
//! Two transports exist, deliberately:
//!
//! * [`Transport::Acp`] — the general case. Everything new goes here.
//! * [`Transport::ClaudeStreamJson`] — the pre-existing native Claude Code
//!   driver. Kept because it needs no Node install and no adapter package, so it
//!   stays the zero-setup path for the most common agent.

use std::path::PathBuf;

use super::env;

/// How SpeakoFlow talks to an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// JSON-RPC 2.0 over stdio, per the Agent Client Protocol.
    Acp,
    /// Claude Code's own bidirectional `stream-json` control protocol.
    ClaudeStreamJson,
}

/// What to run in a terminal to give a session back to the user.
pub struct Handoff {
    /// The command line, with paths already quoted.
    pub command: String,
    /// Whether the agent resumes the conversation, or starts fresh in the folder.
    pub resumes_history: bool,
    /// Whether a first question can be appended to the command line.
    ///
    /// True only where it is verified: `kiro-cli chat [INPUT]` takes "the first
    /// question to ask" as a positional argument, which is what lets a fresh
    /// terminal open already reading the takeover brief instead of blank.
    pub accepts_first_prompt: bool,
}

/// A coding agent we know how to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    /// `kiro-cli acp`. Native ACP, verified working — see
    /// `docs/voice-agent-control.md`.
    KiroCli,
    /// Claude Code over its own protocol. No extra install.
    ClaudeCode,
    /// Claude Code over ACP, via the vendor adapter package.
    ClaudeAcp,
    /// OpenAI Codex over ACP.
    Codex,
    /// Gemini CLI's built-in ACP mode.
    Gemini,
    /// GitHub Copilot CLI over ACP.
    Copilot,
}

/// Every agent, in the order they should be offered.
pub const ALL: [AgentKind; 6] = [
    AgentKind::KiroCli,
    AgentKind::ClaudeCode,
    AgentKind::ClaudeAcp,
    AgentKind::Codex,
    AgentKind::Gemini,
    AgentKind::Copilot,
];

impl AgentKind {
    /// Stable identifier used in tool arguments and settings.
    pub fn id(self) -> &'static str {
        match self {
            AgentKind::KiroCli => "kiro",
            AgentKind::ClaudeCode => "claude",
            AgentKind::ClaudeAcp => "claude-acp",
            AgentKind::Codex => "codex",
            AgentKind::Gemini => "gemini",
            AgentKind::Copilot => "copilot",
        }
    }

    /// Name to say out loud.
    pub fn label(self) -> &'static str {
        match self {
            AgentKind::KiroCli => "Kiro",
            AgentKind::ClaudeCode => "Claude Code",
            AgentKind::ClaudeAcp => "Claude Code (ACP)",
            AgentKind::Codex => "Codex",
            AgentKind::Gemini => "Gemini",
            AgentKind::Copilot => "Copilot",
        }
    }

    pub fn transport(self) -> Transport {
        match self {
            AgentKind::ClaudeCode => Transport::ClaudeStreamJson,
            _ => Transport::Acp,
        }
    }

    /// The executable to look for.
    fn program(self) -> &'static str {
        match self {
            AgentKind::KiroCli => "kiro-cli",
            AgentKind::ClaudeCode => "claude",
            AgentKind::ClaudeAcp => "claude-agent-acp",
            AgentKind::Codex => "codex-acp",
            AgentKind::Gemini => "gemini",
            AgentKind::Copilot => "copilot",
        }
    }

    /// Arguments that put the agent into ACP mode.
    fn acp_args(self) -> &'static [&'static str] {
        match self {
            AgentKind::KiroCli => &["acp"],
            // The adapter packages are ACP servers already.
            AgentKind::ClaudeAcp | AgentKind::Codex => &[],
            AgentKind::Gemini => &["--experimental-acp"],
            AgentKind::Copilot => &["acp"],
            AgentKind::ClaudeCode => &[],
        }
    }

    /// What to tell the user when the agent is not installed.
    pub fn install_hint(self) -> &'static str {
        match self {
            AgentKind::KiroCli => "Install the Kiro CLI and make sure `kiro-cli` is on your PATH.",
            AgentKind::ClaudeCode => {
                "Install the Claude Code CLI, or set SPEAKOFLOW_CLAUDE_PATH to claude.exe."
            }
            AgentKind::ClaudeAcp => {
                "Install the adapter with `npm i -g @agentclientprotocol/claude-agent-acp`."
            }
            AgentKind::Codex => {
                "Install the adapter with `npm i -g @agentclientprotocol/codex-acp`."
            }
            AgentKind::Gemini => "Install the Gemini CLI and make sure `gemini` is on your PATH.",
            AgentKind::Copilot => {
                "Install the GitHub Copilot CLI and make sure `copilot` is on your PATH."
            }
        }
    }

    /// Absolute path to the agent's executable, or an explanation.
    pub fn resolve(self) -> Result<PathBuf, String> {
        // The existing Claude resolver knows about installer-specific locations
        // that a plain PATH search misses, so it stays authoritative for Claude.
        if matches!(self, AgentKind::ClaudeCode) {
            return env::resolve_claude();
        }
        // A per-agent override, for unusual installs.
        let override_var = format!(
            "SPEAKOFLOW_{}_PATH",
            self.id().to_uppercase().replace('-', "_")
        );
        if let Some(explicit) = env::resolve_var(&override_var) {
            let path = PathBuf::from(explicit.trim());
            if path.is_file() {
                return Ok(path);
            }
            return Err(format!(
                "{} points at {}, which is not a file.",
                override_var,
                path.display()
            ));
        }
        env::resolve_program(self.program())
            .ok_or_else(|| format!("Couldn't find {}. {}", self.label(), self.install_hint()))
    }

    /// The command line to start this agent, ready to spawn.
    pub fn command(self) -> Result<(PathBuf, Vec<String>), String> {
        let binary = self.resolve()?;
        let args = self
            .acp_args()
            .iter()
            .map(|a| (*a).to_string())
            .collect::<Vec<_>>();
        Ok((binary, args))
    }

    pub fn is_installed(self) -> bool {
        self.resolve().is_ok()
    }

    /// How to hand a session to a human in a terminal.
    ///
    /// Only Claude Code can genuinely resume: it takes `--resume <id>` and picks
    /// the conversation up mid-thread. Kiro cannot, and this is verified rather
    /// than assumed — an ACP session id does not appear in
    /// `kiro-cli chat --list-sessions`, so `--resume-id` has nothing to find.
    /// ACP sessions and chat sessions are separate stores.
    ///
    /// So for everything else the handoff opens the agent's own interactive CLI
    /// in the right folder, and [`Handoff::resumes_history`] says plainly that
    /// the thread does not carry over. Claiming otherwise produced the worst
    /// possible outcome in testing: a terminal reporting
    /// `No conversation found with session ID`.
    pub fn handoff(self, session_id: &str) -> Result<Handoff, String> {
        match self {
            AgentKind::ClaudeCode | AgentKind::ClaudeAcp => {
                let binary = env::resolve_claude()?;
                Ok(Handoff {
                    command: format!("\"{}\" --resume {}", binary.display(), session_id),
                    resumes_history: true,
                    // The thread is already there; a first question would fire an
                    // unwanted turn the moment the window opened.
                    accepts_first_prompt: false,
                })
            }
            AgentKind::KiroCli => {
                let binary = env::resolve_program("kiro-cli")
                    .ok_or_else(|| "Couldn't find the Kiro CLI to hand over to.".to_string())?;
                Ok(Handoff {
                    command: format!("\"{}\" chat", binary.display()),
                    resumes_history: false,
                    accepts_first_prompt: true,
                })
            }
            // The registry entry for these points at the ACP adapter, which is
            // not what a human wants to drive; the interactive CLI is separate.
            AgentKind::Codex | AgentKind::Gemini | AgentKind::Copilot => {
                let program = match self {
                    AgentKind::Codex => "codex",
                    AgentKind::Gemini => "gemini",
                    _ => "copilot",
                };
                let binary = env::resolve_program(program)
                    .ok_or_else(|| format!("Couldn't find {} to hand over to.", self.label()))?;
                Ok(Handoff {
                    command: format!("\"{}\"", binary.display()),
                    resumes_history: false,
                    // Untested against these CLIs. Guessing at argument shapes is
                    // how a handoff opens a terminal showing a usage error.
                    accepts_first_prompt: false,
                })
            }
        }
    }

    /// Match a spoken or typed agent name.
    ///
    /// Speech-to-text mangles these names badly and consistently — "Kiro"
    /// becomes "keto", "hero", or "kira"; "Claude" becomes "clot", "cloud", or
    /// "clod". Those are not edge cases, they are the normal input to this
    /// function, so the mishearings are listed explicitly rather than left to a
    /// fuzzy score that could just as easily pick the wrong agent.
    pub fn from_spoken(raw: &str) -> Option<Self> {
        let text = raw.trim().to_lowercase();
        if text.is_empty() {
            return None;
        }
        // Longest, most specific patterns first, so "claude code acp" does not
        // match plain "claude".
        const PATTERNS: &[(&str, AgentKind)] = &[
            ("claude-acp", AgentKind::ClaudeAcp),
            ("claude acp", AgentKind::ClaudeAcp),
            ("kiro", AgentKind::KiroCli),
            ("kero", AgentKind::KiroCli),
            ("keto", AgentKind::KiroCli),
            ("kira", AgentKind::KiroCli),
            ("hero code", AgentKind::KiroCli),
            ("kyro", AgentKind::KiroCli),
            ("claude", AgentKind::ClaudeCode),
            ("clod", AgentKind::ClaudeCode),
            ("clot", AgentKind::ClaudeCode),
            ("cloud code", AgentKind::ClaudeCode),
            ("codex", AgentKind::Codex),
            ("code x", AgentKind::Codex),
            ("kodex", AgentKind::Codex),
            ("gemini", AgentKind::Gemini),
            ("gemeni", AgentKind::Gemini),
            ("copilot", AgentKind::Copilot),
            ("co-pilot", AgentKind::Copilot),
            ("pilot", AgentKind::Copilot),
        ];
        for (pattern, kind) in PATTERNS {
            if text.contains(pattern) {
                return Some(*kind);
            }
        }
        None
    }
}

/// Every agent that is actually installed on this machine.
///
/// Used for the machine-context block, so the assistant offers agents the user
/// has rather than agents that exist.
pub fn installed() -> Vec<AgentKind> {
    ALL.iter().copied().filter(|a| a.is_installed()).collect()
}

/// The agent to use when the user did not name one.
///
/// Preference order is deliberate: an explicit setting, then the ACP-native
/// agents, then the legacy Claude transport. ACP is the path that gets better as
/// the ecosystem improves, so it is the default whenever it is available.
pub fn default_agent() -> Option<AgentKind> {
    if let Some(configured) = env::resolve_var("SPEAKOFLOW_AGENT") {
        if let Some(kind) = AgentKind::from_spoken(&configured) {
            if kind.is_installed() {
                return Some(kind);
            }
        }
    }
    installed().first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mishearings_resolve_to_the_right_agent() {
        // Every one of these was produced by real dictation of an agent name.
        for spoken in ["kiro", "Keto code", "hero code", "use kira please"] {
            assert_eq!(
                AgentKind::from_spoken(spoken),
                Some(AgentKind::KiroCli),
                "{spoken}"
            );
        }
        for spoken in ["claude", "clot code", "Cloud Code", "clod"] {
            assert_eq!(
                AgentKind::from_spoken(spoken),
                Some(AgentKind::ClaudeCode),
                "{spoken}"
            );
        }
        assert_eq!(AgentKind::from_spoken("codex"), Some(AgentKind::Codex));
        assert_eq!(AgentKind::from_spoken("code x"), Some(AgentKind::Codex));
    }

    #[test]
    fn the_acp_variant_wins_over_the_plain_name() {
        assert_eq!(
            AgentKind::from_spoken("claude-acp"),
            Some(AgentKind::ClaudeAcp)
        );
    }

    #[test]
    fn an_unknown_name_is_not_guessed() {
        assert_eq!(AgentKind::from_spoken("some other tool"), None);
        assert_eq!(AgentKind::from_spoken(""), None);
    }

    #[test]
    fn only_claude_uses_the_legacy_transport() {
        for kind in ALL {
            let expected = if matches!(kind, AgentKind::ClaudeCode) {
                Transport::ClaudeStreamJson
            } else {
                Transport::Acp
            };
            assert_eq!(kind.transport(), expected, "{}", kind.id());
        }
    }

    #[test]
    fn ids_are_unique_and_round_trip() {
        let mut ids: Vec<&str> = ALL.iter().map(|a| a.id()).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count);
        for kind in ALL {
            assert_eq!(
                AgentKind::from_spoken(kind.id()),
                Some(kind),
                "{}",
                kind.id()
            );
        }
    }
}
