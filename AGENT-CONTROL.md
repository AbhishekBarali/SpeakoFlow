# Voice control for coding agents

Status: **P1–P3 shipped behind the experimental flag. Claude Code only.**
Last updated: 2026-08-13

## The goal

Let a developer run several coding agents at once and never sit in a terminal
watching them. You hold a hotkey, say what you want, and SpeakoFlow starts the
agent, watches it, and answers questions about it out loud.

The five things a user should be able to say:

| Said out loud                              | What happens                                                                        |
| ------------------------------------------ | ----------------------------------------------------------------------------------- |
| "Build me a login page in the web project" | SpeakoFlow cleans up the dictated prompt and starts an agent session in that folder |
| "What's happening?"                        | A one-breath summary of every running session                                       |
| "Is the frontend one done?"                | Status of one session, including whether it's blocked on you                        |
| "Allow it"                                 | Answers the agent's permission prompt — you never tab to the terminal               |
| "Stop that one"                            | Cancels the running turn                                                            |

Plus: open the folder an agent is working in, and open a window to read the
detail when you actually want to look.

Why this and not the obvious voice-assistant features: "open Chrome", "play a
song on YouTube" are commodities. The agent layer is not, and SpeakoFlow already
owns every piece it needs — global hotkey, local speech-to-text, text-to-speech,
an always-on-top panel, and a prompt-cleanup pass.

## Positioning

There are dozens of tools that manage parallel coding agents: claude-squad
(~7k stars), CodexMonitor (~4k), ccmanager, agent-deck, clideck, vibe-kanban,
Fleet Commander, a VS Code Agent Dashboard, and Claude Code's own Agent View.
Every one of them is a dashboard, a TUI, or a tmux/worktree orchestrator.

**Nobody owns the ambient voice layer.** That's the wedge. The goal is not to
out-dashboard claude-squad; it's to be the thing you talk to while your hands
are somewhere else.

A second advantage: that whole ecosystem is built on tmux, which doesn't exist
on Windows. Windows-first is an open niche here, not a compromise.

## The mechanism, and the one we rejected

**Rejected — screen-scraping the agent's TUI.** claude-squad and ccmanager spawn
the agent in a pseudo-terminal and regex the _rendered screen_ to guess state:
`esc to interrupt` means busy, `Do you want` means waiting. It's a permanent
maintenance tax — ccmanager issue #227 is "state detection shows idle when
Claude Code is busy", and the fix needed prompt-box boundary detection, spinner
label matching, and a minimum-state-duration guard against redraw flicker. Every
agent UI release can break it. Study their state machines and bug trackers;
don't adopt their method.

**Chosen — the agents' own control protocols.** All three targets speak
structured, bidirectional JSON over stdio, including server-initiated approval
requests:

- **Claude Code** — `--input-format stream-json --output-format stream-json`,
  with `control_request { subtype: "can_use_tool" }` for permissions and
  `{ subtype: "interrupt" }` to stop. **This is what we build on first.**
- **Codex** — `codex app-server`, JSON-RPC 2.0, approvals with
  `accept | acceptForSession | decline | cancel`.
- **Kiro CLI** — its TUI already speaks ACP internally (see the documented
  `KIRO_ACP_RECORD_PATH` debug setting), but that surface isn't documented for
  outside use, and its agent hooks were reported broken in 2.0.1.

**ACP (Agent Client Protocol)** is the standardised version of this —
`session/new`, `session/prompt`, `session/update`, `session/request_permission`,
`session/cancel` — with 25+ agents supporting it natively (Gemini CLI, Copilot
CLI, Goose, Cline) and adapters for Claude Code and Codex. It's the long-term
answer to "one integration instead of one per agent". Every existing ACP client
is an editor; SpeakoFlow would be the first voice one.

## Two session kinds

Owning the process is what makes control possible, and it has a consequence:
a protocol-driven session is headless, so there's no terminal window to look at.

- **Managed** — SpeakoFlow spawned it. Full control: status, stop, approve,
  open folder. SpeakoFlow renders the transcript itself.
- **Foreign** — the user started it in their own terminal. Read-only awareness
  via hooks and session JSONL. Status and notifications only; no stop, no
  approve.

Build managed as the product, keep foreign as a compatibility layer.

The window this needs is **read-only** — a session list, and per session the
messages, files touched, and pending approvals. No typing into it, no ANSI
rendering, no terminal emulation. Voice stays the interface; the window is the
escape hatch for the 5% of the time you want to read a diff with your eyes.

## What the spike proved

Verified on Windows 11 against Claude Code **2.1.81** using Azure AI Foundry
(`claude-opus-5`). Harness: `spikes/claude-bridge/`.

```
claude -p --input-format stream-json --output-format stream-json --verbose \
  --include-partial-messages --replay-user-messages \
  --permission-mode default --model <deployed-model> \
  --permission-prompt-tool stdio
```

