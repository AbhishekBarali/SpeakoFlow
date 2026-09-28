import { describe, expect, test } from "bun:test";
import { generateKeyPairSync, verify } from "node:crypto";
import { handle, type Env } from "../src/index";
import {
  MAX_MESSAGE_CHARS,
  defuseMentions,
  issueBody,
  issueTitle,
  parseFeedback,
  type Feedback,
} from "../src/feedback";
import { createAppJwt } from "../src/github";

const NOW = new Date("2026-09-28T10:00:00.000Z");

const valid = {
  kind: "bug",
  message: "The overlay disappears when I switch monitors.",
  email: "user@example.com",
  system: {
    app_version: "1.5.0",
    os: "Windows 10.0.26100",
    arch: "x86_64",
    install: "nsis",
  },
};

function request(
  body: unknown,
  init: { headers?: Record<string, string>; method?: string } = {},
) {
  return new Request("https://feedback.speakoflow.com/v1/feedback", {
    method: init.method ?? "POST",
    headers: {
      "Content-Type": "application/json",
      "User-Agent": "SpeakoFlow/1.5.0",
      "CF-Connecting-IP": "203.0.113.7",
      ...init.headers,
    },
    body:
      init.method === "GET"
        ? undefined
        : typeof body === "string"
          ? body
          : JSON.stringify(body),
  });
}

interface Call {
  url: string;
  init?: RequestInit;
}

function github(responses: Array<[number, unknown]>) {
  const calls: Call[] = [];
  const fetch = async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    const [status, body] = responses.shift() ?? [500, {}];
    return new Response(JSON.stringify(body), { status });
  };
  return { calls, fetch };
}

const PAT_ENV: Env = {
  FEEDBACK_REPO: "owner/feedback",
  GITHUB_TOKEN: "pat-123",
};

describe("parseFeedback", () => {
  test("accepts what the app sends", () => {
    const result = parseFeedback(valid);
    expect(result.ok).toBe(true);
  });

  test("an email and system block are optional", () => {
    const result = parseFeedback({ kind: "idea", message: "dark mode please" });
    expect(result).toEqual({
      ok: true,
      feedback: {
        kind: "idea",
        message: "dark mode please",
        email: null,
        system: null,
      },
    });
  });

  test.each([
    ["an unknown kind", { ...valid, kind: "rant" }, "kind"],
    ["a two-letter message", { ...valid, message: "  hi " }, "too short"],
    [
      "an over-long message",
      { ...valid, message: "x".repeat(MAX_MESSAGE_CHARS + 1) },
      "too long",
    ],
    ["a malformed email", { ...valid, email: "nope" }, "email"],
    ["a non-object system block", { ...valid, system: "windows" }, "system"],
    ["an array", [], "object"],
  ])("rejects %s", (_name, body, reason) => {
    const result = parseFeedback(body);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).toContain(reason);
  });

  test("control characters cannot break the issue table", () => {
    const result = parseFeedback({
      ...valid,
      system: { ...valid.system, os: "Win\ndows|x" },
    });
    expect(result.ok && result.feedback.system?.os).toBe("Win dows|x");
  });
});

describe("issue formatting", () => {
  const feedback: Feedback = {
    kind: "bug",
    message:
      "  \n@octocat it crashed when I pressed Ctrl+Space twice\nsecond line",
    email: null,
    system: null,
  };

  test("mentions cannot ping anyone", () => {
    expect(defuseMentions("hey @octocat and me@example.com")).toBe(
      "hey @\u200boctocat and me@\u200bexample.com",
    );
  });

  test("the title is the first line, prefixed and bounded", () => {
    expect(issueTitle(feedback)).toBe(
      "[Bug] @\u200boctocat it crashed when I pressed Ctrl+Space twice",
    );
    const long = issueTitle({ ...feedback, message: "word ".repeat(40) });
    expect(Array.from(long.replace("[Bug] ", "")).length).toBeLessThanOrEqual(
      80,
    );
    expect(long.endsWith("…")).toBe(true);
  });

  test("the body quotes the message and says what was not shared", () => {
    const body = issueBody(feedback, NOW);
    expect(body).toContain("> @\u200boctocat it crashed");
    expect(body).toContain("> second line");
    expect(body).toContain("| Reply to | _No email left_ |");
    expect(body).toContain("| System | _Not shared_ |");
    expect(body).toContain("| Received | 2026-09-28T10:00:00Z |");
  });

  test("system details land in the table, escaped", () => {
    const body = issueBody(
      {
        ...feedback,
        email: "a@b.co",
        system: {
          app_version: "1.5.0",
          os: "Ubuntu | 24.04",
          arch: "x86_64",
          install: "deb",
        },
      },
      NOW,
    );
    expect(body).toContain("| Reply to | a@\u200bb.co |");
    expect(body).toContain("| System | Ubuntu \\| 24.04 (x86_64) |");
    expect(body).toContain("| Install | deb |");
  });
});

