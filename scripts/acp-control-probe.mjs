#!/usr/bin/env node
/**
 * ACP *control surface* probe.
 *
 * `acp-probe.mjs` answers "can we drive a turn end to end". This one answers the
 * separate question that decides how much of a real coding CLI we can offer by
 * voice: **what can be changed about a session while it is open?**
 *
 * Specifically it reports, per agent:
 *
 *   1. Everything `initialize` advertises, verbatim.
 *   2. Everything `session/new` returns, verbatim — including `models`
 *      (`SessionModelState`) and `modes`, which are the protocol's two
 *      switchable dimensions.
 *   3. Every vendor notification that arrives unprompted, verbatim. Kiro's
 *      `_kiro.dev/commands/available` is how its slash commands (`/compact`,
 *      `/model`, `/usage`, …) become discoverable, and `_kiro.dev/metadata`
 *      carries live context-window usage.
 *   4. Whether `session/set_model` exists, by calling it and reading the error
 *      code: `-32601` means the method is absent, anything else means it is
 *      implemented and merely disliked our arguments.
 *   5. Whether a slash command sent as prompt text is executed as a command.
 *   6. Whether `session/load` accepts the session id we were just given.
 *   7. What happens to a second `session/prompt` sent while one is in flight —
 *      the question behind mid-turn steering.
 *
 * None of steps 1–4 or 6 runs a model turn, so they cost no quota. Step 5 may,
 * and step 7 deliberately does; both are opt-in.
 *
 * Usage:
 *   node scripts/acp-control-probe.mjs                      # kiro-cli acp
 *   node scripts/acp-control-probe.mjs "kiro-cli acp --agent-engine v3"
 *   node scripts/acp-control-probe.mjs codex-acp
 *
 * Flags:
 *   --command <name>  Slash command to try as prompt text (default: auto-pick a
 *                     read-only one from what the agent advertises)
 *   --no-command      Skip the slash-command test
 *   --steer           Also run the mid-turn steering test (costs a model turn)
 *   --cwd <path>      Working directory (default: a fresh temp dir)
 *   --settle <ms>     How long to wait for unprompted notifications (default 3000)
 *   --timeout <ms>    Per-request timeout (default 20000)
 */
import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const argv = process.argv.slice(2);

function flag(name, fallback) {
  const i = argv.indexOf(`--${name}`);
  if (i === -1 || i === argv.length - 1) return fallback;
  const value = argv[i + 1];
  argv.splice(i, 2);
  return value;
}

function bool(name) {
  const i = argv.indexOf(`--${name}`);
  if (i === -1) return false;
  argv.splice(i, 1);
  return true;
}

const skipCommand = bool("no-command");
const doSteer = bool("steer");
const wantedCommand = flag("command", null);
const settleMs = Number(flag("settle", "3000"));
const timeoutMs = Number(flag("timeout", "20000"));
const cwd = flag("cwd", mkdtempSync(join(tmpdir(), "acp-control-")));
const command = argv.length ? argv.join(" ") : "kiro-cli acp";

console.log(`agent  : ${command}`);
console.log(`cwd    : ${cwd}`);
console.log("=".repeat(76));

const child = spawn(command, { stdio: "pipe", shell: true });
child.on("error", (err) => {
  console.log(`spawn failed: ${err.message}`);
  process.exit(1);
});
child.stderr.on("data", (chunk) => {
  const text = chunk.toString().trim();
  if (text) console.log(`stderr : ${text.slice(0, 300)}`);
});

/** Pending requests, id -> resolve. */
const pending = new Map();
/** Unprompted notifications, method -> first params seen. */
const notifications = new Map();
/** The most recent vendor metadata frame, which reflects current settings. */
let latestMetadata = null;
let nextId = 1;
let buffer = "";

function write(obj) {
  child.stdin.write(`${JSON.stringify(obj)}\n`);
}

