/**
 * Validation and formatting for one piece of in-app feedback. Pure: no I/O,
 * so every rule here is tested without a network or a Worker runtime.
 */

export type FeedbackKind = "bug" | "idea" | "question";

export interface SystemInfo {
  app_version: string;
  os: string;
  arch: string;
  install: string;
}

export interface Feedback {
  kind: FeedbackKind;
  message: string;
  email: string | null;
  system: SystemInfo | null;
}

/** Same limits as the app (`src-tauri/src/feedback.rs`). */
export const MIN_MESSAGE_CHARS = 3;
export const MAX_MESSAGE_CHARS = 5000;
export const MAX_EMAIL_CHARS = 254;
const MAX_SYSTEM_FIELD_CHARS = 120;
/** Generous for 5,000 characters of any script plus JSON overhead. */
export const MAX_BODY_BYTES = 32 * 1024;

const KINDS: FeedbackKind[] = ["bug", "idea", "question"];

export type ParseResult =
  | { ok: true; feedback: Feedback }
  | { ok: false; error: string };

const charCount = (value: string) => Array.from(value).length;

export function looksLikeEmail(email: string): boolean {
  if (charCount(email) > MAX_EMAIL_CHARS || /\s/.test(email)) return false;
  const parts = email.split("@");
  if (parts.length !== 2) return false;
  const [local, domain] = parts;
  return (
    local.length > 0 &&
    domain.includes(".") &&
    !domain.startsWith(".") &&
    !domain.endsWith(".") &&
    !domain.includes("..")
  );
}

function cleanSystemField(value: unknown): string | null {
  if (typeof value !== "string") return null;
  // Control characters would break the Markdown table the issue shows.
  // eslint-disable-next-line no-control-regex
  const clean = value.replace(/[\u0000-\u001f\u007f]/g, " ").trim();
  return clean
    ? Array.from(clean).slice(0, MAX_SYSTEM_FIELD_CHARS).join("")
    : null;
}

export function parseFeedback(input: unknown): ParseResult {
  if (typeof input !== "object" || input === null || Array.isArray(input)) {
    return { ok: false, error: "body must be a JSON object" };
  }
  const body = input as Record<string, unknown>;

  if (!KINDS.includes(body.kind as FeedbackKind)) {
    return { ok: false, error: "kind must be bug, idea or question" };
  }
  if (typeof body.message !== "string") {
    return { ok: false, error: "message is required" };
  }
  const message = body.message.trim();
  const length = charCount(message);
  if (length < MIN_MESSAGE_CHARS)
    return { ok: false, error: "message is too short" };
  if (length > MAX_MESSAGE_CHARS)
    return { ok: false, error: "message is too long" };

  let email: string | null = null;
  if (body.email !== undefined && body.email !== null) {
    if (typeof body.email !== "string")
      return { ok: false, error: "email must be a string" };
    const trimmed = body.email.trim();
    if (trimmed) {
      if (!looksLikeEmail(trimmed))
        return { ok: false, error: "email is invalid" };
      email = trimmed;
    }
  }

  let system: SystemInfo | null = null;
  if (body.system !== undefined && body.system !== null) {
    if (typeof body.system !== "object" || Array.isArray(body.system)) {
      return { ok: false, error: "system must be an object" };
    }
    const raw = body.system as Record<string, unknown>;
    const fields = {
      app_version: cleanSystemField(raw.app_version),
      os: cleanSystemField(raw.os),
      arch: cleanSystemField(raw.arch),
      install: cleanSystemField(raw.install),
    };
    system = {
      app_version: fields.app_version ?? "unknown",
      os: fields.os ?? "unknown",
      arch: fields.arch ?? "unknown",
      install: fields.install ?? "unknown",
    };
  }

  return {
    ok: true,
    feedback: { kind: body.kind as FeedbackKind, message, email, system },
  };
}

// ── Issue formatting ─────────────────────────────────────────────────────

const LABELS: Record<FeedbackKind, string> = {
  bug: "bug",
  idea: "idea",
  question: "question",
};

const TITLE_PREFIX: Record<FeedbackKind, string> = {
  bug: "Bug",
  idea: "Idea",
  question: "Question",
};

const MAX_TITLE_CHARS = 80;

/**
 * Text from a stranger lands in a repository its owner reads. An "@name"
 * would notify that GitHub user, and in a table cell a pipe or newline breaks
 * the layout, so both are defused. Everything else is left as written:
 * GitHub sanitises HTML itself, and the repository is private.
 */
export function defuseMentions(text: string): string {
  // A zero-width space after the @ keeps it readable but unlinkable.
  return text.replace(/@(?=[A-Za-z0-9-])/g, "@\u200b");
}

function tableCell(text: string): string {
  return defuseMentions(text).replace(/\|/g, "\\|").replace(/\s+/g, " ");
}

export function issueTitle(feedback: Feedback): string {
  const firstLine =
    feedback.message
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find((line) => line.length > 0) ?? "";
  const oneLine = defuseMentions(firstLine.replace(/\s+/g, " "));
  const chars = Array.from(oneLine);
  const summary =
    chars.length > MAX_TITLE_CHARS
      ? `${chars
          .slice(0, MAX_TITLE_CHARS - 1)
          .join("")
          .trimEnd()}…`
      : oneLine;
  return `[${TITLE_PREFIX[feedback.kind]}] ${summary}`;
}

export function issueBody(feedback: Feedback, receivedAt: Date): string {
  const quoted = defuseMentions(feedback.message)
    .split(/\r?\n/)
    .map((line) => (line.length ? `> ${line}` : ">"))
    .join("\n");

  const rows: [string, string][] = [
    [
      "Reply to",
      feedback.email ? tableCell(feedback.email) : "_No email left_",
    ],
  ];
  if (feedback.system) {
    rows.push(
      ["Version", tableCell(feedback.system.app_version)],
      ["System", tableCell(`${feedback.system.os} (${feedback.system.arch})`)],
      ["Install", tableCell(feedback.system.install)],
    );
  } else {
    rows.push(["System", "_Not shared_"]);
  }
  rows.push(["Received", receivedAt.toISOString().replace(/\.\d{3}Z$/, "Z")]);

  const table = [
    "| | |",
    "|---|---|",
    ...rows.map(([key, value]) => `| ${key} | ${value} |`),
  ].join("\n");

  return `${quoted}\n\n${table}\n\n<sub>Sent from SpeakoFlow's in-app feedback form.</sub>\n`;
}

export function issueLabels(feedback: Feedback): string[] {
  return [LABELS[feedback.kind]];
}
