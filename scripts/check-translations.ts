import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { expectedKeys, flatten, placeholdersMatch } from "./i18n-plurals.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Configuration
const LOCALES_DIR = path.join(__dirname, "..", "src", "i18n", "locales");
const REFERENCE_LANG = "en";
// Keys that are allowed to stay identical to English (brand names, key caps,
// sample values, and words that are genuinely the same in that language).
const BASELINE_PATH = path.join(
  __dirname,
  "..",
  "src",
  "i18n",
  "untranslated-baseline.json",
);
const UPDATE_BASELINE = process.argv.includes("--update-baseline");

type TranslationData = Record<string, unknown>;
type FlatStrings = Record<string, string>;

interface ValidationResult {
  valid: boolean;
  missing: string[];
  extra: string[];
}

/** Per-language record of values that are allowed to stay in English. */
interface LocaleBaseline {
  sameAsEnglish: string[];
  englishFromOtherKey: string[];
}

function getLanguages(): string[] {
  const entries = fs.readdirSync(LOCALES_DIR, { withFileTypes: true });
  return entries
    .filter((entry) => entry.isDirectory() && entry.name !== REFERENCE_LANG)
    .map((entry) => entry.name)
    .sort();
}

const LANGUAGES = getLanguages();

// Colors for terminal output
const colors: Record<string, string> = {
  reset: "\x1b[0m",
  red: "\x1b[31m",
  green: "\x1b[32m",
  yellow: "\x1b[33m",
  blue: "\x1b[34m",
};

function colorize(text: string, color: string): string {
  return `${colors[color]}${text}${colors.reset}`;
}

function loadTranslationFile(lang: string): TranslationData | null {
  const filePath = path.join(LOCALES_DIR, lang, "translation.json");

  try {
    const content = fs.readFileSync(filePath, "utf8");
    return JSON.parse(content) as TranslationData;
  } catch (error) {
    console.error(colorize(`✗ Error loading ${lang}/translation.json:`, "red"));
    console.error(`  ${(error as Error).message}`);
    return null;
  }
}

function loadFlat(lang: string): FlatStrings | null {
  const data = loadTranslationFile(lang);
  return data ? (flatten(data) as FlatStrings) : null;
}

/**
 * Locale key → English key it is translated from. Plural groups expand to the
 * categories the language uses (Russian needs `_few`/`_many`, Japanese only
 * `_other`), so a locale is not expected to mirror English's `_one`/`_other`.
 */
function expectedFor(lang: string, en: FlatStrings): Map<string, string> {
  return expectedKeys(lang, en) as Map<string, string>;
}

function loadBaseline(): Record<string, LocaleBaseline> {
  try {
    return JSON.parse(fs.readFileSync(BASELINE_PATH, "utf8")) as Record<
      string,
      LocaleBaseline
    >;
  } catch {
    return {};
  }
}

/**
 * Two ways English copy survives in a locale file:
 *  - sameAsEnglish: the value was never translated;
 *  - englishFromOtherKey: the value is English that matches a *different*
 *    English key, i.e. the source was reworded and the locale kept the old
 *    wording. That is how "Recording" stayed in 19 locales after the English
 *    string became "Dictation".
 * Both are errors unless recorded in untranslated-baseline.json, which holds
 * the legitimate cases: brand names, key caps, sample values, and words that
 * are simply identical in that language.
 */
