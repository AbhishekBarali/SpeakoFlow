// Temporary: list values identical to English that are not in the baseline,
// skipping keys that are English in (almost) every language (brands, samples).
import fs from "node:fs";
import { expectedKeys, flatten } from "./scripts/i18n-plurals.mjs";
const en = flatten(
  JSON.parse(fs.readFileSync("src/i18n/locales/en/translation.json", "utf8")),
);
const base = JSON.parse(
  fs.readFileSync("src/i18n/untranslated-baseline.json", "utf8"),
);
const langs = fs.readdirSync("src/i18n/locales").filter((l) => l !== "en");
const hits = {};
for (const l of langs) {
  const f = flatten(
    JSON.parse(
      fs.readFileSync(`src/i18n/locales/${l}/translation.json`, "utf8"),
    ),
  );
  const allowed = new Set([
    ...(base[l]?.sameAsEnglish ?? []),
    ...(base[l]?.englishFromOtherKey ?? []),
  ]);
  for (const [k, s] of expectedKeys(l, en)) {
    if (f[k] === en[s] && !allowed.has(k)) (hits[k] ??= []).push(l);
  }
}
for (const [k, ls] of Object.entries(hits).sort(
  (a, b) => a[1].length - b[1].length,
)) {
  if (ls.length >= 17) continue;
  console.log(
    `${k} = ${JSON.stringify(en[k] ?? en[k.replace(/_(zero|one|two|few|many|other)$/, "_other")])}  [${ls.join(",")}]`,
  );
}
console.log(
  "\nuniversal:",
  Object.entries(hits)
    .filter(([, ls]) => ls.length >= 17)
    .map(([k]) => k)
    .join(" "),
);
