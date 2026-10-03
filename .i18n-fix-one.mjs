// Temporary: Russian/Ukrainian `_one` also covers 21, 31, …, so it needs {{count}}.
import fs from "node:fs";
const fixes = {
  ru: {
    "feedback.screenshotsDropped_one":
      "Не удалось сохранить {{count}} снимок экрана, но ваше сообщение доставлено.",
    "meetings.summary.skipped_one":
      "{{count}} часть транскрипции не удалось обработать, поэтому в заметках есть пробел.",
    "reminders.snoozedTimes_one": "Отложено {{count}} раз",
  },
  uk: {
    "feedback.screenshotsDropped_one":
      "{{count}} знімок екрана не вдалося зберегти, але ваше повідомлення надійшло.",
    "meetings.summary.skipped_one":
      "{{count}} частину транскрипції не вдалося підсумувати, тож у цих нотатках є прогалина.",
    "reminders.snoozedTimes_one": "Відкладено {{count}}×",
  },
};
for (const [lang, map] of Object.entries(fixes)) {
  const file = `src/i18n/locales/${lang}/translation.json`;
  const j = JSON.parse(fs.readFileSync(file, "utf8"));
  for (const [path, value] of Object.entries(map)) {
    const parts = path.split(".");
    let o = j;
    for (const p of parts.slice(0, -1)) o = o[p];
    if (!(parts.at(-1) in o)) throw new Error(`${lang}:${path} missing`);
    o[parts.at(-1)] = value;
  }
  fs.writeFileSync(file, JSON.stringify(j, null, 2) + "\n", "utf8");
}
console.log("ok");