function checkUntranslated(en: FlatStrings): { hasErrors: boolean } {
  const englishValues = new Set(Object.values(en));
  const baseline = loadBaseline();
  const nextBaseline: Record<string, LocaleBaseline> = {};
  let hasErrors = false;

  console.log(
    colorize("\nUntranslated strings (English copy in a locale):", "blue"),
  );
  console.log("─".repeat(60));

  for (const lang of LANGUAGES) {
    const flat = loadFlat(lang);
    if (!flat) {
      hasErrors = true;
      continue;
    }
    const expected = expectedFor(lang, en);

    const sameAsEnglish: string[] = [];
    const englishFromOtherKey: string[] = [];
    for (const [key, sourceKey] of expected) {
      const value = flat[key];
      if (value === undefined) continue;
      // An empty English value (a description deliberately left blank) has
      // nothing to translate; the locale matching it is correct.
      if (!en[sourceKey]?.trim()) continue;
      if (value === en[sourceKey]) sameAsEnglish.push(key);
      else if (englishValues.has(value)) englishFromOtherKey.push(key);
    }
    nextBaseline[lang] = {
      sameAsEnglish: [...sameAsEnglish].sort(),
      englishFromOtherKey: [...englishFromOtherKey].sort(),
    };

    const allowedSame = new Set(baseline[lang]?.sameAsEnglish ?? []);
    const allowedStale = new Set(baseline[lang]?.englishFromOtherKey ?? []);
    const unexpected = [
      ...sameAsEnglish
        .filter((key) => !allowedSame.has(key))
        .map((key) => `${key} = ${JSON.stringify(flat[key])}`),
      ...englishFromOtherKey
        .filter((key) => !allowedStale.has(key))
        .map(
          (key) =>
            `${key} = ${JSON.stringify(flat[key])} — stale English, source now says ${JSON.stringify(en[expected.get(key)!])}`,
        ),
    ];

    const allowedCount = sameAsEnglish.length + englishFromOtherKey.length;
    if (unexpected.length === 0) {
      const fixed =
        (baseline[lang]?.sameAsEnglish ?? []).filter(
          (key) => !sameAsEnglish.includes(key),
        ).length +
        (baseline[lang]?.englishFromOtherKey ?? []).filter(
          (key) => !englishFromOtherKey.includes(key),
        ).length;
      const note = fixed
        ? ` (${fixed} baseline entr${fixed === 1 ? "y" : "ies"} now translated — run with --update-baseline)`
        : "";
      console.log(
        colorize(
          `✓ ${lang.toUpperCase()}: ${allowedCount} allowed, 0 new${note}`,
          "green",
        ),
      );
      continue;
    }

    hasErrors = true;
    console.log(
      colorize(
        `✗ ${lang.toUpperCase()}: ${unexpected.length} string(s) still in English`,
        "red",
      ),
    );
    unexpected.slice(0, 10).forEach((line) => console.log(`    - ${line}`));
    if (unexpected.length > 10) {
      console.log(
        colorize(`    ... and ${unexpected.length - 10} more`, "yellow"),
      );
    }
  }

  if (UPDATE_BASELINE) {
    fs.writeFileSync(
      BASELINE_PATH,
      JSON.stringify(nextBaseline, null, 2) + "\n",
      "utf8",
    );
    console.log(colorize(`\n↻ Baseline rewritten: ${BASELINE_PATH}`, "yellow"));
    return { hasErrors: false };
  }

  if (hasErrors) {
    console.log(
      colorize(
        "\nTranslate those keys, or if the English form is correct for the\n" +
          "language (brand name, key cap, sample value, identical word), run:\n" +
          "  bun scripts/check-translations.ts --update-baseline",
        "yellow",
      ),
    );
  }

  return { hasErrors };
}

/**
 * i18next substitutes {{placeholders}} at runtime, so a translation that drops
 * or renames one silently ships a broken string. A language's zero/one/two
 * plural forms may leave out {{count}} ("no images"); nothing else may.
 */
