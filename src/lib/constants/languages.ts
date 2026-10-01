export interface Language {
  value: string;
  label: string;
}

export const LANGUAGES: Language[] = [
  { value: "auto", label: "Auto Detect" },
  { value: "en", label: "English" },
  { value: "zh-Hans", label: "Simplified Chinese" },
  { value: "zh-Hant", label: "Traditional Chinese" },
  { value: "yue", label: "Cantonese" },
  { value: "de", label: "German" },
  { value: "es", label: "Spanish" },
  { value: "ru", label: "Russian" },
  { value: "ko", label: "Korean" },
  { value: "fr", label: "French" },
  { value: "ja", label: "Japanese" },
  { value: "pt", label: "Portuguese" },
  { value: "tr", label: "Turkish" },
  { value: "pl", label: "Polish" },
  { value: "ca", label: "Catalan" },
  { value: "nl", label: "Dutch" },
  { value: "ar", label: "Arabic" },
  { value: "sv", label: "Swedish" },
  { value: "it", label: "Italian" },
  { value: "id", label: "Indonesian" },
  { value: "hi", label: "Hindi" },
  { value: "fi", label: "Finnish" },
  { value: "vi", label: "Vietnamese" },
  { value: "he", label: "Hebrew" },
  { value: "uk", label: "Ukrainian" },
  { value: "el", label: "Greek" },
  { value: "ms", label: "Malay" },
  { value: "cs", label: "Czech" },
  { value: "ro", label: "Romanian" },
  { value: "da", label: "Danish" },
  { value: "hu", label: "Hungarian" },
  { value: "ta", label: "Tamil" },
  { value: "no", label: "Norwegian" },
  { value: "th", label: "Thai" },
  { value: "ur", label: "Urdu" },
  { value: "hr", label: "Croatian" },
  { value: "bg", label: "Bulgarian" },
  { value: "lt", label: "Lithuanian" },
  { value: "la", label: "Latin" },
  { value: "mi", label: "Maori" },
  { value: "ml", label: "Malayalam" },
  { value: "cy", label: "Welsh" },
  { value: "sk", label: "Slovak" },
  { value: "te", label: "Telugu" },
  { value: "fa", label: "Persian" },
  { value: "lv", label: "Latvian" },
  { value: "bn", label: "Bengali" },
  { value: "sr", label: "Serbian" },
  { value: "az", label: "Azerbaijani" },
  { value: "sl", label: "Slovenian" },
  { value: "kn", label: "Kannada" },
  { value: "et", label: "Estonian" },
  { value: "mk", label: "Macedonian" },
  { value: "br", label: "Breton" },
  { value: "eu", label: "Basque" },
  { value: "is", label: "Icelandic" },
  { value: "hy", label: "Armenian" },
  { value: "ne", label: "Nepali" },
  { value: "mn", label: "Mongolian" },
  { value: "bs", label: "Bosnian" },
  { value: "kk", label: "Kazakh" },
  { value: "sq", label: "Albanian" },
  { value: "sw", label: "Swahili" },
  { value: "gl", label: "Galician" },
  { value: "mr", label: "Marathi" },
  { value: "pa", label: "Punjabi" },
  { value: "si", label: "Sinhala" },
  { value: "km", label: "Khmer" },
  { value: "sn", label: "Shona" },
  { value: "yo", label: "Yoruba" },
  { value: "so", label: "Somali" },
  { value: "af", label: "Afrikaans" },
  { value: "oc", label: "Occitan" },
  { value: "ka", label: "Georgian" },
  { value: "be", label: "Belarusian" },
  { value: "tg", label: "Tajik" },
  { value: "sd", label: "Sindhi" },
  { value: "gu", label: "Gujarati" },
  { value: "am", label: "Amharic" },
  { value: "yi", label: "Yiddish" },
  { value: "lo", label: "Lao" },
  { value: "uz", label: "Uzbek" },
  { value: "fo", label: "Faroese" },
  { value: "ht", label: "Haitian Creole" },
  { value: "ps", label: "Pashto" },
  { value: "tk", label: "Turkmen" },
  { value: "nn", label: "Nynorsk" },
  { value: "mt", label: "Maltese" },
  { value: "sa", label: "Sanskrit" },
  { value: "lb", label: "Luxembourgish" },
  { value: "my", label: "Myanmar" },
  { value: "bo", label: "Tibetan" },
  { value: "tl", label: "Tagalog" },
  { value: "mg", label: "Malagasy" },
  { value: "as", label: "Assamese" },
  { value: "tt", label: "Tatar" },
  { value: "haw", label: "Hawaiian" },
  { value: "ln", label: "Lingala" },
  { value: "ha", label: "Hausa" },
  { value: "ba", label: "Bashkir" },
  { value: "jw", label: "Javanese" },
  { value: "su", label: "Sundanese" },
];

/**
 * Whisper's codes that are not the BCP 47 tag `Intl.DisplayNames` expects.
 * Whisper still says `jw` for Javanese, which ISO 639 replaced with `jv`.
 */
const DISPLAY_CODE: Readonly<Record<string, string>> = { jw: "jv" };

const displayNamesCache = new Map<string, Intl.DisplayNames | null>();

function displayNamesFor(uiLanguage: string): Intl.DisplayNames | null {
  if (!displayNamesCache.has(uiLanguage)) {
    let display: Intl.DisplayNames | null = null;
    try {
      display = new Intl.DisplayNames([uiLanguage], {
        type: "language",
        fallback: "none",
      });
    } catch {
      display = null;
    }
    displayNamesCache.set(uiLanguage, display);
  }
  return displayNamesCache.get(uiLanguage) ?? null;
}

/**
 * A spoken-language name in the UI language. The labels above are English, so
 * every other locale used to show "German", "Japanese", … in the language
 * pickers. `Intl.DisplayNames` is in every web view the app ships on; a code it
 * does not know falls back to the English label. `auto` has no language name
 * and is resolved by the caller through its translation key.
 */
export function languageLabel(code: string, uiLanguage: string): string {
  const english = LANGUAGES.find((lang) => lang.value === code)?.label ?? code;
  if (code === "auto" || uiLanguage.toLowerCase().startsWith("en")) {
    return english;
  }
  try {
    const name = displayNamesFor(uiLanguage)?.of(DISPLAY_CODE[code] ?? code);
    if (!name) return english;
    // Many locales write language names in lowercase ("allemand"); a picker
    // entry starts a line, so it takes a capital where the script has one.
    return name.charAt(0).toLocaleUpperCase(uiLanguage) + name.slice(1);
  } catch {
    return english;
  }
}

/** Every language, labelled in the UI language (`auto` keeps its English label). */
export function localizedLanguages(uiLanguage: string): Language[] {
  return LANGUAGES.map((lang) => ({
    value: lang.value,
    label: languageLabel(lang.value, uiLanguage),
  }));
}

/**
 * Search match for a language picker: the localized name, and the English one
 * too, so typing "German" still finds "Deutsch".
 */
export function languageMatches(lang: Language, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  if (lang.label.toLowerCase().includes(q)) return true;
  const english = LANGUAGES.find((l) => l.value === lang.value)?.label ?? "";
  return english.toLowerCase().includes(q);
}