describe("handle", () => {
  test("files an issue and reveals nothing about it", async () => {
    const gh = github([[201, { number: 42 }]]);
    const response = await handle(request(valid), PAT_ENV, {
      fetch: gh.fetch,
      now: () => NOW,
    });
    expect(response.status).toBe(201);
    expect(await response.json()).toEqual({ ok: true });

    expect(gh.calls).toHaveLength(1);
    const [call] = gh.calls;
    expect(call.url).toBe("https://api.github.com/repos/owner/feedback/issues");
    const headers = call.init?.headers as Record<string, string>;
    expect(headers.Authorization).toBe("Bearer pat-123");
    expect(headers["User-Agent"]).toBeTruthy();
    const sent = JSON.parse(call.init?.body as string);
    expect(sent.title).toBe(
      "[Bug] The overlay disappears when I switch monitors.",
    );
    expect(sent.labels).toEqual(["bug"]);
    expect(sent.body).toContain("| Version | 1.5.0 |");
  });

  test.each([
    ["a browser (no app User-Agent)", { "User-Agent": "Mozilla/5.0" }, 403],
    [
      "a form post",
      { "Content-Type": "application/x-www-form-urlencoded" },
      415,
    ],
    ["an oversized declaration", { "Content-Length": String(64 * 1024) }, 413],
  ])("refuses %s", async (_name, headers, status) => {
    const gh = github([]);
    const response = await handle(request(valid, { headers }), PAT_ENV, {
      fetch: gh.fetch,
      now: () => NOW,
    });
    expect(response.status).toBe(status);
    expect(gh.calls).toHaveLength(0);
  });

  test("refuses a CORS preflight, so web pages cannot post here", async () => {
    const response = await handle(
      request(null, { method: "OPTIONS" }),
      PAT_ENV,
      {
        fetch: github([]).fetch,
        now: () => NOW,
      },
    );
    expect(response.status).toBe(405);
    expect(response.headers.get("Access-Control-Allow-Origin")).toBeNull();
  });

  test("measures the real body, not the declared length", async () => {
    const response = await handle(
      request(JSON.stringify({ ...valid, message: "x".repeat(40 * 1024) }), {
        headers: { "Content-Length": "10" },
      }),
      PAT_ENV,
      { fetch: github([]).fetch, now: () => NOW },
    );
    expect(response.status).toBe(413);
  });

  test("bad JSON and bad fields are 4xx without touching GitHub", async () => {
    const gh = github([]);
    const deps = { fetch: gh.fetch, now: () => NOW };
    expect((await handle(request("{nope"), PAT_ENV, deps)).status).toBe(400);
    expect(
      (await handle(request({ ...valid, kind: "x" }), PAT_ENV, deps)).status,
    ).toBe(422);
    expect(gh.calls).toHaveLength(0);
  });

  test("rate limits per client before reading the body", async () => {
    const seen: string[] = [];
    const env: Env = {
      ...PAT_ENV,
      PER_CLIENT_LIMIT: {
        limit: async ({ key }) => {
          seen.push(key);
          return { success: false };
        },
      },
    };
    const gh = github([]);
    const response = await handle(request(valid), env, {
      fetch: gh.fetch,
      now: () => NOW,
    });
    expect(response.status).toBe(429);
    expect(seen).toEqual(["203.0.113.7"]);
    expect(gh.calls).toHaveLength(0);
  });

  test("a GitHub failure is a 5xx the app can retry, with no detail leaked", async () => {
    const gh = github([[401, { message: "Bad credentials" }]]);
    const response = await handle(request(valid), PAT_ENV, {
      fetch: gh.fetch,
      now: () => NOW,
    });
    expect(response.status).toBe(502);
    expect(JSON.stringify(await response.json())).not.toContain("credentials");
  });

  test("missing credentials fail closed", async () => {
    const response = await handle(
      request(valid),
      { FEEDBACK_REPO: "owner/feedback" },
      {
        fetch: github([]).fetch,
        now: () => NOW,
      },
    );
    expect(response.status).toBe(502);
  });

  test("health check and unknown paths", async () => {
    const deps = { fetch: github([]).fetch, now: () => NOW };
    const health = await handle(new Request("https://x/health"), PAT_ENV, deps);
    expect(health.status).toBe(200);
    const missing = await handle(new Request("https://x/admin"), PAT_ENV, deps);
    expect(missing.status).toBe(404);
  });
});

