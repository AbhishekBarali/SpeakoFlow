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

/**
 * Whether the row is worth a line in Settings for this person, on top of
 * `showsChineseScript` (which mirrors the backend and decides whether the
 * setting can do anything at all).
 *
 * Chinese or Cantonese as the language always shows it. On Auto the setting
 * can reach Chinese, but for everyone who never speaks it, it was a row about
 * a script they will never see, on every cloud provider and every multilingual
 * model. So on Auto it shows only when the app itself is in Chinese, or when a
 * script has already been chosen, so a choice that is in effect is never
 * hidden from the person who made it.
 */
export function offersChineseScript(
  intent: string,
  supportedLanguages: ReadonlyArray<string> | undefined,
  supportsLanguageDetection: boolean,
  context: { uiLanguage: string; scriptChosen: boolean },
): boolean {
  if (
    !showsChineseScript(intent, supportedLanguages, supportsLanguageDetection)
  ) {
    return false;
  }
  const spoken = baseLanguage(intent);
  if (spoken === "zh" || spoken === "yue") return true;
  const ui = baseLanguage(context.uiLanguage);
  return context.scriptChosen || ui === "zh" || ui === "yue";
}
