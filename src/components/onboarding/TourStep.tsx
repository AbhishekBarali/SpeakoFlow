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
  active?: boolean;
  icon?: React.ReactNode;
  children: React.ReactNode;
}> = ({ active = false, icon, children }) => (
  <span
    className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[0.8125rem] font-medium transition-colors duration-300 ${
      active
        ? "border-accent/40 bg-accent/10 text-accent"
        : "border-hairline bg-surface text-muted"
    }`}
  >
    {icon}
    {children}
  </span>
);

/**
 * Three things you can do, each shown with the surface you will actually see,
 * and a headline and the keys beside it — nothing to read. The preview is the
 * explanation. It only shows: nothing here downloads or switches anything on.
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
  const [pressed, setPressed] = useState(false);
  const [example, setExample] = useState(0);
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
      setPressed(false);
      setExample(0);
      setIndex(Math.max(0, next));
    },
    [onDone],
  );
  const onPressed = useCallback((value: boolean) => setPressed(value), []);
  const onExample = useCallback((value: number) => setExample(value), []);

  const sceneProps = { still, onPressed, holdToTalk };

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
                ? t("onboarding.tour.done")
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

      <div className="ob-tour mt-10 grid items-center gap-10">
        <div key={scene} className="ob-tour-copy min-w-0" aria-live="polite">
          <h1 className="font-display text-[2.75rem] leading-[1.05] whitespace-pre-line text-ink">
            {t(`onboarding.tour.${scene}.title`)}
          </h1>

          <div className="mt-8 flex flex-col items-start gap-2.5">
            <span className="ob-keys" data-pressed={pressed}>
              <Keycaps
                binding={binding}
                size="lg"
                fallback={
                  <span className="text-sm text-muted">
                    {t("settings.general.shortcut.notSet")}
                  </span>
                }
              />
            </span>
            <span className="text-sm text-muted">{hint}</span>
          </div>

          {scene === "dictate" && (
            <div className="mt-8">
              <Chip icon={<Wand2 className="h-3.5 w-3.5" aria-hidden="true" />}>
                {t("onboarding.tour.dictate.cleanup")}
              </Chip>
            </div>
          )}
          {scene === "ask" && (
            <div className="mt-8 flex flex-wrap gap-1.5">
              {ASK_EXAMPLE_IDS.map((id, i) => (
                <Chip key={id} active={!still && i === example}>
                  {t(`onboarding.tour.ask.chips.${id}`)}
                </Chip>
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
            <AskScene key="ask" {...sceneProps} onExample={onExample} />
          ) : (
            <CallScene key="call" {...sceneProps} />
          )}
        </Stage>
      </div>
    </OnboardingFrame>
  );
};
