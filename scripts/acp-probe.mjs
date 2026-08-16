#!/usr/bin/env node
/**
 * ACP conformance probe.
 *
 * Drives any Agent Client Protocol agent through the full loop we depend on --
 * `initialize` -> `session/new` -> `session/prompt` -> tool calls -> permission
 * request -> `stopReason` -- and prints a readable trace of every frame.
 *
 * Why this exists: before wiring a new agent into SpeakoFlow's ACP client we need
 * to know what it actually does, not what its docs claim. Agents differ in which
 * optional capabilities they advertise, whether permission requests carry a tool
 * `kind`, whether request ids are numbers or strings, and what vendor-prefixed
 * extension methods they emit alongside the standard ones. Ten seconds with this
 * script answers all of that.
 *
 * Usage:
 *   node scripts/acp-probe.mjs                       # defaults to kiro-cli acp
 *   node scripts/acp-probe.mjs claude-agent-acp
 *   node scripts/acp-probe.mjs codex-acp
 *   node scripts/acp-probe.mjs "npx -y @google/gemini-cli --experimental-acp"
 *
 * Flags:
 *   --prompt "..."   Task to send (default: create a hello.txt)
 *   --cwd <path>     Working directory (default: a fresh temp dir)
 *   --deny           Reject the first permission request instead of allowing it
 *   --raw            Print every frame verbatim instead of the compact trace
 *   --timeout <ms>   Give up after this long (default: 120000)
 *
 * The default prompt only creates a file inside a throwaway temp directory, so a
 * default run cannot touch anything you care about. It does consume a small
 * amount of the agent's quota, because a real model turn runs.
 */
import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const argv = process.argv.slice(2);

/** Pull `--name value` out of argv, returning the value or a fallback. */
function flag(name, fallback) {
  const i = argv.indexOf(`--${name}`);
  if (i === -1 || i === argv.length - 1) return fallback;
  const value = argv[i + 1];
  argv.splice(i, 2);
  return value;
}

/** Pull a boolean `--name` out of argv. */
function bool(name) {
  const i = argv.indexOf(`--${name}`);
  if (i === -1) return false;
  argv.splice(i, 1);
  return true;
}

const deny = bool("deny");
const raw = bool("raw");
const timeoutMs = Number(flag("timeout", "120000"));
const prompt = flag(
  "prompt",
  "Create a file named hello.txt in the current directory containing exactly: hello",
);
const cwd = flag("cwd", mkdtempSync(join(tmpdir(), "acp-probe-")));

// Everything left in argv is the agent command. Accepts either a bare binary
// name or a full command line in one quoted argument.
const command = argv.length ? argv.join(" ") : "kiro-cli acp";

console.log(`agent   : ${command}`);
console.log(`cwd     : ${cwd}`);
console.log(`prompt  : ${prompt}`);
console.log(`on perm : ${deny ? "reject_once" : "allow_once"}`);
console.log("-".repeat(72));

const child = spawn(command, { stdio: "pipe", shell: true });

/** toolCallId -> what we learned about it from session/update. */
const toolCalls = new Map();
let sessionId = null;
let finished = false;

function send(obj) {
  const line = JSON.stringify(obj);
  console.log("-->", line.length > 220 ? `${line.slice(0, 220)}...` : line);
  child.stdin.write(`${line}\n`);
}

/**
 * Standard ACP session updates and the vendor-prefixed variants some agents also
 * emit (Kiro sends `_kiro.dev/session/update` for chunk-level events alongside
 * the standard `session/update`). Treat both as the same stream.
 */
function isSessionUpdate(method) {
  return method === "session/update" || method.endsWith("/session/update");
}

function isPermissionRequest(method) {
  return (
    method === "session/request_permission" ||
    method.endsWith("/session/request_permission")
  );
}

function handleSessionUpdate(update) {
  const kind = update.sessionUpdate;
  switch (kind) {
    case "agent_message_chunk":
    case "agent_thought_chunk": {
      const text = update.content?.text ?? "";
      if (text.trim()) {
        const label = kind === "agent_thought_chunk" ? "think" : "text ";
        process.stdout.write(`${label} : ${text.replace(/\n/g, " ")}\n`);
      }
      break;
    }
    case "tool_call":
    case "tool_call_chunk":
    case "tool_call_update": {
      // Permission requests may omit `kind` and `title`, so remember whatever we
      // saw here. The approval policy depends on this correlation.
      const prev = toolCalls.get(update.toolCallId) ?? {};
      const merged = {
        title: update.title ?? prev.title,
        kind: update.kind ?? prev.kind,
        status: update.status ?? prev.status,
      };
      toolCalls.set(update.toolCallId, merged);
      console.log(
        `tool  : [${kind}] kind=${merged.kind ?? "?"} status=${
          merged.status ?? "?"
        } ${merged.title ?? ""}`,
      );
      break;
    }
    case "plan":
      console.log(`plan  : ${JSON.stringify(update.entries ?? update)}`);
      break;
    default:
      console.log(`update: ${kind}`);
  }
}

