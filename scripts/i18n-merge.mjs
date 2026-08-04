/**
 * i18n-merge.mjs — merge translated chunk files from .i18n-work back into
 * src/i18n/locales/<lang>/translation.json.
 *
 * A chunk pair is `.i18n-work/<lang>/chunk-N.source.json` (English source) and
 * `.i18n-work/<lang>/chunk-N.<lang>.json` (translation, same keys/order).
 *
 * Usage:
 *   node scripts/i18n-merge.mjs --check          # validate only, write nothing
 *   node scripts/i18n-merge.mjs --apply          # validate then write locales
 *   node scripts/i18n-merge.mjs --apply it de    # limit to given languages
 */
import fs from "node:fs";
import path from "node:path";

const ROOT = process.cwd();
const LOCALES = path.join(ROOT, "src", "i18n", "locales");
const WORK = path.join(ROOT, process.env.I18N_WORK ?? ".i18n-work");

const args = process.argv.slice(2);
const apply = args.includes("--apply");
const onlyLangs = args.filter((a) => !a.startsWith("--"));

function flatten(obj, prefix = "", out = {}) {
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === "object" && !Array.isArray(v)) flatten(v, key, out);
    else out[key] = v;
  }
  return out;
}

function placeholders(s) {
  return (s.match(/\{\{[^}]+\}\}/g) || []).sort();
}

/** Rebuild a nested object following the exact key order of `template`. */
function rebuild(template, flat, prefix = "") {
  const out = {};
  for (const [k, v] of Object.entries(template)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === "object" && !Array.isArray(v)) {
      out[k] = rebuild(v, flat, key);
    } else if (key in flat) {
      out[k] = flat[key];
    }
  }
  return out;
}

const enRaw = JSON.parse(
  fs.readFileSync(path.join(LOCALES, "en", "translation.json"), "utf8"),
);
const en = flatten(enRaw);

const langs = fs
  .readdirSync(WORK, { withFileTypes: true })
  .filter((d) => d.isDirectory())
  .map((d) => d.name)
  .filter((l) => onlyLangs.length === 0 || onlyLangs.includes(l))
  .sort();

let hardFail = false;
const lines = [];

for (const lang of langs) {
  const dir = path.join(WORK, lang);
  const manifest = JSON.parse(
    fs.readFileSync(path.join(dir, "manifest.json"), "utf8"),
  );
  const localePath = path.join(LOCALES, lang, "translation.json");
  const localeFlat = flatten(JSON.parse(fs.readFileSync(localePath, "utf8")));

  const problems = [];
  let applied = 0;
  let missingChunks = 0;

  for (let n = 1; n <= manifest.chunks; n++) {
    const srcPath = path.join(dir, `chunk-${n}.source.json`);
    const outPath = path.join(dir, `chunk-${n}.${lang}.json`);
    const source = JSON.parse(fs.readFileSync(srcPath, "utf8"));
    if (!fs.existsSync(outPath)) {
      missingChunks++;
      continue;
    }
    let translated;
    try {
      translated = JSON.parse(fs.readFileSync(outPath, "utf8"));
    } catch (e) {
      problems.push(`chunk-${n}: invalid JSON (${e.message})`);
      hardFail = true;
      continue;
    }

    const srcKeys = Object.keys(source);
    const outKeys = Object.keys(translated);
    const missingKeys = srcKeys.filter((k) => !(k in translated));
    const extraKeys = outKeys.filter((k) => !(k in source));
    if (missingKeys.length)
      problems.push(
        `chunk-${n}: ${missingKeys.length} keys not translated (${missingKeys
          .slice(0, 4)
          .join(", ")}${missingKeys.length > 4 ? ", …" : ""})`,
      );
    if (extraKeys.length)
      problems.push(
        `chunk-${n}: ${extraKeys.length} unknown keys (${extraKeys
          .slice(0, 4)
          .join(", ")})`,
      );
    if (missingKeys.length || extraKeys.length) hardFail = true;

    for (const k of srcKeys) {
      const v = translated[k];
      if (typeof v !== "string" || !v.trim()) continue;
      const sp = placeholders(source[k]).join("|");
      const tp = placeholders(v).join("|");
      if (sp !== tp) {
        problems.push(
          `chunk-${n}: placeholder mismatch on ${k} (${sp} → ${tp})`,
        );
        hardFail = true;
        continue;
      }
      localeFlat[k] = v;
      applied++;
    }
  }

  if (missingChunks)
    problems.push(
      `${missingChunks}/${manifest.chunks} chunk files not produced yet`,
    );

  if (apply && !problems.length) {
    const rebuilt = rebuild(enRaw, localeFlat);
    fs.writeFileSync(
      localePath,
      JSON.stringify(rebuilt, null, 2) + "\n",
      "utf8",
    );
  }

  lines.push(
    `${lang}: todo=${manifest.total} applied=${applied}${
      problems.length ? ` PROBLEMS:\n    - ${problems.join("\n    - ")}` : " OK"
    }`,
  );
}

console.log(lines.join("\n"));
console.log(
  apply
    ? hardFail
      ? "\nnot written for languages with problems"
      : "\nwritten"
    : "\ncheck only",
);
process.exit(hardFail ? 1 : 0);
