/**
 * i18n-merge.mjs — merge translated chunk files from .i18n-work back into
 * src/i18n/locales/<lang>/translation.json.
 *
 * A chunk pair is `.i18n-work/<lang>/chunk-N.source.json` (English source) and
 * `.i18n-work/<lang>/chunk-N.<lang>.json` (translation, same keys/order).
 *
 * The written file follows the English key order, with plural groups expanded
 * to the language's own categories (see i18n-plurals.mjs). Keys that have no
 * translation and are not translatable (brand names, URLs, sample values) are
 * copied from English so every locale carries the complete key set; keys that
 * no longer exist in English are dropped.
 *
 * Usage:
 *   node scripts/i18n-merge.mjs --check          # validate only, write nothing
 *   node scripts/i18n-merge.mjs --apply          # validate then write locales
 *   node scripts/i18n-merge.mjs --apply it de    # limit to given languages
 */
import fs from "node:fs";
import path from "node:path";
import {
  expectedKeys,
  flatten,
  placeholdersMatch,
  rebuild,
} from "./i18n-plurals.mjs";

const ROOT = process.cwd();
const LOCALES = path.join(ROOT, "src", "i18n", "locales");
const WORK = path.join(ROOT, process.env.I18N_WORK ?? ".i18n-work");

const args = process.argv.slice(2);
const apply = args.includes("--apply");
const onlyLangs = args.filter((a) => !a.startsWith("--"));

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
  const expected = expectedKeys(lang, en);

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
      if (typeof v !== "string" || !v.trim()) {
        if (k in translated) {
          problems.push(`chunk-${n}: empty translation for ${k}`);
          hardFail = true;
        }
        continue;
      }
      if (!placeholdersMatch(k, source[k], v)) {
        problems.push(`chunk-${n}: placeholder mismatch on ${k}`);
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

  // Complete the key set: anything still absent keeps its English value.
  let filled = 0;
  for (const [key, sourceKey] of expected) {
    if (!(key in localeFlat)) {
      localeFlat[key] = en[sourceKey];
      filled++;
    }
  }

  if (apply && !problems.length) {
    const rebuilt = rebuild(lang, enRaw, localeFlat);
    fs.writeFileSync(
      localePath,
      JSON.stringify(rebuilt, null, 2) + "\n",
      "utf8",
    );
  }

  lines.push(
    `${lang}: todo=${manifest.total} applied=${applied} english-filled=${filled}${
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
