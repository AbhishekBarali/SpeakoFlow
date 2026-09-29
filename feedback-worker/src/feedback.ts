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

export type AttachmentType = "image/png" | "image/jpeg" | "image/webp";

/** A screenshot, already compressed by the app. `data` is plain base64. */
export interface Attachment {
  mediaType: AttachmentType;
  data: string;
  bytes: number;
}

export interface Feedback {
  kind: FeedbackKind;
  message: string;
  email: string | null;
  system: SystemInfo | null;
  attachments: Attachment[];
}

/** Same limits as the app (`src-tauri/src/feedback.rs`). */
export const MIN_MESSAGE_CHARS = 3;
export const MAX_MESSAGE_CHARS = 5000;
export const MAX_EMAIL_CHARS = 254;
const MAX_SYSTEM_FIELD_CHARS = 120;
/**
 * Screenshots are committed to the feedback repository, so they are kept
 * small on purpose: the app downscales and re-encodes each one to a few
 * hundred KB, and anything over a megabyte is refused rather than stored.
 */
export const MAX_ATTACHMENTS = 3;
export const MAX_ATTACHMENT_BYTES = 1024 * 1024;
/** Text alone stays well under 32 KB; base64 adds a third to each image. */
const TEXT_BODY_BYTES = 32 * 1024;
export const MAX_BODY_BYTES =
  TEXT_BODY_BYTES + Math.ceil((MAX_ATTACHMENT_BYTES * 4) / 3) * MAX_ATTACHMENTS;

export const ATTACHMENT_EXTENSIONS: Record<AttachmentType, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/webp": "webp",
};

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

const BASE64 = /^[A-Za-z0-9+/]*={0,2}$/;

/** Decoded size of a well-formed base64 string, without decoding it. */
export function base64DecodedBytes(data: string): number {
  const padding = data.endsWith("==") ? 2 : data.endsWith("=") ? 1 : 0;
  return (data.length / 4) * 3 - padding;
}

/**
 * What the leading bytes say the file is. The declared type is not trusted:
 * this endpoint writes into a repository, so only the three image formats the
 * app produces are stored, and only when the bytes agree.
 */
export function sniffImage(head: Uint8Array): AttachmentType | null {
  const starts = (...bytes: number[]) =>
    bytes.every((byte, index) => head[index] === byte);
  if (starts(0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a))
    return "image/png";
  if (starts(0xff, 0xd8, 0xff)) return "image/jpeg";
  if (
    starts(0x52, 0x49, 0x46, 0x46) &&
    head[8] === 0x57 &&
    head[9] === 0x45 &&
    head[10] === 0x42 &&
    head[11] === 0x50
  ) {
    return "image/webp";
  }
  return null;
}

type AttachmentResult =
  | { ok: true; attachments: Attachment[] }
  | { ok: false; error: string };

export function parseAttachments(input: unknown): AttachmentResult {
  if (input === undefined || input === null)
    return { ok: true, attachments: [] };
  if (!Array.isArray(input))
    return { ok: false, error: "attachments must be an array" };
  if (input.length > MAX_ATTACHMENTS) {
    return { ok: false, error: `at most ${MAX_ATTACHMENTS} attachments` };
  }
  const attachments: Attachment[] = [];
  for (const item of input) {
    if (typeof item !== "object" || item === null || Array.isArray(item)) {
      return { ok: false, error: "attachment must be an object" };
    }
    const raw = item as Record<string, unknown>;
    const declared = raw.media_type;
    if (typeof declared !== "string" || !(declared in ATTACHMENT_EXTENSIONS)) {
      return { ok: false, error: "attachment must be png, jpeg or webp" };
    }
    if (typeof raw.data !== "string") {
      return { ok: false, error: "attachment data is required" };
    }
    const data = raw.data.replace(/\s+/g, "");
    if (data.length === 0 || data.length % 4 !== 0 || !BASE64.test(data)) {
      return { ok: false, error: "attachment data must be base64" };
    }
    const bytes = base64DecodedBytes(data);
    if (bytes > MAX_ATTACHMENT_BYTES) {
      return { ok: false, error: "attachment is too large" };
    }
    const head = Uint8Array.from(atob(data.slice(0, 16)), (c) =>
      c.charCodeAt(0),
    );
    if (sniffImage(head) !== declared) {
      return { ok: false, error: "attachment is not the image it claims" };
    }
    attachments.push({ mediaType: declared as AttachmentType, data, bytes });
  }
  return { ok: true, attachments };
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

  const attached = parseAttachments(body.attachments);
  if (!attached.ok) return attached;

  return {
    ok: true,
    feedback: {
      kind: body.kind as FeedbackKind,
      message,
      email,
      system,
      attachments: attached.attachments,
    },
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

/**
 * Where the screenshots of one report are stored: grouped by month so the
 * folder stays browsable, and prefixed with the receive time so files sort in
 * the order the reports arrived. `id` keeps two reports in the same second
 * apart.
 */
export function attachmentPath(
  receivedAt: Date,
  id: string,
  index: number,
  mediaType: AttachmentType,
): string {
  const iso = receivedAt.toISOString();
  const month = iso.slice(0, 7);
  const stamp = iso.slice(0, 19).replace(/[-:]/g, "").replace("T", "-");
  return `attachments/${month}/${stamp}-${id}-${index + 1}.${ATTACHMENT_EXTENSIONS[mediaType]}`;
}

/** What happened to the screenshots, for the issue body. */
export interface StoredImages {
  urls: string[];
  /** Attached in the app but not stored (a failed upload, or the cap). */
  dropped: number;
}

function imagesSection(images: StoredImages): string {
  const parts = images.urls.map(
    (url, index) => `![Screenshot ${index + 1}](${url})`,
  );
  if (images.dropped > 0) {
    parts.push(
      `_${images.dropped} screenshot${images.dropped === 1 ? " was" : "s were"} attached but couldn't be stored._`,
    );
  }
  return parts.length ? `${parts.join("\n\n")}\n\n` : "";
}

export function issueBody(
  feedback: Feedback,
  receivedAt: Date,
  images: StoredImages = { urls: [], dropped: 0 },
): string {
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

  return `${quoted}\n\n${imagesSection(images)}${table}\n\n<sub>Sent from SpeakoFlow's in-app feedback form.</sub>\n`;
}

export function issueLabels(feedback: Feedback): string[] {
  return [LABELS[feedback.kind]];
}
