/**
 * i18n-plurals.mjs — the key layout each locale file is expected to have.
 *
 * English only needs `_one` and `_other`, but i18next picks the plural suffix
 * with `Intl.PluralRules` for the *active* language: Russian asks for `_few`
 * and `_many`, Arabic for all six forms, Japanese only for `_other`. A locale
 * that copies English's two forms therefore falls back to English for every
 * count its own language inflects differently. So a plural group in English
 * (`X_one` + `X_other`) expands to exactly the categories the target language
 * uses, each seeded from the English form it corresponds to.
 *
 * Shared by check-translations.ts, i18n-todo.mjs and i18n-merge.mjs.
 */

/** CLDR order, so every file lists the forms the same way. */
export const PLURAL_ORDER = ["zero", "one", "two", "few", "many", "other"];

export function flatten(obj, prefix = "", out = {}) {
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === "object" && !Array.isArray(v)) flatten(v, key, out);
    else out[key] = v;
  }
  return out;
}

/** Plural categories the language actually uses, in CLDR order. */
export function pluralCategories(lang) {
  const cats = new Intl.PluralRules(lang).resolvedOptions().pluralCategories;
  return PLURAL_ORDER.filter((c) => cats.includes(c));
}

/** Bases of English plural groups: both `X_one` and `X_other` exist. */
export function pluralBases(enFlat) {
  const bases = new Set();
  for (const key of Object.keys(enFlat)) {
    const m = key.match(/^(.*)_other$/);
    if (m && `${m[1]}_one` in enFlat) bases.add(m[1]);
  }
  return bases;
}

/** The English form a locale plural variant is translated from. */
function sourceFormFor(base, cat) {
  return cat === "one" ? `${base}_one` : `${base}_other`;
}

/**
 * Ordered map of every key the locale file should contain → the English key
 * it is translated from.
 */
export function expectedKeys(lang, enFlat) {
  const bases = pluralBases(enFlat);
  const cats = pluralCategories(lang);
  const out = new Map();
  for (const key of Object.keys(enFlat)) {
    const m = key.match(/^(.*)_(one|other)$/);
    if (m && bases.has(m[1])) {
      const base = m[1];
      if (out.has(`${base}_${cats[0]}`)) continue; // group already emitted
      for (const cat of cats)
        out.set(`${base}_${cat}`, sourceFormFor(base, cat));
      continue;
    }
    out.set(key, key);
  }
  return out;
}

/** True when the placeholder sets are compatible for this (locale) key. */
export function placeholdersMatch(localeKey, sourceText, text) {
  const ph = (s) => [...new Set(s.match(/\{\{[^}]+\}\}/g) ?? [])].sort();
  const want = ph(sourceText);
  const got = ph(text);
  const cat = localeKey.match(/_(zero|one|two|few|many|other)$/)?.[1];
  if (cat) {
    // i18next always supplies {{count}} to a plural lookup, so a form may use
    // it even where English does not: Russian `_one` also covers 21, 31, …,
    // so "Snoozed once" there has to become "{{count}} раз". Conversely a
    // zero/one/two form is often written without the number ("no images").
    const rest = (list) => list.filter((p) => p !== "{{count}}");
    if (rest(want).join() !== rest(got).join()) return false;
    const needsCount = !["zero", "one", "two"].includes(cat);
    return !(
      needsCount &&
      want.includes("{{count}}") &&
      !got.includes("{{count}}")
    );
  }
  return want.join() === got.join();
}

/**
 * Nested locale object in the English file's key order, with each plural
 * group expanded to the language's categories. Keys absent from `flat` are
 * left out (i18next falls back to English for them).
 */
export function rebuild(lang, template, flat, prefix = "") {
  const cats = pluralCategories(lang);
  const out = {};
  const entries = Object.entries(template);
  const siblings = new Set(entries.map(([k]) => k));
  for (const [k, v] of entries) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (v && typeof v === "object" && !Array.isArray(v)) {
      out[k] = rebuild(lang, v, flat, key);
      continue;
    }
    const m = k.match(/^(.*)_(one|other)$/);
    if (m && siblings.has(`${m[1]}_one`) && siblings.has(`${m[1]}_other`)) {
      const base = m[1];
      if (cats.some((c) => `${base}_${c}` in out)) continue;
      for (const cat of cats) {
        const variant = prefix ? `${prefix}.${base}_${cat}` : `${base}_${cat}`;
        if (variant in flat) out[`${base}_${cat}`] = flat[variant];
      }
      continue;
    }
    if (key in flat) out[k] = flat[key];
  }
  return out;
}
