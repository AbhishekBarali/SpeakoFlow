// P0 spike: can an external program drive Claude Code over its bidirectional
// stream-json control protocol on Windows?
//
// This is a throwaway probe, not shipping code. It answers four questions that
// the whole "SpeakoFlow drives coding agents by voice" plan rests on:
//
//   1. Does `claude -p --input-format stream-json --output-format stream-json`
//      hand us a session id and streaming events?
//   2. When the agent wants permission, does the request arrive on the control
//      channel as `control_request { subtype: "can_use_tool" }`, and can we
//      answer allow/deny from here (i.e. can SpeakoFlow be the approval UI)?
//   3. Can we cancel a running turn with `control_request { subtype: "interrupt" }`?
//   4. Do we get enough structure to summarize state without scraping a TUI?
//
// Usage (from this directory):
//   node probe.mjs "<prompt>"
// Env:
//   CWD=<dir>            working directory handed to the agent (default: ./sandbox)
//   PERMISSION=allow|deny   how to answer permission requests (default: deny)
//   INTERRUPT_MS=<ms>    send an interrupt this long after the first tool use
//   MODEL=<model>        pass through to --model

import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import process from "node:process";

const prompt =
  process.argv.slice(2).join(" ") ||
  "Say hello in five words. Do not use any tools.";
const cwd = resolve(process.env.CWD ?? "./sandbox");
const permission = (process.env.PERMISSION ?? "deny").toLowerCase();
const interruptMs = process.env.INTERRUPT_MS
  ? Number(process.env.INTERRUPT_MS)
  : null;

mkdirSync(cwd, { recursive: true });

const args = [
  "-p",
  "--input-format",
  "stream-json",
  "--output-format",
  "stream-json",
  "--verbose",
  "--include-partial-messages",
  "--replay-user-messages",
  "--permission-mode",
  "default",
];
if (process.env.MODEL) args.push("--model", process.env.MODEL);
// Undocumented in --help, but this is how the official SDKs route permission
// prompts back over the control channel instead of auto-denying them.
if (process.env.PERMISSION_TOOL)
  args.push("--permission-prompt-tool", process.env.PERMISSION_TOOL);

const started = Date.now();
const ms = () => String(Date.now() - started).padStart(6, " ");
const log = (tag, msg) =>
  console.log(`[${ms()}ms] ${tag.padEnd(22)} ${msg ?? ""}`);

log("SPAWN", `claude ${args.join(" ")}`);
log("CWD", cwd);
log("PERMISSION_POLICY", permission);

const child = spawn("claude", args, {
  cwd,
  stdio: ["pipe", "pipe", "pipe"],
  shell: false,
  windowsHide: true,
});

let sessionId = null;
let firstTokenAt = null;
let interruptSent = false;
let partialChars = 0;
const toolsSeen = [];
const permissionRequests = [];

function send(obj) {
  child.stdin.write(JSON.stringify(obj) + "\n");
}

function sendUser(text) {
  send({
    type: "user",
    message: { role: "user", content: [{ type: "text", text }] },
    parent_tool_use_id: null,
    session_id: sessionId ?? "",
  });
}

function sendInterrupt() {
  if (interruptSent) return;
  interruptSent = true;
  log("-> INTERRUPT", "control_request { subtype: interrupt }");
  send({
    type: "control_request",
    request_id: `int_${Date.now()}`,
    request: { subtype: "interrupt" },
  });
}

// Answer a server-initiated permission request. This is the single most
// important line in the spike: if this works, SpeakoFlow can be the thing that
// says "allow" instead of the user tabbing to a terminal.
function answerPermission(requestId, toolName) {
  const response =
    permission === "allow"
      ? {
          subtype: "success",
          request_id: requestId,
          response: { behavior: "allow", updatedInput: undefined },
        }
      : {
          subtype: "success",
          request_id: requestId,
          response: {
            behavior: "deny",
            message: "Denied by SpeakoFlow spike.",
          },
        };
  log("-> PERMISSION", `${permission.toUpperCase()} ${toolName}`);
  send({ type: "control_response", response });
}

let buf = "";
child.stdout.on("data", (chunk) => {
  buf += chunk.toString("utf8");
  let idx;
  while ((idx = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, idx).trim();
    buf = buf.slice(idx + 1);
    if (line) handleLine(line);
  }
});

