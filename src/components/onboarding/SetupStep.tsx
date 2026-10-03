import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Check, Download } from "lucide-react";
import type { ModelInfo } from "@/bindings";
import { Button } from "@/components/ui/Button";
import Badge from "@/components/ui/Badge";
import { getModelBrand } from "@/components/icons/BrandLogos";
import { useModelStore } from "@/stores/modelStore";
import { formatModelSize } from "@/lib/utils/format";
import {
  getTranslatedModelDescription,
  getTranslatedModelName,
} from "@/lib/utils/modelTranslation";
import { OnboardingFrame, StepHeading } from "./OnboardingFrame";
import {
  baseLanguages,
  languageNames,
  recommend,
  RECOMMENDED_SETUP_SPEECH,
  SETUP_MODELS,
  SETUP_SPEECH_OPTIONS,
} from "./recommend";
import { useSetupQueue } from "./setupQueue";
import { useHardwareFacts } from "./useHardwareFacts";

interface Option {
  model: ModelInfo;
  name: string;
  /** The card's one sentence. */
  about: string;
  /** The model this computer already dictates with, when setup does not list
   *  it. Shown so that Continue keeps it rather than replacing it. */
  current: boolean;
}

/**
 * One model, as a choice: its mark, its name with the languages it hears, and
 * one sentence of why you would pick it. On the right, its size (or that it is
 * installed) and the radio.
 *
 * That is all on purpose. The card used to carry a third line of facts —
 * languages, live text, translation, five accuracy dots and five speed dots —
 * and a first-run choice between five models read like a spec sheet. Each
 * sentence already says what the model trades (fast, tiny, most accurate,
 * most languages), the language count answers the one question that can rule
 * a model out, and everything else is on the Models page.
 */
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
        <span className="flex min-w-0 items-baseline gap-2">
          <span className="truncate text-base font-semibold text-ink">
            {option.name}
          </span>
          <span
            className="shrink-0 text-[0.8125rem] text-muted"
            // The language names are there for anyone who wants them, and
            // cost nobody else a line of reading.
            title={
              languages.length > 1
                ? languageNames(languages, i18n.language).join(", ")
                : undefined
            }
          >
            {reach}
          </span>
          {option.current ? (
            <Badge variant="active">{t("modelSelector.active")}</Badge>
          ) : (
            recommended && (
              <Badge variant="active">{t("onboarding.recommended")}</Badge>
            )
          )}
        </span>
        <span className="mt-0.5 block text-sm leading-snug text-body">
          {option.about}
        </span>
      </span>
      <span className="flex shrink-0 items-center gap-4 self-center">
        <span className="text-[0.8125rem] tabular-nums text-muted">
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
      </span>
    </button>
  );
};

/**
 * One decision: which speech model. Five good choices, each saying in one
 * sentence what it is for, and the languages it hears. The
 * full catalog lives on the Models page. The model for this computer's
 * language is picked already.
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

  const options = useMemo<Option[]>(() => {
    const listed: Option[] = SETUP_SPEECH_OPTIONS.flatMap((entry) => {
      const model = models.find((m) => m.id === entry.id);
      return model
        ? [
            {
              model,
              name: entry.name,
              about: t(`onboarding.speech.about.${entry.about}`),
              current: false,
            },
          ]
        : [];
    });
    // Someone replaying setup who dictates with a model setup does not list
    // sees that one model too, so Continue can keep it. Only that one: every
    // other installed model stays on the Models page, where its near-namesakes
    // are told apart.
    const current = models.find(
      (m) => m.id === currentModel && m.is_downloaded,
    );
    if (current && !listed.some((option) => option.model.id === current.id)) {
      listed.unshift({
        model: current,
        name: getTranslatedModelName(current, t),
        about: getTranslatedModelDescription(current, t),
        current: true,
      });
    }
    return listed;
  }, [models, currentModel, t]);

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
  const failed = initialized && !loading && models.length === 0;
  const model = selected?.model;
  const ready = !!model && (model.is_downloaded || model.is_downloading);

  const start = () => {
    if (!model || !selected) return;
    useSetupQueue.getState().start([
      {
        job: "stt",
        modelId: model.id,
        label: selected.name,
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
        RECOMMENDED_SETUP_SPEECH.has(option.model.id) &&
        !option.model.is_downloaded
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
      ) : options.length === 0 ? (
        <div
          className="ob-gap space-y-2.5"
          role="status"
          aria-label={t("onboarding.speech.loading")}
        >
          {[0, 1, 2].map((i) => (
            <div
              key={i}
              className="h-[4.75rem] animate-pulse rounded-2xl border border-hairline bg-surface motion-reduce:animate-none"
            />
          ))}
        </div>
      ) : (
        <div
          role="radiogroup"
          aria-label={t("onboarding.speech.title")}
          className="ob-gap space-y-2.5"
        >
          {options.map(renderOption)}
        </div>
      )}

      <p className="mt-6 px-1 text-[0.8125rem] text-muted">
        {t("onboarding.speech.anytime")}
      </p>
    </OnboardingFrame>
  );
}
