import fs from "node:fs";
import path from "node:path";

const LOCALES = path.join(process.cwd(), "src", "i18n", "locales");
const WORK = path.join(process.cwd(), process.env.I18N_WORK ?? ".i18n-work2");

// Keys whose translations are demonstrably out of date: the locale still holds
// old English text, or was written before the English string was reworded
// (missing Flow / TinyFish / Hugging Face mentions, changed behaviour).
// A JSON array of keys can be supplied instead: node … <keys.json>
const KEY_FILE = process.argv[2];
const KEYS = KEY_FILE
  ? JSON.parse(fs.readFileSync(KEY_FILE, "utf8"))
  : [
      "sectionSubtitles.history",
      "settings.general.pushToTalk.description",
      "settings.general.pushToTalk.hold",
      "settings.general.pushToTalk.tap",
      "settings.models.customModel.subtitle",
      "settings.models.customModel.searchPlaceholder",
      "settings.postProcessing.tone.description",
      "settings.postProcessing.api.baseUrl.description",
      "settings.history.storage.description",
      "settings.history.empty",
      "settings.history.emptyRecordings",
      "settings.debug.historyLimit.description",
      "settings.assistant.brain.catalogTitle",
      "settings.assistant.brain.catalogDescription",
      "settings.assistant.brain.huggingFaceTitle",
      "settings.assistant.brain.huggingFaceDescription",
      "settings.assistant.webSearch.providerDescription",
      "onboarding.speechToText.title",
      "settings.advanced.customWords.description",
    ];
const flat = (o, p = "", out = {}) => {
  for (const [k, v] of Object.entries(o)) {
    const key = p ? `${p}.${k}` : k;
    if (v && typeof v === "object" && !Array.isArray(v)) flat(v, key, out);
    else if (typeof v === "string") out[key] = v;
  }
  return out;
};

const en = flat(
  JSON.parse(
    fs.readFileSync(path.join(LOCALES, "en", "translation.json"), "utf8"),
  ),
);
for (const k of KEYS) if (!(k in en)) throw new Error(`unknown key ${k}`);

const langs = fs
  .readdirSync(LOCALES, { withFileTypes: true })
  .filter((d) => d.isDirectory() && d.name !== "en")
  .map((d) => d.name)
  .sort();

fs.rmSync(WORK, { recursive: true, force: true });
for (const lang of langs) {
  const dir = path.join(WORK, lang);
  fs.mkdirSync(dir, { recursive: true });
  const localeFlat = flat(
    JSON.parse(
      fs.readFileSync(path.join(LOCALES, lang, "translation.json"), "utf8"),
    ),
  );
  const source = {};
  const outdated = {};
  for (const k of KEYS) {
    source[k] = en[k];
    outdated[k] = localeFlat[k];
  }
  fs.writeFileSync(
    path.join(dir, "chunk-1.source.json"),
    JSON.stringify(source, null, 2) + "\n",
    "utf8",
  );
  fs.writeFileSync(
    path.join(dir, "current-outdated.json"),
    JSON.stringify(outdated, null, 2) + "\n",
    "utf8",
  );
  fs.writeFileSync(
    path.join(dir, "manifest.json"),
    JSON.stringify({ lang, total: KEYS.length, chunks: 1 }, null, 2) + "\n",
    "utf8",
  );
}
console.log(`${KEYS.length} keys × ${langs.length} languages`);
