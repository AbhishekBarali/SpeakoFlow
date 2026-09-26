/**
 * Turn the ids and file names that models arrive with into names a person
 * would say out loud.
 *
 * A provider hands back `google.gemma-4-26b-a4b`, `openai/gpt-oss-120b` or
 * `scribe_v2_realtime`; a model registered from disk arrives as
 * `FLOWQwen3.5 0.8B.Q8 0`. Those are exact and must still be what the app
 * sends — this only changes what is *shown*, and every caller keeps the raw id
 * one hover away. The rules are deliberately conservative: they strip
 * packaging (vendor namespaces, region prefixes, release dates, quantization
 * tags) and fix casing, but never drop a word that distinguishes one model
 * from another.
 */

/** Words whose casing is a brand, not a sentence-case word. */
const CASED: Record<string, string> = {
  openai: "OpenAI",
  gpt: "GPT",
  oss: "OSS",
  deepseek: "DeepSeek",
  gemma: "Gemma",
  gemini: "Gemini",
  qwen: "Qwen",
  qwq: "QwQ",
  llama: "Llama",
  mistral: "Mistral",
  mixtral: "Mixtral",
  codestral: "Codestral",
  claude: "Claude",
  grok: "Grok",
  kimi: "Kimi",
  glm: "GLM",
  nova: "Nova",
  whisper: "Whisper",
  scribe: "Scribe",
  eleven: "Eleven",
  mai: "MAI",
  tts: "TTS",
  stt: "STT",
  asr: "ASR",
  hd: "HD",
  vl: "VL",
  it: "IT",
  moe: "MoE",
  phi: "Phi",
  sonar: "Sonar",
  nemotron: "Nemotron",
  parakeet: "Parakeet",
  canary: "Canary",
  voxtral: "Voxtral",
  chirp: "Chirp",
  aura: "Aura",
  kokoro: "Kokoro",
  speakoflow: "SpeakoFlow",
};

/** Namespaces a hosted catalog puts before the model (`google.`, `us.`). */
const VENDOR_PREFIXES = new Set([
  "amazon",
  "anthropic",
  "cohere",
  "meta",
  "mistral",
  "ai21",
  "google",
  "deepseek",
  "qwen",
  "openai",
  "writer",
  "stability",
  "moonshot",
  "moonshotai",
]);
const REGION_PREFIX = /^(us|eu|apac|global)\./i;

const SIZE = /^\d+(\.\d+)?[bmkt]$/i; // 120b, 0.8b, 7m
const EXPERT_SIZE = /^\d+x\d+(\.\d+)?b$/i; // 8x7b
const ACTIVE_SIZE = /^[a-z]\d+(\.\d+)?b$/i; // a4b, e2b
const VERSION = /^v\d+(\.\d+)*$/i; // v2, v3.2
const DIGIT = /^\d$/;

const casedToken = (token: string): string => {
  const lower = token.toLowerCase();
  if (CASED[lower]) return CASED[lower];
  if (SIZE.test(token) || EXPERT_SIZE.test(token)) {
    return token.slice(0, -1) + token.slice(-1).toUpperCase();
  }
  if (ACTIVE_SIZE.test(token)) return token.toUpperCase();
  if (VERSION.test(token)) return `v${token.slice(1)}`;
  // "qwen3.5", "llama3": a brand glued to its version.
  const glued = /^([a-z]+)(\d[\w.]*)$/i.exec(token);
  if (glued && CASED[glued[1].toLowerCase()]) {
    return CASED[glued[1].toLowerCase()] + glued[2];
  }
  if (/^[a-z]/.test(token)) return token[0].toUpperCase() + token.slice(1);
  return token;
};

/**
 * `openai/gpt-oss-120b` → `GPT-OSS 120B`, `scribe_v2_realtime` →
 * `Scribe v2 Realtime`, `google.gemma-4-26b-a4b` → `Gemma 4 26B A4B`.
 * Names that are already readable come back unchanged.
 */
