/**
 * i18n-stale.mjs — find translations that were written against an older version
 * of the English string.
 *
 * For every key it compares the commit that last changed the value in
 * `en/translation.json` with the commit that last changed the value in the
 * locale file. If English moved more recently, the translation is stale.
 *
 * Usage: node scripts/i18n-stale.mjs [--json]
 */
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";

const LOCALES = "src/i18n/locales";
const flat = (o, p = "", out = {}) => {
  for (const [k, v] of Object.entries(o)) {
    const key = p ? `${p}.${k}` : k;
    if (v && typeof v === "object" && !Array.isArray(v)) flat(v, key, out);
    else if (typeof v === "string") out[key] = v;
  }
  return out;
};

const git = (args) =>
  execFileSync("git", args, { maxBuffer: 200e6, encoding: "buffer" });

/** Commits that touched a path, oldest first, as [sha, unixTime]. */
function commits(file) {
  const out = git(["log", "--reverse", "--format=%H %at", "--", file]).toString(
    "utf8",
  );
  return out
    .split("\n")
    .filter(Boolean)
    .map((line) => line.split(" "));
}

function fileAt(sha, file) {
  try {
    return JSON.parse(git(["show", `${sha}:${file}`]).toString("utf8"));
  } catch {
    return null;
  }
}

/** key -> unix time of the commit that last changed that key's value. */
function lastChangedPerKey(file) {
  const result = new Map();
  let previous = new Map();
  for (const [sha, time] of commits(file)) {
    const parsed = fileAt(sha, file);
    if (!parsed) continue;
    const current = new Map(Object.entries(flat(parsed)));
    for (const [key, value] of current) {
      if (previous.get(key) !== value) result.set(key, Number(time));
    }
    previous = current;
  }
  return result;
}

const enFile = `${LOCALES}/en/translation.json`;
const enChanged = lastChangedPerKey(enFile);
const en = flat(JSON.parse(fs.readFileSync(enFile, "utf8")));

const langs = fs
  .readdirSync(LOCALES, { withFileTypes: true })
  .filter((d) => d.isDirectory() && d.name !== "en")
  .map((d) => d.name)
  .sort();

const report = {};
const lines = [];
for (const lang of langs) {
  const file = `${LOCALES}/${lang}/translation.json`;
  const localeChanged = lastChangedPerKey(file);
  const current = flat(JSON.parse(fs.readFileSync(file, "utf8")));
  const committed = flat(fileAt("HEAD", file) ?? {});
  const stale = [];
  for (const key of Object.keys(en)) {
    if (!(key in current)) continue;
    // Values already updated in the working tree are current by definition.
    if (current[key] !== committed[key]) continue;
    const enTime = enChanged.get(key);
    const locTime = localeChanged.get(key);
    if (enTime === undefined || locTime === undefined) continue;
    if (enTime > locTime) stale.push(key);
  }
  report[lang] = stale;
  lines.push(`${lang}: ${stale.length} stale`);
  stale.forEach((key) =>
    lines.push(
      `    ${key}\n      en    : ${JSON.stringify(en[key])}\n      locale: ${JSON.stringify(current[key])}`,
    ),
  );
}

fs.writeFileSync(".i18n-stale.txt", lines.join("\n") + "\n", "utf8");
if (process.argv.includes("--json"))
  fs.writeFileSync(".i18n-stale.json", JSON.stringify(report, null, 2), "utf8");
console.log(langs.map((l) => `${l}=${report[l].length}`).join(" "));
