// Temporary: flag translated chunk values that are still English prose.
import fs from "node:fs";
const W = ".i18n-work";
for (const l of fs
  .readdirSync(W, { withFileTypes: true })
  .filter((d) => d.isDirectory())
  .map((d) => d.name)) {
  const m = JSON.parse(fs.readFileSync(`${W}/${l}/manifest.json`, "utf8"));
  const same = [];
  for (let n = 1; n <= m.chunks; n++) {
    const s = JSON.parse(
      fs.readFileSync(`${W}/${l}/chunk-${n}.source.json`, "utf8"),
    );
    const t = JSON.parse(
      fs.readFileSync(`${W}/${l}/chunk-${n}.${l}.json`, "utf8"),
    );
    for (const k in s)
      if (s[k] === t[k] && /[a-z]{3,} [a-z]{3,} [a-z]{3,}/i.test(s[k]))
        same.push(`${k} = ${s[k].slice(0, 70)}`);
  }
  console.log(l, same.length);
  same.slice(0, 8).forEach((x) => console.log("   ", x));
}
