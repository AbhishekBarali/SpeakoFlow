import type { TFunction } from "i18next";
import type { ModelInfo } from "@/bindings";

/**
 * Get the translated name for a model
 * @param model - The model info object
 * @param t - The translation function from useTranslation
 * @returns The translated model name, or the original name if no translation exists
 */
export function getTranslatedModelName(model: ModelInfo, t: TFunction): string {
  const translationKey = `onboarding.models.${model.id}.name`;
  const translated = t(translationKey, { defaultValue: "" });
  return translated !== "" ? translated : model.name;
}

/**
 * Get the translated description for a model
 * @param model - The model info object
 * @param t - The translation function from useTranslation
 * @returns The translated model description, or the original description if no translation exists
 */
export function getTranslatedModelDescription(
  model: ModelInfo,
  t: TFunction,
): string {
  // Custom models use their own description when available (e.g. the source
  // repo for a user-added GGUF model), falling back to a generic label.
  if (model.is_custom) {
    return model.description?.trim()
      ? model.description
      : t("onboarding.customModelDescription");
  }
  const translationKey = `onboarding.models.${model.id}.description`;
  const translated = t(translationKey, { defaultValue: "" });
  return translated !== "" ? translated : model.description;
}
