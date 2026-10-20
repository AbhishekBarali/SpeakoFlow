import {
  AUTO_LANGUAGE,
  baseLanguage,
  effectiveLanguage,
  modelSupportsLanguage,
} from "@/lib/languageCodes";

/**
 * Whether the Chinese script choice can change anything for this language and
 * model, so the row is only shown where it does.
 *
 * Mirrors `script_rule` in `src-tauri/src/managers/transcription/chinese_script.rs`:
 * the setting reaches Chinese from a plain Chinese or Cantonese run, or from a
 * run whose language the model detects, on a model that can write Chinese.
 * `zh-Hans` and `zh-Hant` already name a script, so the row is not needed for
 * them. Without a language list (a cloud provider, a custom Whisper file) any
 * language is possible.
 */
export function showsChineseScript(
  intent: string,
  supportedLanguages?: ReadonlyArray<string>,
  supportsLanguageDetection = true,
): boolean {
  if (intent === "zh-Hans" || intent === "zh-Hant") return false;
  const languages = supportedLanguages ?? [];
  const effective =
    languages.length > 0
      ? effectiveLanguage(intent, languages, supportsLanguageDetection)
      : intent;
  const base = baseLanguage(effective);
  if (base === "zh" || base === "yue") return true;
  if (effective !== AUTO_LANGUAGE) return false;
  return (
    languages.length === 0 ||
    modelSupportsLanguage(languages, "zh") ||
    modelSupportsLanguage(languages, "yue")
  );
}
