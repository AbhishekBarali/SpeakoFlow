// Temporary: add new English source keys, preserving existing order.
import fs from "node:fs";
const file = "src/i18n/locales/en/translation.json";
const en = JSON.parse(fs.readFileSync(file, "utf8"));
const add = {
  "common.play": "Play",
  "common.pause": "Pause",
  "settings.sound.soundTheme.themes.marimba": "Marimba",
  "settings.sound.soundTheme.themes.pop": "Pop",
  "settings.sound.soundTheme.themes.click": "Click",
  "settings.sound.soundTheme.themes.custom": "Custom",
  "settings.debug.logLevel.levels.error": "Error",
  "settings.debug.logLevel.levels.warn": "Warning",
  "settings.debug.logLevel.levels.info": "Info",
  "settings.debug.logLevel.levels.debug": "Debug",
  "settings.debug.logLevel.levels.trace": "Trace",
  "settings.postProcessing.api.baseUrl.managed":
    "Base URL is managed by the selected provider.",
  "settings.assistant.tts.kokoroVoices.usFemale": "{{name}} (US female)",
  "settings.assistant.tts.kokoroVoices.usFemaleSoft":
    "{{name}} (US female, soft)",
  "settings.assistant.tts.kokoroVoices.usMale": "{{name}} (US male)",
  "settings.assistant.tts.kokoroVoices.ukFemale": "{{name}} (UK female)",
  "settings.assistant.tts.kokoroVoices.ukMale": "{{name}} (UK male)",
  "settings.assistant.tts.dtypes.fp32": "fp32 (best quality, WebGPU)",
  "settings.assistant.tts.dtypes.fp16": "fp16 (half precision)",
  "settings.assistant.tts.dtypes.q8": "q8 (8-bit, fast on CPU)",
  "settings.assistant.tts.dtypes.q4": "q4 (4-bit, fastest)",
  "settings.assistant.tts.dtypes.q4f16": "q4f16 (4-bit mixed)",
};
for (const [path, value] of Object.entries(add)) {
  const parts = path.split(".");
  let o = en;
  for (const p of parts.slice(0, -1)) {
    if (o[p] === undefined) o[p] = {};
    if (typeof o[p] !== "object") throw new Error(`${path}: ${p} is a string`);
    o = o[p];
  }
  const last = parts.at(-1);
  if (last in o && o[last] !== value) throw new Error(`${path} exists`);
  o[last] = value;
}
fs.writeFileSync(file, JSON.stringify(en, null, 2) + "\n", "utf8");
console.log("added", Object.keys(add).length);