export const prettyModelName = (raw: string | null | undefined): string => {
  let id = (raw ?? "").trim();
  if (!id) return "";

  // A hosted namespace: keep only the model after the last slash.
  id = id.slice(id.lastIndexOf("/") + 1);
  // Routing and revision suffixes: `:free`, `:online`, Bedrock's `:0`.
  id = id.replace(/:[\w.-]+$/, "");
  id = id.replace(REGION_PREFIX, "");

  const dot = id.indexOf(".");
  if (dot > 0) {
    const vendor = id.slice(0, dot).toLowerCase();
    const rest = id.slice(dot + 1);
    if (VENDOR_PREFIXES.has(vendor)) {
      // `deepseek.v3.2` names the model by its vendor; `google.gemma-4`
      // repeats a vendor the model name already carries.
      id = /^(v?\d)/i.test(rest) ? `${vendor}-${rest}` : rest;
    }
  }

  // Release dates and the "latest" alias say nothing about the model.
  id = id
    .replace(/[-_@](\d{8}|\d{4}-\d{2}-\d{2})(?=$|[-_])/g, "")
    .replace(/[-_]latest$/i, "");

  const tokens = id.split(/[-_\s]+/).filter(Boolean);
  const words: string[] = [];
  for (let i = 0; i < tokens.length; i += 1) {
    const token = tokens[i];
    // `claude-3-5-sonnet`: two lone digits are one version number.
    if (DIGIT.test(token) && DIGIT.test(tokens[i + 1] ?? "")) {
      words.push(`${token}.${tokens[i + 1]}`);
      i += 1;
      continue;
    }
    words.push(casedToken(token));
  }

  // GPT is written joined to what follows it: GPT-4o, GPT-OSS.
  const joined: string[] = [];
  for (let i = 0; i < words.length; i += 1) {
    if (words[i] === "GPT" && words[i + 1]) {
      joined.push(`GPT-${words[i + 1]}`);
      i += 1;
    } else {
      joined.push(words[i]);
    }
  }
  return joined.join(" ");
};

/** A GGUF quantization tag, however it was written: Q8_0, q4 k m, IQ3_XS, BF16. */
const QUANT_BODY = String.raw`(?:I?Q\d(?:[_ ][A-Z0-9]{1,3}){0,2}|BF16|F16|F32)`;
const QUANT_IN_PARENS = new RegExp(String.raw`\s*\((${QUANT_BODY})\)\s*$`, "i");
const QUANT_SUFFIX = new RegExp(String.raw`[.\-_ ](${QUANT_BODY})$`, "i");
const QUANT_IN_FILE = new RegExp(String.raw`[.\-_](${QUANT_BODY})\.gguf$`, "i");

const normalizeQuant = (quant: string): string =>
  quant.trim().replace(/\s+/g, "_").toUpperCase();

export interface LocalModelName {
  /** What to call the model. */
  name: string;
  /** Its quantization, shown as a quiet tag rather than inside the name. */
  quant: string | null;
}

/**
 * A downloaded or imported model's name with the packaging split off.
 *
 * `FLOWQwen3.5 0.8B.Q8 0` → `FLOWQwen3.5 0.8B` + `Q8_0`, and the Hugging Face
 * importer's `Qwen Qwen3.5 0.8B (Q4_K_M)` → `Qwen3.5 0.8B` + `Q4_K_M` (the
 * repository owner repeated in front of a name that already starts with it).
 */
export const splitLocalModelName = (
  name: string,
  filename?: string | null,
): LocalModelName => {
  let base = name.trim().replace(/\s*\(vision\)\s*$/i, "");
  let quant: string | null = null;

  const inParens = QUANT_IN_PARENS.exec(base);
  if (inParens) {
    quant = normalizeQuant(inParens[1]);
    base = base.slice(0, inParens.index);
  } else {
    const suffix = QUANT_SUFFIX.exec(base);
    if (suffix) {
      quant = normalizeQuant(suffix[1]);
      base = base.slice(0, suffix.index);
    }
  }
  if (!quant && filename) {
    const fromFile = QUANT_IN_FILE.exec(filename.trim());
    if (fromFile) quant = normalizeQuant(fromFile[1]);
  }

  const words = base.trim().split(/\s+/);
  if (
    words.length > 1 &&
    words[0].length >= 3 &&
    words[1].toLowerCase().startsWith(words[0].toLowerCase())
  ) {
    words.shift();
  }
  return { name: words.join(" ").trim() || name.trim(), quant };
};