describe("GitHub App authentication", () => {
  // GitHub issues App keys as PKCS#1, which WebCrypto cannot import directly.
  const { privateKey, publicKey } = generateKeyPairSync("rsa", {
    modulusLength: 2048,
    privateKeyEncoding: { type: "pkcs1", format: "pem" },
    publicKeyEncoding: { type: "spki", format: "pem" },
  });

  const decode = (part: string) =>
    JSON.parse(
      Buffer.from(
        part.replace(/-/g, "+").replace(/_/g, "/"),
        "base64",
      ).toString(),
    );

  test("signs a JWT GitHub will accept from a PKCS#1 key", async () => {
    const jwt = await createAppJwt("12345", privateKey, 1_790_000_000);
    const [header, payload, signature] = jwt.split(".");
    expect(decode(header)).toEqual({ alg: "RS256", typ: "JWT" });
    expect(decode(payload)).toEqual({
      iat: 1_789_999_940,
      exp: 1_790_000_540,
      iss: "12345",
    });
    const ok = verify(
      "sha256",
      Buffer.from(`${header}.${payload}`),
      publicKey,
      Buffer.from(signature.replace(/-/g, "+").replace(/_/g, "/"), "base64"),
    );
    expect(ok).toBe(true);
  });

  test("accepts a key pasted with literal \\n", async () => {
    const flattened = privateKey.replace(/\n/g, "\\n");
    await expect(
      createAppJwt("1", flattened, 1_790_000_000),
    ).resolves.toContain(".");
  });

  test("files the issue as the app: find installation, mint token, create", async () => {
    const gh = github([
      [200, { id: 777 }],
      [201, { token: "ghs_installation" }],
      [201, { number: 9 }],
    ]);
    const env: Env = {
      FEEDBACK_REPO: "owner/feedback",
      GITHUB_APP_ID: "12345",
      GITHUB_APP_PRIVATE_KEY: privateKey,
      // A stray PAT must not win over the App.
      GITHUB_TOKEN: "pat-should-not-be-used",
    };
    const response = await handle(request(valid), env, {
      fetch: gh.fetch,
      now: () => NOW,
    });
    expect(response.status).toBe(201);
    expect(gh.calls.map((c) => c.url)).toEqual([
      "https://api.github.com/repos/owner/feedback/installation",
      "https://api.github.com/app/installations/777/access_tokens",
      "https://api.github.com/repos/owner/feedback/issues",
    ]);
    const last = gh.calls[2].init?.headers as Record<string, string>;
    expect(last.Authorization).toBe("Bearer ghs_installation");
  });

  test("a configured installation id skips the lookup", async () => {
    const gh = github([
      [201, { token: "ghs_installation" }],
      [201, { number: 10 }],
    ]);
    const env: Env = {
      FEEDBACK_REPO: "owner/feedback",
      GITHUB_APP_ID: "12345",
      GITHUB_APP_PRIVATE_KEY: privateKey,
      GITHUB_APP_INSTALLATION_ID: "555",
    };
    await handle(request(valid), env, { fetch: gh.fetch, now: () => NOW });
    expect(gh.calls[0].url).toBe(
      "https://api.github.com/app/installations/555/access_tokens",
    );
  });
});
