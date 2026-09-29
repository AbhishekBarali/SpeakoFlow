/**
 * Filing the issue. Two ways to authenticate, because they differ in the one
 * thing the maintainer cares about — whether GitHub emails them:
 *
 *  - A GitHub App (GITHUB_APP_ID + GITHUB_APP_PRIVATE_KEY). Issues are opened
 *    by "<app>[bot]", so the repository owner is notified like for any other
 *    new issue in a repository they watch. Recommended.
 *  - A fine-grained personal access token (GITHUB_TOKEN). Simpler, but the
 *    issue is opened *as the owner*, and GitHub does not email you about your
 *    own activity unless "Include your own updates" is switched on in
 *    notification settings.
 *
 * The App path is used whenever its credentials are present.
 */

export interface GitHubEnv {
  FEEDBACK_REPO: string;
  GITHUB_TOKEN?: string;
  GITHUB_APP_ID?: string;
  GITHUB_APP_PRIVATE_KEY?: string;
  GITHUB_APP_INSTALLATION_ID?: string;
}

export type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

const API = "https://api.github.com";

function headers(token: string): Record<string, string> {
  return {
    Authorization: `Bearer ${token}`,
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
    // GitHub rejects API requests without a User-Agent.
    "User-Agent": "speakoflow-feedback-worker",
  };
}

// ── base64url / DER helpers (Workers have no Buffer) ─────────────────────

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(binary);
}

