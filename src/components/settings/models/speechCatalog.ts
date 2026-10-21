/**
 * The rules behind the speech-to-text catalog on the Models page: which
 * models are offered, in what order, which are recommended, and what the
 * filters keep. Pure, so the rules are tested rather than eyeballed across
 * 69 rows.
 */

import type { ModelInfo } from "@/bindings";
import { modelSupportsLanguage, uniqueLanguages } from "@/lib/languageCodes";

/**
 * Handy's catalog order, the same rule as `catalog_order` in
 * `src-tauri/src/managers/model.rs`: editorial rank first whatever the
 * recommended flag says, then recommended before not, then more accurate,
 * then faster, then by name, with the id as the last tie-break.
 */
export function compareCatalogOrder(a: ModelInfo, b: ModelInfo): number {
  const rank = (m: ModelInfo) => m.recommended_rank ?? Number.MAX_SAFE_INTEGER;
  return (
    rank(a) - rank(b) ||
    Number(b.is_recommended) - Number(a.is_recommended) ||
    b.accuracy_score - a.accuracy_score ||
    b.speed_score - a.speed_score ||
    a.name.localeCompare(b.name) ||
    a.id.localeCompare(b.id)
  );
}

/**
 * Whether a model belongs in the list at all. An older model that is not on
 * this computer is offered nowhere: its catalog twin is the same model in the
 * current format, and two rows with one name read as the same model twice.
 */
export function isOffered(model: ModelInfo, onDisk: boolean): boolean {
  return !model.is_legacy || onDisk;
}

/** Streams text as you speak. A retired model does nothing at all. */
export function isStreamingModel(model: ModelInfo): boolean {
  return model.supports_streaming && !model.is_retired;
}

/** Can translate speech into English. */
export function isTranslatingModel(model: ModelInfo): boolean {
  return model.supports_translation && !model.is_retired;
}

export interface SpeechFilters {
  /** A canonical language code (`zh`, `no`), or `"all"`. */
  language: string;
  streaming: boolean;
  translate: boolean;
}

export const NO_FILTERS: SpeechFilters = {
  language: "all",
  streaming: false,
  translate: false,
};

export function hasFilters(filters: SpeechFilters): boolean {
  return filters.language !== "all" || filters.streaming || filters.translate;
}

/**
 * Whether a model passes the toolbar's filters. The language filter uses the
 * canonical rule, so "Chinese" finds a model listing `zh` and one listing
 * `zh-Hans`, and "Norwegian" finds one listing `nb`.
 */
export function matchesFilters(
  model: ModelInfo,
  filters: SpeechFilters,
): boolean {
  if (filters.streaming && !isStreamingModel(model)) return false;
  if (filters.translate && !isTranslatingModel(model)) return false;
  if (
    filters.language !== "all" &&
    !modelSupportsLanguage(model.supported_languages, filters.language)
  ) {
    return false;
  }
  return true;
}

/**
 * What the speech catalog lists: a short list of good choices (the default),
 * or every model with search and filters.
 */
export type SpeechView = "recommended" | "all";

/** For one language, the Recommended view tops its picks up to this many. */
export const RECOMMENDED_FOR_LANGUAGE = 3;

/** A model the catalog can recommend: a current catalog model, not an older
 *  one kept on disk and not a file the user brought. */
function isRecommendable(model: ModelInfo): boolean {
  return (
    !model.is_legacy &&
    !model.is_retired &&
    !model.is_custom &&
    !model.local_path
  );
}

/** Dictation wants accuracy first, but a model too slow to keep up is not a
 *  good pick either. */
const balancedScore = (model: ModelInfo) =>
  model.accuracy_score * 0.6 + model.speed_score * 0.4;

/**
 * The Recommended view's models, in the order shown.
 *
 * For every language, the catalog's own picks (`recommended`: the five Handy
 * recommends and first-run setup offers) in catalog order. For one language,
 * the picks that hear it, topped up to `RECOMMENDED_FOR_LANGUAGE` with the
 * best other models for it, so someone who speaks Russian or Nepali still
 * gets a short list rather than one model or none. The top-up ranks by
 * accuracy and speed together: accuracy alone would hand everyone a 3 GB
 * model that cannot keep up with dictation.
 */
export function recommendedModels(
  models: ReadonlyArray<ModelInfo>,
  language: string,
): ModelInfo[] {
  const candidates = models.filter(
    (model) =>
      isRecommendable(model) &&
      (language === "all" ||
        modelSupportsLanguage(model.supported_languages, language)),
  );
  const picks = candidates
    .filter((model) => model.is_recommended)
    .sort(compareCatalogOrder);
  if (language === "all" || picks.length >= RECOMMENDED_FOR_LANGUAGE) {
    return picks;
  }
  const topUp = candidates
    .filter((model) => !model.is_recommended)
    .sort(
      (a, b) =>
        balancedScore(b) - balancedScore(a) || compareCatalogOrder(a, b),
    )
    .slice(0, RECOMMENDED_FOR_LANGUAGE - picks.length);
  return [...picks, ...topUp];
}

/**
 * The languages the filter offers: every language some listed model hears,
 * each once in canonical form, so Simplified and Traditional Chinese are one
 * "Chinese" and `nb` and `no` one "Norwegian".
 */
export function filterLanguages(models: ReadonlyArray<ModelInfo>): string[] {
  return uniqueLanguages(models.flatMap((model) => model.supported_languages));
}
