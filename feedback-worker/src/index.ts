/**
 * SpeakoFlow feedback endpoint: `POST /v1/feedback` from the desktop app,
 * filed as an issue in a private GitHub repository.
 *
 * It is a public URL with no user accounts behind it, so the defences are
 * about keeping it boring rather than secret:
 *  - JSON only and no CORS at all. A browser cannot send a cross-origin JSON
 *    POST without a preflight, and this Worker answers no preflight, so a web
 *    page cannot use a visitor's browser to post here.
 *  - The desktop app's User-Agent. Trivially spoofed, but it keeps generic
 *    scanners and form-spam bots out of the issue tracker.
 *  - Size and shape limits identical to the app's.
 *  - Rate limits per client address and in total, so a loop — or a person —
 *    cannot exhaust the GitHub API quota or bury real reports.
 * The GitHub credential lives only here, as a Worker secret.
 */
import {
  MAX_BODY_BYTES,
  issueBody,
  issueLabels,
  issueTitle,
  parseFeedback,
} from "./feedback";
import { GitHubError, createIssue, type Fetch, type GitHubEnv } from "./github";

interface RateLimiter {
  limit(options: { key: string }): Promise<{ success: boolean }>;
}

export interface Env extends GitHubEnv {
  PER_CLIENT_LIMIT?: RateLimiter;
  GLOBAL_LIMIT?: RateLimiter;
}

export interface Deps {
  fetch: Fetch;
  now: () => Date;
}

function json(status: number, body: Record<string, unknown>): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
    },
  });
}

export async function handle(
  request: Request,
  env: Env,
  deps: Deps,
): Promise<Response> {
  const url = new URL(request.url);

  if (url.pathname === "/" || url.pathname === "/health") {
    return request.method === "GET" || request.method === "HEAD"
      ? new Response("ok\n", { headers: { "Content-Type": "text/plain" } })
      : json(405, { error: "method not allowed" });
  }
  if (url.pathname !== "/v1/feedback") return json(404, { error: "not found" });
  if (request.method !== "POST")
    return json(405, { error: "method not allowed" });

  const agent = request.headers.get("User-Agent") ?? "";
  if (!agent.startsWith("SpeakoFlow/"))
    return json(403, { error: "forbidden" });

  const type = request.headers.get("Content-Type") ?? "";
  if (!type.toLowerCase().startsWith("application/json")) {
    return json(415, { error: "expected application/json" });
  }

  const declared = Number(request.headers.get("Content-Length") ?? "0");
  if (declared > MAX_BODY_BYTES) return json(413, { error: "too large" });

  const client = request.headers.get("CF-Connecting-IP") ?? "unknown";
  if (
    env.PER_CLIENT_LIMIT &&
    !(await env.PER_CLIENT_LIMIT.limit({ key: client })).success
  ) {
    return json(429, { error: "slow down" });
  }
  if (
    env.GLOBAL_LIMIT &&
    !(await env.GLOBAL_LIMIT.limit({ key: "all" })).success
  ) {
    return json(429, { error: "slow down" });
  }

  // Content-Length can be absent or wrong; measure what actually arrived.
  const raw = await request.arrayBuffer();
  if (raw.byteLength > MAX_BODY_BYTES) return json(413, { error: "too large" });

  let parsed: unknown;
  try {
    parsed = JSON.parse(new TextDecoder().decode(raw));
  } catch {
    return json(400, { error: "invalid JSON" });
  }
  const result = parseFeedback(parsed);
  if (!result.ok) return json(422, { error: result.error });

  const now = deps.now();
  try {
    const number = await createIssue(
      env,
      {
        title: issueTitle(result.feedback),
        body: issueBody(result.feedback, now),
        labels: issueLabels(result.feedback),
      },
      deps.fetch,
      Math.floor(now.getTime() / 1000),
    );
    console.log(`feedback #${number} (${result.feedback.kind})`);
    // The issue lives in a private repository; nothing about it goes back.
    return json(201, { ok: true });
  } catch (error) {
    const status = error instanceof GitHubError ? error.status : 0;
    console.error(`feedback failed: ${(error as Error).message}`);
    return json(status === 403 || status === 429 ? 503 : 502, {
      error: "could not file feedback",
    });
  }
}

export default {
  fetch(request: Request, env: Env): Promise<Response> {
    return handle(request, env, {
      fetch: (input, init) => fetch(input, init),
      now: () => new Date(),
    });
  },
};
