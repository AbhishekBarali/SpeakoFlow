/**
 * i18n-todo.mjs — generate per-language work lists of strings that still need
 * translating (keys missing from the locale file, plus keys whose value is
 * still byte-identical to the English source).
 *
 * Usage:
 *   node scripts/i18n-todo.mjs            # write .i18n-work/<lang>/chunk-N.json
 *   node scripts/i18n-todo.mjs --stats    # only print per-language counts
 */
import fs from "node:fs";
import path from "node:path";

const ROOT = process.cwd();
const LOCALES = path.join(ROOT, "src", "i18n", "locales");
const WORK = path.join(ROOT, ".i18n-work");
const CHUNK_SIZE = 150;

/** Values that are brand names / identifiers and must stay as-is everywhere. */
const KEEP_AS_IS = new Set([
  "OpenAI",
  "Ollama",
  "LM Studio",
  "OpenRouter",
  "Anthropic",
  "Azure",
  "ElevenLabs",
  "Kokoro",
  "Gemini",
  "Groq",
  "Mistral",
  "DeepSeek",
  "Serper",
  "Brave",
  "Tavily",
  "Exa",
  "SerpAPI",
  "TinyFish",
  "SpeakoFlow",
  "Whisper",
  "Parakeet",
  "GitHub",
  "Hugging Face",
  "MIT",
]);

export function flatten(obj, prefix = "", out = {}) {
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === "object" && !Array.isArray(v)) flatten(v, key, out);
    else out[key] = v;
  }
  return out;
}

/** Heuristic: a value with no translatable words (pure brand/symbol/number). */
function isUntranslatable(value) {
  if (typeof value !== "string") return true;
  const trimmed = value.trim();
  if (!trimmed) return true;
  if (KEEP_AS_IS.has(trimmed)) return true;
  // URLs, API-key samples and other verbatim technical values.
  if (/^https?:\/\//i.test(trimmed)) return true;
  if (/^(sk|pk|gsk)-/i.test(trimmed)) return true;
  if (/^[a-z0-9._-]+\/[a-z0-9._/-]+$/i.test(trimmed)) return true;
  // Only placeholders, punctuation, digits, or units.
  const stripped = trimmed
    .replace(/\{\{[^}]+\}\}/g, "")
    .replace(/[^A-Za-z]/g, "");
  if (!stripped) return true;
  return false;
}

const en = flatten(
  JSON.parse(
    fs.readFileSync(path.join(LOCALES, "en", "translation.json"), "utf8"),
  ),
);
const enKeys = Object.keys(en);

const langs = fs
  .readdirSync(LOCALES, { withFileTypes: true })
  .filter((d) => d.isDirectory() && d.name !== "en")
  .map((d) => d.name)
  .sort();

const statsOnly = process.argv.includes("--stats");
if (!statsOnly && fs.existsSync(WORK)) {
  // Only clear per-language folders; keep anything else in the work dir.
  for (const d of fs.readdirSync(WORK, { withFileTypes: true })) {
    if (d.isDirectory())
      fs.rmSync(path.join(WORK, d.name), { recursive: true, force: true });
  }
}

const summary = [];
for (const lang of langs) {
  const flat = flatten(
    JSON.parse(
      fs.readFileSync(path.join(LOCALES, lang, "translation.json"), "utf8"),
    ),
  );
  const todo = enKeys.filter((k) => {
    if (isUntranslatable(en[k])) return false;
    if (!(k in flat)) return true; // missing entirely
    return flat[k] === en[k]; // still English
  });

  summary.push(`${lang}\t${todo.length}`);
  if (statsOnly) continue;

  const dir = path.join(WORK, lang);
  fs.mkdirSync(dir, { recursive: true });
  for (let i = 0; i < todo.length; i += CHUNK_SIZE) {
    const slice = todo.slice(i, i + CHUNK_SIZE);
    const payload = {};
    for (const k of slice) payload[k] = en[k];
    const n = Math.floor(i / CHUNK_SIZE) + 1;
    fs.writeFileSync(
      path.join(dir, `chunk-${n}.source.json`),
      JSON.stringify(payload, null, 2) + "\n",
      "utf8",
    );
  }
  const chunks = Math.ceil(todo.length / CHUNK_SIZE);
  fs.writeFileSync(
    path.join(dir, "manifest.json"),
    JSON.stringify({ lang, total: todo.length, chunks }, null, 2) + "\n",
    "utf8",
  );
}

console.log("lang\ttodo");
console.log(summary.join("\n"));
