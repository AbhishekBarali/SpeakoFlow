import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Check, ChevronDown, Download } from "lucide-react";
import type { ModelInfo } from "@/bindings";
import { Button } from "@/components/ui/Button";
import Badge from "@/components/ui/Badge";
import { getModelBrand } from "@/components/icons/BrandLogos";
import { useModelStore } from "@/stores/modelStore";
import { formatModelSize } from "@/lib/utils/format";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { getTranslatedModelName } from "@/lib/utils/modelTranslation";
import { OnboardingFrame, StepHeading } from "./OnboardingFrame";
import {
  baseLanguages,
  languageNames,
  recommend,
  SETUP_MODELS,
  SETUP_SPEECH_OPTIONS,
} from "./recommend";
import { useSetupQueue } from "./setupQueue";
import { useHardwareFacts } from "./useHardwareFacts";

interface Option {
  model: ModelInfo;
  name: string;
  /** Which model of the family ("Medium"), or null when `name` says it all. */
  variant: string | null;
  /** `onboarding.speech.tags.*`, or null for a model setup has no word for. */
  tag: (typeof SETUP_SPEECH_OPTIONS)[number]["tag"] | null;
  primary: boolean;
}

/** The option's full name, for anything that shows it without the label. */
const fullName = (option: Option) =>
  option.variant ? `${option.name} ${option.variant}` : option.name;

/** One model, as a choice: its mark, its name, a few words, its size. */
const ModelOption: React.FC<{
  option: Option;
  selected: boolean;
  recommended: boolean;
  onSelect: () => void;
}> = ({ option, selected, recommended, onSelect }) => {
  const { t, i18n } = useTranslation();
  const { model } = option;
  const brand = getModelBrand(model);
  const languages = baseLanguages(model.supported_languages);
  const reach =
    languages.length <= 1
      ? t("onboarding.speech.english")
      : t("onboarding.speech.languages", { count: languages.length });
  const line = option.tag
    ? `${reach} · ${t(`onboarding.speech.tags.${option.tag}`)}`
    : reach;
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className={`ob-option group flex w-full cursor-pointer items-center gap-4 rounded-2xl border px-4 py-3.5 text-start transition-[border-color,background-color,box-shadow] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${
        selected
          ? "border-accent/60 bg-accent/[0.05] shadow-[0_0_0_1px_color-mix(in_srgb,var(--color-accent)_30%,transparent)]"
          : "border-hairline bg-surface elev-card hover:border-hairline-strong"
      }`}
    >
      <span
        aria-hidden="true"
        className={`grid h-11 w-11 shrink-0 place-items-center overflow-hidden rounded-xl [&_svg]:h-6 [&_svg]:w-6 ${brand.tileClass}`}
      >
        {brand.icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex min-w-0 items-center gap-2">
          <span className="truncate text-base font-semibold text-ink">
            {option.name}
          </span>
          {option.variant && (
            <span className="shrink-0 rounded-md border border-hairline-strong px-1.5 py-px text-[0.6875rem] font-medium tabular-nums text-muted">
              {option.variant}
            </span>
          )}
          {recommended && (
            <Badge variant="active">{t("onboarding.recommended")}</Badge>
          )}
        </span>
        <span
          className="mt-0.5 block truncate text-[0.8125rem] text-muted"
          // The language names are there for anyone who wants them, and cost
          // nobody else a line of reading.
          title={
            languages.length > 1
              ? languageNames(languages, i18n.language).join(", ")
              : undefined
          }
        >
          {line}
        </span>
      </span>
      <span className="shrink-0 text-[0.8125rem] tabular-nums text-muted">
        {model.is_downloaded ? (
          <span className="inline-flex items-center gap-1 font-medium text-accent">
            <Check className="h-3.5 w-3.5" aria-hidden="true" />
            {t("onboarding.setup.installed")}
          </span>
        ) : (
          formatModelSize(Number(model.size_mb))
        )}
      </span>
      <span
        aria-hidden="true"
        className={`grid h-5 w-5 shrink-0 place-items-center rounded-full border-2 transition-colors ${
          selected ? "border-accent bg-accent" : "border-hairline-strong"
        }`}
      >
        {selected && <span className="h-2 w-2 rounded-full bg-surface" />}
      </span>
    </button>
  );
};

/**
 * One decision: which speech model. It is a list of choices, not an
 * explanation: the two most people want (English, or any of 28 languages), the
 * rest one click away under "More models", and each card says only what tells
 * them apart. The model for this computer's language is picked already.
 */