| Capability                        | Evidence                                                                                               |
| --------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Start a session, address it by id | `system/init` → `session_id`, `model`, `tools=29`, `cwd`                                               |
| Live streaming, no PTY            | `stream_event` deltas; first token 4.7s                                                                |
| Permission request routed to us   | `control_request { subtype: "can_use_tool" }`                                                          |
| Granting from outside works       | replied `{ behavior: "allow" }` → `hello.txt` created, 3 bytes, `hi`                                   |
| Cancel a running turn             | `interrupt` → `{ still_queued: [] }`, `[Request interrupted by user]`, result `error_during_execution` |
| Same from Rust                    | dependency-free `std::process` probe captured its own session id and completed a turn                  |

### Hard-won details

**Permission routing is opt-in and undocumented.** Without
`--permission-prompt-tool stdio` _and_ an `initialize` control request, Claude
Code silently **auto-denies** and emits only `system/permission_denied`. The
flag is absent from `--help`, so pin the tested CLI version.

**The permission payload is rich enough to speak aloud.** It carries
`tool_name`, `display_name`, the full `input` (exact path and content),
a human `description`, `tool_use_id`, and `permission_suggestions` — the CLI
itself proposed `setMode: acceptEdits, destination: session`. So "allow once" vs
"allow for this session" is native, not invented.

**Model selection needs real discovery.** `settings.json` requested
`claude-sonnet-5`, whose Foundry deployment was `status=running`, not
`succeeded`. Claude Code's error only suggests the next-older model, one run at
a time. Enumerate deployments; don't trust config.

**Never wait on the coding agent to answer a status question.** An Opus turn
took 25s to first token. Status must be answered from our own digest with our
own faster model. Every `result` includes `total_cost_usd`, so per-session cost
display is free.

## Architecture sketch

```
hotkey / voice
   │
   ├─ prompt cleanup ......... reuse actions.rs::post_process_transcription
   │
   ├─ agents/ (new module in src-tauri)
   │    ├─ session manager ... spawns claude.exe, owns stdin/stdout per session
   │    ├─ event digest ...... rolling per-session state, NOT raw transcripts:
   │    │                      status, elapsed, last tool, files touched,
   │    │                      last assistant line, pending approval, cost
   │    ├─ registry .......... all sessions, managed + foreign
   │    └─ foreign watcher ... hooks + session JSONL for terminals we didn't start
   │
   ├─ assistant tool loop ..... list_agent_sessions, get_agent_session,
   │                            start_agent_session, cancel_agent_session,
   │                            answer_agent_permission, open_session_folder
   │
   └─ TTS + panel ............. spoken status, spoken approval prompts,
                                read-only session window
```

The digest is the performance-critical piece: a status question must be
answerable with roughly 200 tokens of pre-rendered text and zero file I/O.

## Safety requirements

These are not optional, and two of them are new risk introduced by this feature.

1. **Voice approvals.** Read the actual command or path aloud before approving.
   Require an explicit confirmation word. Classify obviously destructive
   commands and refuse to voice-approve them at all. Never auto-approve. Log
   every decision visibly. A misheard "yes" must never be able to delete
   something.
2. **Prompt injection.** The assistant will read agent transcripts — which
   contain web pages, error text, and code from anywhere — while holding the
   power to spawn, stop, and approve agents. Transcript content goes in as
   delimiter-wrapped, advisory, instruction-stripped data, using the same
   discipline `memory.rs` already applies to personal memory.
3. **No new shell surface.** SpeakoFlow never gets a command prompt. It talks to
   the agent; the agent runs commands under its own existing sandbox and
   permission model.
4. **Model tiering.** Small local GGUF models are unreliable at multi-tool
   calling. This feature needs a mid-tier model or better, and the basic status
   line should still work with no LLM at all. It's summarising structured
   events, not coding.
5. **Tool-prompt bloat.** Tool definitions ride in every request. Gate the agent
   tool group so unrelated questions don't pay for it in time-to-first-token.

## Current state of the code

Everything below is behind the existing `experimental_enabled` setting
(Settings → General → Experimental). With it off nothing is exposed: no tools
reach the model, and the sidebar section is hidden.

### Backend

- `src-tauri/src/agents/mod.rs` — session manager. Spawns `claude`, one reader
  thread per session folding events into the digest, permission parking, risk
  classification, cancel/close, spoken notifications.
- `src-tauri/src/commands/agents.rs` — seven Tauri commands for the UI,
  including the force-approval path that voice cannot reach.
- `src-tauri/src/assistant.rs` — eight tool definitions and their dispatch,
  appended after the existing web and screen tools.
- `src-tauri/src/lib.rs` — registers the manager and commands, and kills every
  session on `RunEvent::Exit`.

Assistant tools: `list_agent_sessions`, `get_agent_session`,
`start_agent_session`, `send_agent_message`, `cancel_agent_session`,
`answer_agent_permission`, `close_agent_session`, `open_agent_folder`.

Events to the webview: `agent-session-update` (one session's digest, on every
meaningful change) and `agent-notification` (a spoken-ready sentence, only for
the transitions worth interrupting someone for — blocked, finished, failed).

Spoken notifications reuse the assistant's own TTS split: Kokoro renders in the
webview via `assistant-tts`, every other engine plays through
`tts::speak_remote`. Silent unless `assistant_tts_enabled` is already on, so it
can never surprise anyone with audio.