/** Send a request and resolve with `{ result }` or `{ error }`. */
function call(method, params) {
  const id = nextId++;
  write({ jsonrpc: "2.0", id, method, params });
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      resolve({ error: { code: "timeout", message: `no reply in ${timeoutMs}ms` } });
    }, timeoutMs);
    pending.set(id, (msg) => {
      clearTimeout(timer);
      resolve(msg);
    });
  });
}

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
      console.log(`nonjson: ${line.slice(0, 160)}`);
      continue;
    }
    if (msg.method) {
      // The agent asking us something, or telling us something.
      if (msg.id !== undefined) {
        // A request. Permission prompts get denied: this probe is not here to
        // let an agent write anything.
        if (msg.method.endsWith("session/request_permission")) {
          const options = msg.params?.options ?? [];
          const reject =
            options.find((o) => o.kind === "reject_once") ?? options[0] ?? null;
          console.log(`PERM   : denied "${msg.params?.toolCall?.title ?? "?"}"`);
          write({
            jsonrpc: "2.0",
            id: msg.id,
            result: {
              outcome: reject
                ? { outcome: "selected", optionId: reject.optionId }
                : { outcome: "cancelled" },
            },
          });
        } else {
          write({
            jsonrpc: "2.0",
            id: msg.id,
            error: { code: -32601, message: "probe does not implement this" },
          });
        }
        continue;
      }
      if (!notifications.has(msg.method)) {
        notifications.set(msg.method, msg.params ?? null);
      }
      // Metadata is pushed repeatedly and is the only place some agents report
      // the *current* effort level, so the newest one is kept separately: the
      // first one cannot show whether a command we sent afterwards took effect.
      if (/\/metadata$/.test(msg.method)) {
        latestMetadata = msg.params ?? null;
      }
      continue;
    }
    const resolve = pending.get(msg.id);
    if (resolve) {
      pending.delete(msg.id);
      resolve(msg);
    }
  }
});

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const show = (value) => JSON.stringify(value, null, 2);

function section(title) {
  console.log(`\n${"-".repeat(76)}\n${title}\n${"-".repeat(76)}`);
}

/** Does this error mean "no such method"? */
function methodMissing(error) {
  return error?.code === -32601;
}

