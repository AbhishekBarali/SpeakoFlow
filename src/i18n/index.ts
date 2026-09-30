import i18n, { type BackendModule } from "i18next";
import { initReactI18next } from "react-i18next";
import { locale } from "@tauri-apps/plugin-os";
import { LANGUAGE_METADATA } from "./languages";
import { commands } from "@/bindings";
import {
  getLanguageDirection,
  updateDocumentDirection,
  updateDocumentLanguage,
} from "@/lib/utils/rtl";
import en from "./locales/en/translation.json";

type TranslationTable = Record<string, unknown>;

// Every locale file becomes its own lazily loaded chunk. English, the source and
// the fallback for any missing key, is bundled; everything else is fetched only
// when that language is actually selected. All 20 used to be eager, which
// parsed ~1.8 MB of JSON into every window (settings, the assistant panel, the
// overlay, the reminder popup, the meeting pill) to show one language.
const localeLoaders = import.meta.glob<{ default: TranslationTable }>(
  "./locales/*/translation.json",
);
const loaders: Record<string, () => Promise<{ default: TranslationTable }>> =
  {};
for (const [path, load] of Object.entries(localeLoaders)) {
  const langCode = path.match(/\.\/locales\/(.+)\/translation\.json/)?.[1];
  if (langCode) {
    loaders[langCode] = load;
  }
}

/** i18next backend that resolves a language through its lazy chunk. */
const lazyLocales: BackendModule = {
  type: "backend",
  init: () => {},
  read: (language, _namespace, callback) => {
    if (language === "en") {
      callback(null, en as TranslationTable);
      return;
    }
    const load = loaders[language];
    if (!load) {
      callback(null, {});
      return;
    }
    load()
      .then((module) => callback(null, module.default))
      .catch((error: unknown) => callback(error as Error, false));
  },
};

// Build supported languages list from discovered locales + metadata
export const SUPPORTED_LANGUAGES = Object.keys(loaders)
  .map((code) => {
    const meta = LANGUAGE_METADATA[code];
    if (!meta) {
      console.warn(`Missing metadata for locale "${code}" in languages.ts`);
      return { code, name: code, nativeName: code, priority: undefined };
    }
    return {
      code,
      name: meta.name,
      nativeName: meta.nativeName,
      priority: meta.priority,
    };
  })
  .sort((a, b) => {
    // Sort by priority first (lower = higher), then alphabetically
    if (a.priority !== undefined && b.priority !== undefined) {
      return a.priority - b.priority;
    }
    if (a.priority !== undefined) return -1;
    if (b.priority !== undefined) return 1;
    return a.name.localeCompare(b.name);
  });

export type SupportedLanguageCode = string;

// Check if a language code is supported
const getSupportedLanguage = (
  langCode: string | null | undefined,
): SupportedLanguageCode | null => {
  if (!langCode) return null;
  const normalized = langCode.toLowerCase();
  // Try exact match first
  let supported = SUPPORTED_LANGUAGES.find(
    (lang) => lang.code.toLowerCase() === normalized,
  );
  if (!supported) {
    // Fall back to prefix match (language only, without region)
    const prefix = normalized.split("-")[0];
    supported = SUPPORTED_LANGUAGES.find(
      (lang) => lang.code.toLowerCase() === prefix,
    );
  }
  return supported ? supported.code : null;
};

// Initialize i18n with English as default
// Language will be synced from settings after init
i18n
  .use(lazyLocales)
  .use(initReactI18next)
  .init({
    // English ships in the bundle; other languages arrive through `lazyLocales`
    // the first time they are selected.
    resources: { en: { translation: en as TranslationTable } },
    partialBundledLanguages: true,
    // Load exactly the selected language (plus the English fallback), not
    // also its base language — "zh-TW" should not pull in "zh" as well.
    load: "currentOnly",
    lng: "en",
    fallbackLng: "en",
    interpolation: {
      escapeValue: false, // React already escapes values
    },
    react: {
      useSuspense: false, // Disable suspense for SSR compatibility
    },
  });

// Sync language from app settings
export const syncLanguageFromSettings = async () => {
  try {
    const result = await commands.getAppSettings();
    if (result.status === "ok" && result.data.app_language) {
      const supported = getSupportedLanguage(result.data.app_language);
      if (supported && supported !== i18n.language) {
        await i18n.changeLanguage(supported);
      }
    } else {
      // Fall back to system locale detection if no saved preference
      const systemLocale = await locale();
      const supported = getSupportedLanguage(systemLocale);
      if (supported && supported !== i18n.language) {
        await i18n.changeLanguage(supported);
      }
    }
  } catch (e) {
    console.warn("Failed to sync language from settings:", e);
  }
};

// Run language sync on init
syncLanguageFromSettings();

// Listen for language changes to update HTML dir and lang attributes
i18n.on("languageChanged", (lng) => {
  const dir = getLanguageDirection(lng);
  updateDocumentDirection(dir);
  updateDocumentLanguage(lng);
});

// Re-export RTL utilities for convenience
export { getLanguageDirection, isRTLLanguage } from "@/lib/utils/rtl";

export default i18n;