let buffer = "";
child.stdout.on("data", (chunk) => {
  buffer += chunk.toString();
  let newline;
  while ((newline = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, newline).trim();
    buffer = buffer.slice(newline + 1);
    if (!line) continue;

    let msg;
    try {
      msg = JSON.parse(line);
    } catch {
      console.log(`nonjson: ${line.slice(0, 200)}`);
      continue;
    }

    if (raw) console.log("<--", line);

    const method = msg.method ?? "";

    if (isSessionUpdate(method)) {
      if (!raw) handleSessionUpdate(msg.params?.update ?? {});
      continue;
    }

    if (isPermissionRequest(method)) {
      const call = toolCalls.get(msg.params?.toolCall?.toolCallId) ?? {};
      const title = msg.params?.toolCall?.title ?? call.title ?? "(untitled)";
      console.log(
        `PERM  : ${title} | correlated kind=${call.kind ?? "UNKNOWN"}`,
      );
      console.log(
        `        options: ${JSON.stringify(msg.params?.options ?? [])}`,
      );
      if (msg.params?._meta) {
        console.log(`        _meta: ${JSON.stringify(msg.params._meta)}`);
      }
      const options = msg.params?.options ?? [];
      const wanted = deny ? "reject_once" : "allow_once";
      const chosen =
        options.find((o) => o.kind === wanted) ?? options[0] ?? null;
      send({
        jsonrpc: "2.0",
        id: msg.id,
        result: {
          outcome: chosen
            ? { outcome: "selected", optionId: chosen.optionId }
            : { outcome: "cancelled" },
        },
      });
      continue;
    }

    // Client-side methods the agent may call on us. The probe declines them so we
    // can see which ones an agent actually relies on.
    if (method.startsWith("fs/") || method.startsWith("terminal/")) {
      console.log(
        `client: ${method} ${JSON.stringify(msg.params ?? {}).slice(0, 200)}`,
      );
      send({
        jsonrpc: "2.0",
        id: msg.id,
        error: { code: -32601, message: "probe does not implement this" },
      });
      continue;
    }

    if (method) {
      // Vendor extensions (`_kiro.dev/metadata`, `_kiro.dev/commands/available`,
      // and friends). Worth seeing, not worth acting on.
      if (!raw) console.log(`ext   : ${method}`);
      continue;
    }

    // Responses to our own requests, keyed by the id we sent.
    if (msg.id === 1) {
      console.log(`init  : ${JSON.stringify(msg.result ?? msg.error)}`);
      if (msg.error) {
        finished = true;
        child.kill();
        process.exit(1);
      }
      send({
        jsonrpc: "2.0",
        id: 2,
        method: "session/new",
        params: { cwd, mcpServers: [] },
      });
    } else if (msg.id === 2) {
      if (msg.error) {
        console.log(`session/new failed: ${JSON.stringify(msg.error)}`);
        finished = true;
        child.kill();
        process.exit(1);
      }
      sessionId = msg.result.sessionId;
      console.log(`session: ${sessionId}`);
      if (msg.result.modes) {
        console.log(`modes  : ${JSON.stringify(msg.result.modes)}`);
      }
      send({
        jsonrpc: "2.0",
        id: 3,
        method: "session/prompt",
        params: { sessionId, prompt: [{ type: "text", text: prompt }] },
      });
    } else if (msg.id === 3) {
      console.log("-".repeat(72));
      console.log(`result : ${JSON.stringify(msg.result ?? msg.error)}`);
      finished = true;
      child.kill();
      process.exit(0);
    }
  }
});

child.stderr.on("data", (chunk) => {
  const text = chunk.toString().trim();
  if (text) console.log(`stderr: ${text.slice(0, 400)}`);
});

child.on("error", (err) => {
  console.log(`spawn failed: ${err.message}`);
  process.exit(1);
});

send({
  jsonrpc: "2.0",
  id: 1,
  method: "initialize",
  params: {
    protocolVersion: 1,
    clientCapabilities: {
      fs: { readTextFile: false, writeTextFile: false },
      terminal: false,
    },
  },
});

setTimeout(() => {
  if (!finished) {
    console.log("-".repeat(72));
    console.log("timed out with no final result");
    child.kill();
    process.exit(1);
  }
}, timeoutMs);