async function main() {
  section("1. initialize");
  const init = await call("initialize", {
    protocolVersion: 1,
    clientCapabilities: {
      fs: { readTextFile: false, writeTextFile: false },
      terminal: false,
    },
    clientInfo: { name: "SpeakoFlow control probe", version: "1" },
  });
  console.log(show(init.result ?? init.error));
  if (!init.result) {
    child.kill();
    process.exit(1);
  }

  section("2. session/new (verbatim — look for `models` and `modes`)");
  const session = await call("session/new", { cwd, mcpServers: [] });
  console.log(show(session.result ?? session.error));
  const sessionId = session.result?.sessionId;
  if (!sessionId) {
    child.kill();
    process.exit(1);
  }

  await sleep(settleMs);
  section(`3. unprompted notifications in the first ${settleMs}ms`);
  if (notifications.size === 0) {
    console.log("(none)");
  }
  for (const [method, params] of notifications) {
    console.log(`\n### ${method}\n${show(params)}`);
  }

  section("4. session/set_model");
  const models = session.result?.models;
  const modelIds = (models?.availableModels ?? [])
    .map((m) => m.modelId ?? m.id)
    .filter(Boolean);
  console.log(
    modelIds.length
      ? `advertised models: ${modelIds.join(", ")}`
      : "session/new advertised no models; calling anyway to see if the method exists",
  );
  // Prefer a model that is not the current one, so a success is observable.
  const current = models?.currentModelId;
  const candidate =
    modelIds.find((id) => id !== current) ?? modelIds[0] ?? "probe-nonexistent-model";
  console.log(`calling session/set_model with modelId=${candidate}`);
  const setModel = await call("session/set_model", { sessionId, modelId: candidate });
  console.log(show(setModel.result ?? setModel.error));
  console.log(
    methodMissing(setModel.error)
      ? "=> NOT IMPLEMENTED by this agent"
      : "=> method exists (result or a non -32601 error)",
  );

  // Kiro exposes its agents as modes; switching one is free and reversible.
  const modeIds = (session.result?.modes?.availableModes ?? [])
    .map((m) => m.id)
    .filter(Boolean);
  if (modeIds.length > 1) {
    section("4b. session/set_mode");
    const currentMode = session.result?.modes?.currentModeId;
    const target = modeIds.find((id) => id !== currentMode) ?? modeIds[0];
    console.log(`switching mode ${currentMode} -> ${target}`);
    const setMode = await call("session/set_mode", { sessionId, modeId: target });
    console.log(show(setMode.result ?? setMode.error));
    if (currentMode) {
      const back = await call("session/set_mode", { sessionId, modeId: currentMode });
      console.log(`restored: ${show(back.result ?? back.error)}`);
    }
  }

  section("5. slash command sent as prompt text");
  const advertised = [];
  for (const [method, params] of notifications) {
    if (!/commands/i.test(method)) continue;
    const list = params?.availableCommands ?? params?.commands ?? params;
    if (Array.isArray(list)) {
      for (const entry of list) {
        const name = entry?.name ?? entry?.command ?? entry;
        if (typeof name === "string") advertised.push(name.replace(/^\//, ""));
      }
    }
  }
  console.log(
    advertised.length ? `advertised commands: ${advertised.join(", ")}` : "(no commands advertised)",
  );
  if (skipCommand) {
    console.log("skipped (--no-command)");
  } else {
    // Read-only commands only. Never auto-run anything that mutates a session.
    const safe = ["usage", "context", "help", "model", "agent", "tools"];
    const pick =
      wantedCommand?.replace(/^\//, "") ??
      safe.find((name) => advertised.includes(name)) ??
      (advertised.length ? null : "help");
    if (!pick) {
      console.log("no read-only command to try; pass --command <name> to force one");
    } else {
      console.log(`sending prompt text "/${pick}"`);
      const out = await call("session/prompt", {
        sessionId,
        prompt: [{ type: "text", text: `/${pick}` }],
      });
      console.log(show(out.result ?? out.error));
      const after = [...notifications.keys()].join(", ");
      console.log(`notification methods seen so far: ${after}`);
      // The proof, where there is any: a command that changes a session setting
      // shows up in the newest metadata frame, not the first one.
      await sleep(1000);
      console.log(`newest metadata after the command:\n${show(latestMetadata)}`);
      const reloaded = await call("session/load", { sessionId, cwd, mcpServers: [] });
      const models = reloaded.result?.models;
      if (models) {
        console.log(`current model after the command: ${models.currentModelId}`);
      }
    }
  }

  section("6. session/load with the id we already hold");
  const load = await call("session/load", { sessionId, cwd, mcpServers: [] });
  console.log(show(load.result ?? load.error));
  console.log(
    methodMissing(load.error) ? "=> NOT IMPLEMENTED" : "=> method exists",
  );

  if (doSteer) {
    section("7. a second session/prompt while one is in flight");
    const first = call("session/prompt", {
      sessionId,
      prompt: [
        {
          type: "text",
          text: "Count slowly from 1 to 30, one number per line, with no tools.",
        },
      ],
    });
    await sleep(1500);
    const second = await call("session/prompt", {
      sessionId,
      prompt: [{ type: "text", text: "Actually, stop at 3." }],
    });
    console.log(`second prompt returned: ${show(second.result ?? second.error)}`);
    const firstOut = await first;
    console.log(`first prompt returned : ${show(firstOut.result ?? firstOut.error)}`);
  }

  section("done");
  child.kill();
  process.exit(0);
}

main().catch((err) => {
  console.log(`probe failed: ${err?.stack ?? err}`);
  child.kill();
  process.exit(1);
});