export function SetupStep({
  onContinue,
}: {
  onContinue: (result: { skippedModels: boolean }) => void;
}) {
  const { t } = useTranslation();
  const facts = useHardwareFacts();
  const models = useModelStore((s) => s.models);
  const currentModel = useModelStore((s) => s.currentModel);
  const loading = useModelStore((s) => s.loading);
  const initialized = useModelStore((s) => s.initialized);
  const loadModels = useModelStore((s) => s.loadModels);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [showMore, setShowMore] = useState(false);

  const options = useMemo<Option[]>(() => {
    const listed = SETUP_SPEECH_OPTIONS.flatMap((entry) => {
      const model = models.find((m) => m.id === entry.id);
      return model
        ? [
            {
              model,
              name: entry.name,
              variant: entry.variant,
              tag: entry.tag,
              primary: entry.primary,
            },
          ]
        : [];
    });
    // A speech model already on this machine that setup does not list (a
    // replay, or an upgrade from an older version) is still a choice.
    const listedIds = new Set(listed.map((option) => option.model.id));
    const installed = models
      .filter(
        (m) =>
          m.is_downloaded &&
          getModelCategory(m) === "stt" &&
          !listedIds.has(m.id),
      )
      .map((model) => ({
        model,
        name: getTranslatedModelName(model, t),
        variant: null,
        tag: null,
        primary: false,
      }));
    return [...listed, ...installed];
  }, [models, t]);

  const recommendedId = facts
    ? SETUP_MODELS.speech[recommend(facts).speech]
    : null;
  const installedCurrent = options.find(
    (option) => option.model.id === currentModel && option.model.is_downloaded,
  );

  // Someone who already dictates keeps their model; everyone else gets the
  // one for their language.
  useEffect(() => {
    if (selectedId) return;
    const initial = installedCurrent?.model.id ?? recommendedId;
    if (initial && options.some((option) => option.model.id === initial)) {
      setSelectedId(initial);
    }
  }, [installedCurrent, recommendedId, options, selectedId]);

  const selected = options.find((option) => option.model.id === selectedId);
  // Opening on a model that lives under "More models" shows it.
  useEffect(() => {
    if (selected && !selected.primary) setShowMore(true);
  }, [selected]);

  const failed = initialized && !loading && models.length === 0;
  const primary = options.filter((option) => option.primary);
  const more = options.filter((option) => !option.primary);
  const model = selected?.model;
  const ready = !!model && (model.is_downloaded || model.is_downloading);

  const start = () => {
    if (!model || !selected) return;
    useSetupQueue.getState().start([
      {
        job: "stt",
        modelId: model.id,
        label: fullName(selected),
        sizeMb: model.is_downloaded ? 0 : Number(model.size_mb),
        wiring: { kind: "stt" },
      },
    ]);
    onContinue({ skippedModels: false });
  };

  const renderOption = (option: Option) => (
    <ModelOption
      key={option.model.id}
      option={option}
      selected={option.model.id === selectedId}
      recommended={
        option.model.id === recommendedId && !option.model.is_downloaded
      }
      onSelect={() => setSelectedId(option.model.id)}
    />
  );

  return (
    <OnboardingFrame
      step="setup"
      footer={
        <>
          <Button
            variant="ghost"
            size="lg"
            onClick={() => onContinue({ skippedModels: true })}
          >
            {t("onboarding.speech.later")}
          </Button>
          <Button size="lg" disabled={!model} onClick={start}>
            {!ready && <Download className="h-4 w-4" aria-hidden="true" />}
            {t(
              ready
                ? "onboarding.speech.continue"
                : "onboarding.speech.download",
            )}
            <ArrowRight className="h-4 w-4 rtl:rotate-180" aria-hidden="true" />
          </Button>
        </>
      }
    >
      <StepHeading
        title={t("onboarding.speech.title")}
        body={t("onboarding.speech.body")}
      />

      {failed ? (
        <div
          role="alert"
          className="ob-gap flex flex-wrap items-center gap-3 rounded-2xl border border-hairline bg-surface px-5 py-4 elev-card"
        >
          <p className="min-w-0 flex-1 text-sm text-ink">
            {t("onboarding.setup.loadFailed")}
          </p>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => void loadModels()}
          >
            {t("onboarding.setup.retry")}
          </Button>
        </div>
      ) : primary.length === 0 ? (
        <div
          className="ob-gap space-y-2.5"
          role="status"
          aria-label={t("onboarding.speech.loading")}
        >
          {[0, 1].map((i) => (
            <div
              key={i}
              className="h-[4.5rem] animate-pulse rounded-2xl border border-hairline bg-surface motion-reduce:animate-none"
            />
          ))}
        </div>
      ) : (
        <div
          role="radiogroup"
          aria-label={t("onboarding.speech.title")}
          className="ob-gap space-y-2.5"
        >
          {primary.map(renderOption)}
          {more.length > 0 &&
            (showMore ? (
              <div className="ob-reveal space-y-2.5">
                {more.map(renderOption)}
              </div>
            ) : (
              <button
                type="button"
                onClick={() => setShowMore(true)}
                className="flex cursor-pointer items-center gap-1.5 rounded-lg px-2 py-1.5 text-[0.8125rem] font-medium text-muted transition-colors hover:bg-ink/[0.05] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
              >
                <ChevronDown className="h-4 w-4" aria-hidden="true" />
                {t("onboarding.speech.more", { count: more.length })}
              </button>
            ))}
        </div>
      )}

      <p className="mt-6 px-1 text-[0.8125rem] text-muted">
        {t("onboarding.speech.anytime")}
      </p>
    </OnboardingFrame>
  );
}