function base64Url(bytes: Uint8Array): string {
  return bytesToBase64(bytes)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

function base64ToBytes(base64: string): Uint8Array {
  const binary = atob(base64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

function derLength(length: number): Uint8Array {
  if (length < 0x80) return Uint8Array.of(length);
  const bytes: number[] = [];
  for (let n = length; n > 0; n >>= 8) bytes.unshift(n & 0xff);
  return Uint8Array.of(0x80 | bytes.length, ...bytes);
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((sum, p) => sum + p.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

/**
 * GitHub hands out App keys as PKCS#1 ("BEGIN RSA PRIVATE KEY"); WebCrypto
 * only imports PKCS#8. The two differ by a fixed wrapper, so wrap it here
 * rather than asking the maintainer to run openssl.
 */
export function pkcs1ToPkcs8(pkcs1: Uint8Array): Uint8Array {
  const version = Uint8Array.of(0x02, 0x01, 0x00);
  // AlgorithmIdentifier { rsaEncryption, NULL }
  const algorithm = Uint8Array.of(
    0x30,
    0x0d,
    0x06,
    0x09,
    0x2a,
    0x86,
    0x48,
    0x86,
    0xf7,
    0x0d,
    0x01,
    0x01,
    0x01,
    0x05,
    0x00,
  );
  const octets = concat(Uint8Array.of(0x04), derLength(pkcs1.length), pkcs1);
  const body = concat(version, algorithm, octets);
  return concat(Uint8Array.of(0x30), derLength(body.length), body);
}

export function pemToPkcs8(pem: string): Uint8Array {
  // Secrets pasted through a single-line field often arrive with literal "\n".
  const text = pem.replace(/\\n/g, "\n").trim();
  const isPkcs1 = text.includes("BEGIN RSA PRIVATE KEY");
  const base64 = text
    .replace(/-----BEGIN [A-Z ]+-----/, "")
    .replace(/-----END [A-Z ]+-----/, "")
    .replace(/\s+/g, "");
  const der = base64ToBytes(base64);
  return isPkcs1 ? pkcs1ToPkcs8(der) : der;
}

/** An App JWT: RS256, issued a minute in the past for clock skew, 9 minutes long. */
export async function createAppJwt(
  appId: string,
  privateKeyPem: string,
  nowSeconds: number,
): Promise<string> {
  const encoder = new TextEncoder();
  const header = base64Url(
    encoder.encode(JSON.stringify({ alg: "RS256", typ: "JWT" })),
  );
  const payload = base64Url(
    encoder.encode(
      JSON.stringify({
        iat: nowSeconds - 60,
        exp: nowSeconds + 540,
        iss: appId,
      }),
    ),
  );
  const key = await crypto.subtle.importKey(
    "pkcs8",
    pemToPkcs8(privateKeyPem),
    { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const signingInput = `${header}.${payload}`;
  const signature = await crypto.subtle.sign(
    "RSASSA-PKCS1-v1_5",
    key,
    encoder.encode(signingInput),
  );
  return `${signingInput}.${base64Url(new Uint8Array(signature))}`;
}

export class GitHubError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

async function expectOk(response: Response, what: string): Promise<Response> {
  if (response.ok) return response;
  const detail = (await response.text().catch(() => "")).slice(0, 300);
  throw new GitHubError(
    `${what}: HTTP ${response.status} ${detail}`,
    response.status,
  );
}

async function appInstallationToken(
  env: GitHubEnv,
  fetchImpl: Fetch,
  nowSeconds: number,
): Promise<string> {
  const jwt = await createAppJwt(
    env.GITHUB_APP_ID!,
    env.GITHUB_APP_PRIVATE_KEY!,
    nowSeconds,
  );
  let installationId = env.GITHUB_APP_INSTALLATION_ID?.trim();
  if (!installationId) {
    const found = await expectOk(
      await fetchImpl(`${API}/repos/${env.FEEDBACK_REPO}/installation`, {
        headers: headers(jwt),
      }),
      "finding the app installation",
    );
    installationId = String(((await found.json()) as { id: number }).id);
  }
  const minted = await expectOk(
    await fetchImpl(
      `${API}/app/installations/${installationId}/access_tokens`,
      {
        method: "POST",
        headers: headers(jwt),
      },
    ),
    "minting an installation token",
  );
  return ((await minted.json()) as { token: string }).token;
}

export async function resolveToken(
  env: GitHubEnv,
  fetchImpl: Fetch,
  nowSeconds: number,
): Promise<string> {
  if (env.GITHUB_APP_ID && env.GITHUB_APP_PRIVATE_KEY) {
    return appInstallationToken(env, fetchImpl, nowSeconds);
  }
  if (env.GITHUB_TOKEN) return env.GITHUB_TOKEN;
  throw new GitHubError("no GitHub credentials configured", 500);
}

export interface NewIssue {
  title: string;
  body: string;
  labels: string[];
}

export async function createIssue(
  env: GitHubEnv,
  token: string,
  issue: NewIssue,
  fetchImpl: Fetch,
): Promise<number> {
  const response = await expectOk(
    await fetchImpl(`${API}/repos/${env.FEEDBACK_REPO}/issues`, {
      method: "POST",
      headers: { ...headers(token), "Content-Type": "application/json" },
      body: JSON.stringify(issue),
    }),
    "creating the issue",
  );
  return ((await response.json()) as { number: number }).number;
}

/**
 * Commit one file to the feedback repository's default branch and return a
 * URL an issue can embed.
 *
 * The URL is the file's `blob` page with `?raw=true`, not the API's
 * `download_url`: in a private repository the latter carries a short-lived
 * token and stops rendering within the hour, while the blob URL redirects
 * through the viewer's own GitHub session every time it is loaded — so the
 * image shows for anyone who can see the repository and for nobody else.
 *
 * Needs Contents: Read and write on the App or token, on top of Issues.
 */
export async function uploadFile(
  env: GitHubEnv,
  token: string,
  path: string,
  base64: string,
  message: string,
  fetchImpl: Fetch,
): Promise<string> {
  const encodedPath = path.split("/").map(encodeURIComponent).join("/");
  const response = await expectOk(
    await fetchImpl(
      `${API}/repos/${env.FEEDBACK_REPO}/contents/${encodedPath}`,
      {
        method: "PUT",
        headers: { ...headers(token), "Content-Type": "application/json" },
        body: JSON.stringify({ message, content: base64 }),
      },
    ),
    "uploading an attachment",
  );
  const result = (await response.json()) as {
    content?: { html_url?: string };
  };
  const page =
    result.content?.html_url ??
    `https://github.com/${env.FEEDBACK_REPO}/blob/HEAD/${encodedPath}`;
  return `${page}?raw=true`;
}
