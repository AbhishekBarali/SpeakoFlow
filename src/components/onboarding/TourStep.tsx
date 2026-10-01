import React, { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowLeft,
  ArrowRight,
  MessageCircle,
  Mic,
  PhoneCall,
  Wand2,
} from "lucide-react";
import { Button } from "@/components/ui/Button";
import { Keycaps } from "@/components/ui/Keycaps";
import { Tabs } from "@/components/ui/Tabs";
import { useSettings } from "@/hooks/useSettings";
import { useReducedMotion } from "@/hooks/useReducedMotion";
import { OnboardingFrame } from "./OnboardingFrame";
import {
  ASK_EXAMPLE_IDS,
  AskScene,
  CallScene,
  DictateScene,
  Stage,
  type SceneId,
} from "./tourScenes";

const ORDER: SceneId[] = ["dictate", "ask", "call"];
const ICONS = { dictate: Mic, ask: MessageCircle, call: PhoneCall } as const;
const BINDING: Record<SceneId, string> = {
  dictate: "transcribe",
  ask: "assistant",
  call: "assistant_call",
};

/** A small rounded label, the app's chip style. */
const Chip: React.FC<{
  icon?: React.ReactNode;
  children: React.ReactNode;
}> = ({ icon, children }) => (
  <span className="inline-flex items-center gap-1.5 rounded-full border border-hairline bg-surface px-2.5 py-1 text-[0.8125rem] font-medium text-muted">
    {icon}
    {children}
  </span>
);

/**
 * How it is done, in three numbered steps. They hold still: the preview beside
 * them is the moving part. Lighting each step in turn as the preview acted it
 * out put a second animation next to the first, and the eye kept jumping
 * between the 1-2-3 loop and the preview instead of watching either.
 */
const Steps: React.FC<{
  label: string;
  steps: React.ReactNode[];
}> = ({ label, steps }) => (
  <ol className="ob-steps" aria-label={label}>
    {steps.map((content, i) => (
      <li key={i} className="ob-step">
        <span className="ob-step-num" aria-hidden="true">
          {i + 1}
        </span>
        <span className="ob-step-body">{content}</span>
      </li>
    ))}
  </ol>
);

/**
 * Three things you can do, each shown with the surface you will actually see,
 * and beside it the steps to do it. This is the last screen of first-run
 * setup: Continue opens the app, while the speech model chosen in setup keeps
 * downloading (the sidebar shows its progress).
 * It only shows: nothing here downloads or switches anything on.
 */
export const TourStep: React.FC<{
  onDone: () => void;
  initialIndex?: number;
}> = ({ onDone, initialIndex = 0 }) => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const still = useReducedMotion();
  const [index, setIndex] = useState(() =>
    Math.min(Math.max(0, initialIndex), ORDER.length - 1),
  );
  const scene = ORDER[index];
  const holdToTalk = settings?.push_to_talk ?? true;
  const binding = settings?.bindings?.[BINDING[scene]]?.current_binding;
  const compactOverlay = settings?.overlay_style === "minimal";
  const hint =
    scene === "call"
      ? t("onboarding.tour.keys.call")
      : holdToTalk
        ? t("onboarding.tour.keys.hold")
        : t("onboarding.tour.keys.tap");

  const go = useCallback(
    (next: number) => {
      if (next >= ORDER.length) {
        onDone();
        return;
      }
      setIndex(Math.max(0, next));
    },
    [onDone],
  );

  const sceneProps = { still, holdToTalk, binding };

  // The keys are a step of their own, drawn as keys, with how to press them
  // beside them.
  const keys = (
    <span className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
      <Keycaps
        binding={binding}
        size="md"
        fallback={
          <span className="text-muted">
            {t("settings.general.shortcut.notSet")}
          </span>
        }
      />
      <span>{hint}</span>
    </span>
  );
  const steps: Record<SceneId, React.ReactNode[]> = {
    dictate: [
      keys,
      t("onboarding.tour.steps.speak"),
      t("onboarding.tour.steps.typed"),
    ],
    ask: [
      t("onboarding.tour.steps.select"),
      keys,
      t("onboarding.tour.steps.use"),
    ],
    call: [
      keys,
      t("onboarding.tour.steps.talk"),
      t("onboarding.tour.steps.answer"),
    ],
  };

  return (
    <OnboardingFrame
      step="tour"
      width="wide"
      footer={
        <>
          <Button variant="ghost" size="lg" onClick={onDone}>
            {t("onboarding.tour.skip")}
          </Button>
          <div className="flex items-center gap-3">
            {index > 0 && (
              <Button
                variant="secondary"
                size="lg"
                onClick={() => go(index - 1)}
                aria-label={t("common.back")}
                className="w-10 px-0!"
              >
                <ArrowLeft
                  className="h-4 w-4 rtl:rotate-180"
                  aria-hidden="true"
                />
              </Button>
            )}
            <Button size="lg" onClick={() => go(index + 1)}>
              {index === ORDER.length - 1
                ? t("onboarding.tour.finish")
                : t("onboarding.tour.next")}
              <ArrowRight
                className="h-4 w-4 rtl:rotate-180"
                aria-hidden="true"
              />
            </Button>
          </div>
        </>
      }
    >
      <Tabs
        label={t("onboarding.tour.features")}
        value={scene}
        onChange={(id) => go(ORDER.indexOf(id))}
        items={ORDER.map((id) => ({
          id,
          label: t(`onboarding.tour.tabs.${id}`),
          icon: ICONS[id],
        }))}
      />

      <div className="ob-tour grid items-center">
        <div key={scene} className="ob-tour-copy min-w-0" aria-live="polite">
          <h1 className="ob-tour-title font-display leading-[1.05] whitespace-pre-line text-ink">
            {t(`onboarding.tour.${scene}.title`)}
          </h1>

          <div className="ob-tour-step">
            <Steps
              label={t("onboarding.tour.steps.label")}
              steps={steps[scene]}
            />
          </div>

          {scene === "dictate" && (
            <div className="ob-tour-step ob-tour-extra">
              <Chip icon={<Wand2 className="h-3.5 w-3.5" aria-hidden="true" />}>
                {t("onboarding.tour.dictate.cleanup")}
              </Chip>
            </div>
          )}
          {scene === "ask" && (
            <div className="ob-tour-step ob-tour-extra flex flex-wrap gap-1.5">
              {ASK_EXAMPLE_IDS.map((id) => (
                <Chip key={id}>{t(`onboarding.tour.ask.chips.${id}`)}</Chip>
              ))}
              <Chip>{t("onboarding.tour.ask.chips.anything")}</Chip>
            </div>
          )}
        </div>

        <Stage scene={scene}>
          {scene === "dictate" ? (
            <DictateScene
              key="dictate"
              {...sceneProps}
              compactOverlay={compactOverlay}
            />
          ) : scene === "ask" ? (
            <AskScene key="ask" {...sceneProps} />
          ) : (
            <CallScene key="call" {...sceneProps} />
          )}
        </Stage>
      </div>
    </OnboardingFrame>
  );
};
