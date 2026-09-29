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
 *  - Size and shape limits identical to the app's. Screenshots must be real
 *    PNG/JPEG/WebP bytes, at most three, at most 1 MB each.
 *  - Rate limits per client address and in total, so a loop — or a person —
 *    cannot exhaust the GitHub API quota or bury real reports. Screenshots
 *    have their own global budget, since they are what grows the repository.
 * The GitHub credential lives only here, as a Worker secret.
 */
import {
  MAX_BODY_BYTES,
  attachmentPath,
  issueBody,
  issueLabels,
  issueTitle,
  parseFeedback,
  type Feedback,
  type StoredImages,
} from "./feedback";
import {
  GitHubError,
  createIssue,
  resolveToken,
  uploadFile,
  type Fetch,
  type GitHubEnv,
} from "./github";

interface RateLimiter {
  limit(options: { key: string }): Promise<{ success: boolean }>;
}

export interface Env extends GitHubEnv {
  PER_CLIENT_LIMIT?: RateLimiter;
  GLOBAL_LIMIT?: RateLimiter;
  /** Reports carrying screenshots, all clients together. Bounds repo growth. */
  ATTACHMENT_LIMIT?: RateLimiter;
}

export interface Deps {
  fetch: Fetch;
  now: () => Date;
  /** Distinguishes two reports' files; injectable for tests. */
  randomId?: () => string;
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
  const feedback = result.feedback;
  try {
    const token = await resolveToken(
      env,
      deps.fetch,
      Math.floor(now.getTime() / 1000),
    );
    const images = await storeAttachments(env, token, feedback, now, deps);
    const number = await createIssue(
      env,
      token,
      {
        title: issueTitle(feedback),
        body: issueBody(feedback, now, images),
        labels: issueLabels(feedback),
      },
      deps.fetch,
    );
    console.log(
      `feedback #${number} (${feedback.kind}, ${images.urls.length} image(s)${images.dropped ? `, ${images.dropped} dropped` : ""})`,
    );
    // The issue lives in a private repository; nothing about it goes back —
    // except that a screenshot didn't make it, which the person should know.
    return json(
      201,
      images.dropped > 0
        ? { ok: true, attachments_dropped: images.dropped }
        : { ok: true },
    );
  } catch (error) {
    const status = error instanceof GitHubError ? error.status : 0;
    console.error(`feedback failed: ${(error as Error).message}`);
    return json(status === 403 || status === 429 ? 503 : 502, {
      error: "could not file feedback",
    });
  }
}

const randomId = (): string =>
  Array.from(crypto.getRandomValues(new Uint8Array(3)), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");

/**
 * Commit each screenshot, one after another (parallel writes to one branch
 * race and GitHub answers 409). A screenshot is supporting evidence, never the
 * report itself: any failure here — a token without Contents permission, the
 * attachment budget spent, GitHub hiccuping — files the issue anyway and says
 * how many images are missing, rather than losing the text too.
 */
async function storeAttachments(
  env: Env,
  token: string,
  feedback: Feedback,
  now: Date,
  deps: Deps,
): Promise<StoredImages> {
  const total = feedback.attachments.length;
  if (total === 0) return { urls: [], dropped: 0 };
  if (
    env.ATTACHMENT_LIMIT &&
    !(await env.ATTACHMENT_LIMIT.limit({ key: "all" })).success
  ) {
    console.warn(`attachment budget spent; dropping ${total}`);
    return { urls: [], dropped: total };
  }
  const id = (deps.randomId ?? randomId)();
  const urls: string[] = [];
  for (const [index, attachment] of feedback.attachments.entries()) {
    try {
      urls.push(
        await uploadFile(
          env,
          token,
          attachmentPath(now, id, index, attachment.mediaType),
          attachment.data,
          `feedback: screenshot ${index + 1} of ${total} (${feedback.kind})`,
          deps.fetch,
        ),
      );
    } catch (error) {
      // The rest would fail the same way (usually a missing permission).
      console.error(`attachment upload failed: ${(error as Error).message}`);
      break;
    }
  }
  return { urls, dropped: total - urls.length };
}

export default {
  fetch(request: Request, env: Env): Promise<Response> {
    return handle(request, env, {
      fetch: (input, init) => fetch(input, init),
      now: () => new Date(),
    });
  },
};
