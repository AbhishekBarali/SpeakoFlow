/**
 * Spoken-language codes: how the user's choice meets a model's list.
 *
 * The app's own codes (`selected_language`, the language picker) are
 * Whisper's plus `zh-Hans` / `zh-Hant`, because the Simplified/Traditional
 * conversion runs on the transcript afterwards. A model lists whatever its
 * authors wrote: `nb` or `no` for Norwegian, `fil` or `tl` for Filipino, `zh`
 * for Chinese, sometimes a full locale. Every comparison goes through one
 * canonical form, so "Chinese" finds every Chinese model and "Norwegian"
 * finds the ones that say `nb`.
 *
 * This mirrors `src-tauri/src/managers/language.rs` rule for rule, and the two
 * are tested against the same cases. Change one, change both.
 */

/** The value of `selected_language` that asks the model to detect it. */
export const AUTO_LANGUAGE = "auto";

/** The primary language subtag, lowercased: `en-US` → `en`, `zh-Hant` → `zh`. */
export function baseLanguage(code: string): string {
  return code.trim().split(/[-_]/)[0]?.toLowerCase() ?? "";
}

/**
 * The form two codes are compared in: the base language, with the aliases
 * model authors disagree on folded together. Bokmål (`nb`) is Norwegian
 * (`no`) while Nynorsk (`nn`) stays its own language; Filipino (`fil`) is
 * Tagalog (`tl`). Cantonese (`yue`) is not a kind of Chinese.
 */
export function canonicalLanguageCode(code: string): string {
  const base = baseLanguage(code);
  if (base === "nb") return "no";
  if (base === "fil") return "tl";
  return base;
}

/** Whether two codes name the same spoken language. */
export function languagesMatch(a: string, b: string): boolean {
  const canonical = canonicalLanguageCode(a);
  return canonical !== "" && canonical === canonicalLanguageCode(b);
}

/** Whether a model's language list includes `code`, by the canonical rule. */
export function modelSupportsLanguage(
  supported: ReadonlyArray<string>,
  code: string,
): boolean {
  return supported.some((entry) => languagesMatch(entry, code));
}

/**
 * A model's code written the way the app writes it. Chinese becomes
 * Simplified, the script nearly every Chinese model writes by default; the two
 * script variants the app already uses are kept.
 */
export function appLanguageCode(modelCode: string): string {
  const code = modelCode.trim();
  if (code === "zh-Hans" || code === "zh-Hant") return code;
  const canonical = canonicalLanguageCode(code);
  return canonical === "zh" ? "zh-Hans" : canonical;
}

/**
 * The distinct languages in a list, each once, in canonical form and in the
 * order first seen: `en-US` and `en-GB` are one, and so are `nb` and `no`.
 */
export function uniqueLanguages(codes: ReadonlyArray<string>): string[] {
  return [
    ...new Set(codes.map(canonicalLanguageCode).filter((code) => code !== "")),
  ];
}

/**
 * The language a model will actually use for the user's `intent`, in the
 * app's vocabulary.
 *
 * The intent itself when the model lists it (exactly, by base language, or by
 * alias); otherwise `auto` when the model detects the language; otherwise
 * English when it has it; otherwise the model's first language. A model with
 * no list is language-agnostic and takes the intent as it is.
 */
export function effectiveLanguage(
  intent: string,
  supported: ReadonlyArray<string>,
  supportsDetection: boolean,
): string {
  if (supported.length === 0) return intent;
  if (intent !== AUTO_LANGUAGE && modelSupportsLanguage(supported, intent)) {
    return intent;
  }
  if (supportsDetection) return AUTO_LANGUAGE;
  if (supported.some((code) => baseLanguage(code) === "en")) return "en";
  return appLanguageCode(supported[0]);
}