function checkPlaceholders(en: FlatStrings): { hasErrors: boolean } {
  let hasErrors = false;
  const findings: string[] = [];

  for (const lang of LANGUAGES) {
    const flat = loadFlat(lang);
    if (!flat) {
      hasErrors = true;
      continue;
    }
    for (const [key, sourceKey] of expectedFor(lang, en)) {
      if (!(key in flat)) continue;
      if (!placeholdersMatch(key, en[sourceKey], flat[key])) {
        hasErrors = true;
        const ph = (s: string) => (s.match(/\{\{[^}]+\}\}/g) ?? []).join(",");
        findings.push(
          `  ${lang}: ${key} expected [${ph(en[sourceKey])}] got [${ph(flat[key])}]`,
        );
      }
    }
  }

  console.log(colorize("\nPlaceholder check:", "blue"));
  console.log("─".repeat(60));
  if (findings.length === 0) {
    console.log(
      colorize("✓ All {{placeholders}} match the English source", "green"),
    );
  } else {
    console.log(
      colorize(`✗ ${findings.length} placeholder mismatch(es):`, "red"),
    );
    findings.slice(0, 20).forEach((line) => console.log(line));
    if (findings.length > 20) {
      console.log(colorize(`  ... and ${findings.length - 20} more`, "yellow"));
    }
  }

  return { hasErrors };
}

function validateTranslations(): void {
  console.log(colorize("\n🌍 Translation Consistency Check\n", "blue"));

  // Load reference file
  console.log(`Loading reference language: ${REFERENCE_LANG}`);
  const en = loadFlat(REFERENCE_LANG);

  if (!en) {
    console.error(
      colorize(`\n✗ Failed to load reference file (${REFERENCE_LANG})`, "red"),
    );
    process.exit(1);
  }

  console.log(`Reference has ${Object.keys(en).length} keys\n`);

  // Track validation results
  let hasErrors = false;
  const results: Record<string, ValidationResult> = {};

  // Validate each language
  for (const lang of LANGUAGES) {
    const flat = loadFlat(lang);

    if (!flat) {
      hasErrors = true;
      results[lang] = { valid: false, missing: [], extra: [] };
      continue;
    }

    const expected = expectedFor(lang, en);
    const missing = [...expected.keys()].filter((key) => !(key in flat));
    const extra = Object.keys(flat).filter((key) => !expected.has(key));

    results[lang] = {
      valid: missing.length === 0 && extra.length === 0,
      missing,
      extra,
    };

    if (missing.length > 0 || extra.length > 0) {
      hasErrors = true;
    }
  }

  // Print results
  console.log(colorize("Results:", "blue"));
  console.log("─".repeat(60));

  for (const lang of LANGUAGES) {
    const result = results[lang];

    if (result.valid) {
      console.log(
        colorize(`✓ ${lang.toUpperCase()}: All keys present`, "green"),
      );
    } else {
      console.log(colorize(`✗ ${lang.toUpperCase()}: Issues found`, "red"));

      if (result.missing.length > 0) {
        console.log(
          colorize(`  Missing ${result.missing.length} keys:`, "yellow"),
        );
        result.missing.slice(0, 10).forEach((key) => {
          console.log(`    - ${key}`);
        });
        if (result.missing.length > 10) {
          console.log(
            colorize(
              `    ... and ${result.missing.length - 10} more`,
              "yellow",
            ),
          );
        }
      }

      if (result.extra.length > 0) {
        console.log(
          colorize(
            `  Extra ${result.extra.length} keys (not in reference):`,
            "yellow",
          ),
        );
        result.extra.slice(0, 10).forEach((key) => {
          console.log(`    - ${key}`);
        });
        if (result.extra.length > 10) {
          console.log(
            colorize(`    ... and ${result.extra.length - 10} more`, "yellow"),
          );
        }
      }

      console.log("");
    }
  }

  console.log("─".repeat(60));

  const untranslated = checkUntranslated(en);
  if (untranslated.hasErrors) hasErrors = true;

  const placeholders = checkPlaceholders(en);
  if (placeholders.hasErrors) hasErrors = true;

  // Summary
  const validCount = Object.values(results).filter((r) => r.valid).length;
  const totalCount = LANGUAGES.length;

  if (hasErrors) {
    console.log(
      colorize(
        `\n✗ Validation failed: ${validCount}/${totalCount} languages have complete key sets`,
        "red",
      ),
    );
    process.exit(1);
  } else {
    console.log(
      colorize(
        `\n✓ All ${totalCount} languages have complete translations!`,
        "green",
      ),
    );
    process.exit(0);
  }
}

// Run validation
validateTranslations();