### Frontend

- `src/components/settings/agents/AgentsSettings.tsx` — the Agents section:
  blocked-first approval cards, the session list, stop / open folder / close.
- `src/components/Sidebar.tsx` — adds the `agents` section, gated on
  `experimental_enabled`.
- `src/i18n/locales/en/translation.json` — `sidebar.agents`,
  `sectionSubtitles.agents`, `settings.agents.*`.
- `src/bindings.ts` — hand-added command signatures and the three new types.

### Finding the CLI, and its environment

`agents/env.rs` exists because two real failures made inherited environment
untrustworthy:

1. A handed-off terminal reported `'claude' is not recognized` **and**
   `'DOSKEY' is not recognized`. DOSKEY lives in System32, so the child had no
   usable `PATH` at all, even though the machine's own `PATH` was healthy (48
   user entries, System32 in the machine hive).
2. Sessions failed with `Not logged in · Please run /login` while the user's
   Foundry credentials were set correctly — because they were set _after_
   SpeakoFlow launched. A running Windows process never sees a later `setx`, and
   closing SpeakoFlow's window only hides it to the tray, so a stale environment
   survives what looks like a restart. `anthropics/claude-code#30132` is the same
   symptom in the VS Code extension.

So nothing is left to inheritance:

- `resolve_claude()` returns an absolute path, checking
  `SPEAKOFLOW_CLAUDE_PATH`, then the known install locations
  (`~/.local/bin`, `~/.claude/local`, `%APPDATA%\npm`, `~/.bun/bin`,
  `%LOCALAPPDATA%\Programs\claude`, `/usr/local/bin`, `/opt/homebrew/bin`),
  then every directory on the effective `PATH`.
- `effective_path()` merges the live `PATH` with both registry hives and, on
  Windows, appends the system directories as a floor.
- `forwarded_vars()` reads the provider variables (`CLAUDE_CODE_USE_FOUNDRY`,
  `ANTHROPIC_FOUNDRY_*`, `ANTHROPIC_API_KEY`, Bedrock and Vertex switches, and
  friends) from `HKCU\Environment` / the machine hive and fills only the gaps, so
  an explicitly exported value still wins. Names are logged, never values.

The terminal handoff uses the same three, plus an absolute `cmd.exe` from
`%SYSTEMROOT%` and Windows Terminal resolved by path rather than by name.

### Model selection

`SPEAKOFLOW_AGENT_MODEL` overrides the model handed to the CLI, and is read
through the same registry-aware lookup, so `setx` takes effect without a restart.
Deliberately an environment variable rather than a settings field while the shape
settles, since a real setting would ripple into the generated bindings.

### The spike

`spikes/claude-bridge/` is still there as a reference harness — `probe.mjs` for
the protocol and a dependency-free Rust proof. It sits outside the app build
(its `Cargo.toml` declares an empty `[workspace]`) and can be deleted with no
effect on SpeakoFlow.

## Verification

- `cargo test --lib` — 331 passed, 0 failed, 3 ignored
- `cargo clippy --lib` — exit 0
- `cargo fmt --all --check` — clean
- `bun run lint` (ESLint, including the i18n rule) — exit 0
- `bun run build` (`tsc && vite build`) — exit 0
- `bun run format:check` (Prettier + `cargo fmt`) — clean

Twelve unit tests cover the digest: init capture, tool and file recording,
permission parking, the risk classifier, success and failure results, keeping the
"stopped" label through the error result an interrupt produces, loose argument
parsing, duration wording, which transitions notify, and which stay silent.

## Not verified

The end-to-end voice path has not been exercised in a running app, which needs a
GUI session. The protocol underneath is proven and the digest logic is
unit-tested, but "hold the hotkey, say status, hear the answer" is untested.

`src/bindings.ts` was hand-written to match what tauri-specta generates.
TypeScript compiles against it, but the next `bun run tauri dev` regenerates the
file and may produce a cosmetic diff.

## Plan

- **P0 — protocol spike.** Done.
- **P1 — read-only awareness.** Done: registry, digest, voice status, event and
  spoken notifications.
- **P2 — start and stop by voice.** Done: start, follow-up message, cancel,
  close, open folder.
- **P3 — approvals.** Done: voice approves ordinary actions; destructive ones are
  refused on the voice path and require the on-screen confirmation in the Agents
  section, where the exact command is visible.
- **P4 — richer session view.** Not started: a live transcript rather than just
  the last line, diffs, and starting a session from the UI.
- **Later — more agents.** Codex via `app-server`, then ACP as the universal path.

## Open decisions

1. Foreign sessions — the ones started in the user's own terminal — are still
   unsupported. Hooks plus session JSONL would cover status and notifications,
   but never stop or approve.
2. Should this get its own window rather than a settings section, once there is a
   transcript worth showing?
3. The feature cannot work with a small local model, which is an explicit
   exception to SpeakoFlow's local-first positioning and needs wording in the
   README before it ships to everyone.
4. Sessions do not survive an app restart. `--resume` plus the stored
   `agent_session_id` would fix that.
