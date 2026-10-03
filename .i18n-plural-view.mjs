// Temporary: print plural groups whose English singular lacks {{count}}.
import fs from "node:fs";
import { flatten } from "./scripts/i18n-plurals.mjs";
const langs = process.argv.slice(2);
const bases = [
  "feedback.screenshotsDropped",
  "meetings.summary.skipped",
  "reminders.snoozedTimes",
];
for (const l of langs) {
  const f = flatten(
    JSON.parse(
      fs.readFileSync(`src/i18n/locales/${l}/translation.json`, "utf8"),
    ),
  );
  for (const b of bases)
    for (const k in f) if (k.startsWith(b + "_")) console.log(l, k, "=", f[k]);
}