function handleLine(line) {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    log("NON-JSON", line.slice(0, 200));
    return;
  }

  switch (msg.type) {
    case "system":
      if (msg.subtype === "init") {
        sessionId = msg.session_id;
        log(
          "<= system/init",
          `session_id=${msg.session_id} model=${msg.model} tools=${(msg.tools ?? []).length} cwd=${msg.cwd ?? ""}`,
        );
        log(
          "",
          `   permissionMode=${msg.permissionMode} slash=${(msg.slash_commands ?? []).length}`,
        );
      } else {
        log(`<= system/${msg.subtype}`, JSON.stringify(msg).slice(0, 200));
      }
      break;

    case "stream_event": {
      // Token-level deltas. Proves we can show live progress without a PTY.
      const ev = msg.event ?? {};
      if (ev.type === "content_block_delta") {
        const d = ev.delta ?? {};
        const text = d.text ?? d.partial_json ?? "";
        partialChars += text.length;
        if (!firstTokenAt && text) {
          firstTokenAt = Date.now() - started;
          log("<= FIRST TOKEN", `${firstTokenAt}ms`);
          // Interrupt mid-stream: the "stop it" path, which must work while the
          // agent is talking, not only while a tool is pending.
          if (interruptMs !== null) setTimeout(sendInterrupt, interruptMs);
        }
      } else if (
        ev.type === "content_block_start" &&
        ev.content_block?.type === "tool_use"
      ) {
        log("<= tool_use start", ev.content_block.name);
      }
      break;
    }

    case "assistant": {
      const content = msg.message?.content ?? [];
      for (const block of content) {
        if (block.type === "text") {
          log("<= assistant text", JSON.stringify(block.text).slice(0, 160));
        } else if (block.type === "tool_use") {
          toolsSeen.push(block.name);
          log(
            "<= assistant tool_use",
            `${block.name} ${JSON.stringify(block.input).slice(0, 160)}`,
          );
          if (interruptMs !== null) setTimeout(sendInterrupt, interruptMs);
        } else if (block.type === "thinking") {
          log(
            "<= assistant thinking",
            `${(block.thinking ?? "").length} chars`,
          );
        }
      }
      break;
    }

    case "user":
      log(
        "<= user (replay)",
        JSON.stringify(msg.message?.content ?? "").slice(0, 160),
      );
      break;

    case "control_request": {
      // Server-initiated. This is the approval channel.
      const req = msg.request ?? {};
      log("<= CONTROL_REQUEST", `subtype=${req.subtype} id=${msg.request_id}`);
      console.log("      full payload:", JSON.stringify(msg));
      if (req.subtype === "can_use_tool") {
        permissionRequests.push(req.tool_name);
        answerPermission(msg.request_id, req.tool_name);
      }
      break;
    }

    case "control_response":
      log("<= control_response", JSON.stringify(msg).slice(0, 200));
      break;

    case "result":
      log(
        "<= RESULT",
        `subtype=${msg.subtype} turns=${msg.num_turns} cost=$${msg.total_cost_usd ?? 0} dur=${msg.duration_ms}ms`,
      );
      if (msg.result) log("", `   ${JSON.stringify(msg.result).slice(0, 200)}`);
      summarize();
      child.stdin.end();
      break;

    default:
      log(`<= ${msg.type}`, JSON.stringify(msg).slice(0, 200));
  }
}

child.stderr.on("data", (d) =>
  log("STDERR", d.toString().trim().slice(0, 300)),
);

child.on("close", (code) => {
  log("EXIT", `code=${code}`);
  process.exit(code ?? 0);
});

function summarize() {
  console.log("\n================ SPIKE RESULT ================");
  console.log(
    `session_id captured        : ${sessionId ? "YES (" + sessionId + ")" : "NO"}`,
  );
  console.log(
    `token streaming            : ${firstTokenAt !== null ? "YES (first token " + firstTokenAt + "ms, " + partialChars + " chars)" : "NO"}`,
  );
  console.log(
    `tool calls observed        : ${toolsSeen.length ? toolsSeen.join(", ") : "none"}`,
  );
  console.log(
    `permission requests to us  : ${permissionRequests.length ? permissionRequests.join(", ") : "none"}`,
  );
  console.log(`interrupt sent             : ${interruptSent ? "YES" : "no"}`);
  console.log("=============================================\n");
}

// Kick off the turn once the process is up.
setTimeout(() => {
  // Advertise ourselves as a client that can answer permission prompts. The
  // official SDKs send this before the first turn; the response tells us which
  // capabilities the CLI accepted.
  log("-> initialize", "control_request { subtype: initialize }");
  send({
    type: "control_request",
    request_id: "init_1",
    request: {
      subtype: "initialize",
      capabilities: { canUseTool: true, permissions: true },
      canUseTool: true,
      hooks: {},
    },
  });
  setTimeout(() => {
    log("-> user message", JSON.stringify(prompt));
    sendUser(prompt);
  }, 200);
}, 300);
