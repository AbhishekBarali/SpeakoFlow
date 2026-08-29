import React from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Check, Download } from "lucide-react";
import type { ModelInfo } from "@/bindings";
import { formatModelSize } from "@/lib/utils/format";
import {
  getTranslatedModelDescription,
  getTranslatedModelName,
} from "@/lib/utils/modelTranslation";
import { getModelBrand } from "@/components/icons/BrandLogos";
import Badge from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";

interface CleanupModelRowProps {
  /** The model currently assigned to on-device cleanup, if it is on disk. */
  model?: ModelInfo;
  onChangeModel: () => void;
}

/**
 * Collapse a model name or filename to its bare characters, so two spellings of
 * the same thing compare equal: `FLOW Qwen3.5-2B.Q8_0.gguf` and the display name
 * derived from it, `FLOW Qwen3.5 2B.Q8 0`, both become `flowqwen352bq80`.
 */
const comparableName = (value: string): string =>
  value
    .replace(/\.(gguf|bin|safetensors)$/i, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "");

/**
 * The on-device cleanup model, as a card rather than a dropdown entry.
 *
 * A dropdown was the wrong control here: it showed a filename and nothing that
 * helps choose (what the model is for, how big it is, whether it is ours), and
 * it could only list models that were *already downloaded* — so the only way to
 * get a new one was to leave Dictation entirely and go find the assistant's
 * page. This card carries the decision-making facts and owns the route to the
 * catalog, so choosing a cleanup model never starts on a page about something
 * else.
 */
export const CleanupModelRow: React.FC<CleanupModelRowProps> = ({
  model,
  onChangeModel,
}) => {
  const { t } = useTranslation();

  if (!model) {
    return (
      <div className="flex flex-col gap-3 px-4 py-3.5 sm:flex-row sm:items-center">
        <div className="flex min-w-0 flex-1 items-start gap-3.5">
          <span className="grid h-10 w-10 shrink-0 place-items-center rounded-xl bg-accent/12 text-accent">
            <Download className="h-[18px] w-[18px]" aria-hidden="true" />
          </span>
          <div className="min-w-0 flex-1">
            <h3 className="text-sm font-semibold tracking-tight text-ink">
              {t("settings.dictation.aiCleanup.model.emptyTitle")}
            </h3>
            <p className="mt-1 max-w-[68ch] text-xs leading-relaxed text-muted">
              {t("settings.dictation.aiCleanup.model.emptyDescription")}
            </p>
          </div>
        </div>
        <Button variant="primary" size="sm" onClick={onChangeModel}>
          {t("settings.dictation.aiCleanup.model.choose")}
          <ArrowRight className="h-3.5 w-3.5" aria-hidden="true" />
        </Button>
      </div>
    );
  }

  const brand = getModelBrand(model);
  const displayName = getTranslatedModelName(model, t).replace(
    /\s*\(vision\)\s*$/i,
    "",
  );
  // The line under the name has to carry a *second* fact. For a catalog model it
  // does: the description says what the model is for. For a model the user
  // registered from disk there is none — its description is the file path, and
  // the filename that collapses to is the very string the display name is
  // derived from ("FLOW Qwen3.5 2B.Q8 0" over "FLOW Qwen3.5-2B.Q8_0.gguf"). Two
  // renderings of one string is not a second fact, so the subtitle is dropped
  // whenever it merely restates the name. The full path stays in the tooltip,
  // where it matters when something breaks rather than while reading the row.
  const candidateSubtitle = model.local_path
    ? (model.local_path.split(/[\\/]/).pop() ?? model.local_path)
    : getTranslatedModelDescription(model, t);
  const subtitle =
    candidateSubtitle &&
    comparableName(candidateSubtitle) !== comparableName(displayName)
      ? candidateSubtitle
      : null;
  const size = formatModelSize(Number(model.size_mb));

  return (
    <div className="flex flex-col gap-3 px-4 py-3.5 sm:flex-row sm:items-center">
      <div className="flex min-w-0 flex-1 items-start gap-3.5">
        <span
          className={`grid h-10 w-10 shrink-0 place-items-center rounded-xl ${brand.tileClass}`}
        >
          {brand.icon}
        </span>

        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="text-sm font-semibold tracking-tight text-ink">
              {displayName}
            </h3>
            <Badge variant="active" className="gap-1">
              <Check className="h-3 w-3" aria-hidden="true" />
              {t("modelSelector.active")}
            </Badge>
            {model.is_cleanup_specialist && (
              <Badge variant="active">
                {t("settings.dictation.aiCleanup.model.tunedBadge")}
              </Badge>
            )}
          </div>
          {/* One line, size included, so the card stays a row rather than a
              paragraph. */}
          <p
            className="mt-1 truncate text-xs leading-relaxed text-muted"
            title={model.local_path ?? undefined}
          >
            {subtitle ? (
              <>
                {subtitle}
                <span className="text-muted-soft">
                  {" · "}
                  {size}
                </span>
              </>
            ) : (
              size
            )}
          </p>
        </div>
      </div>

      <Button variant="secondary" size="sm" onClick={onChangeModel}>
        {t("settings.dictation.hero.changeModel")}
      </Button>
    </div>
  );
};
