import React from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight } from "lucide-react";
import { Page } from "@/components/ui/Page";
import { Button } from "@/components/ui/Button";
import { Hero, HeroShortcut } from "@/components/ui/Hero";
import { ShortcutInput } from "@/components/settings/ShortcutInput";
import { useSettings } from "@/hooks/useSettings";
import { useNavigation } from "@/components/shell/navigation";
import { useModelSlots } from "@/components/shell/useModelSlots";
import { ShortcutsCard } from "./home/ShortcutsCard";
import { ModelsCard } from "./home/ModelsCard";
import { RecentCard } from "./home/RecentCard";
import { partOfDay } from "./home/usageMath";

/** Shown above everything else when dictation cannot run yet. */
const NeedsSpeechModel: React.FC = () => {
  const { t } = useTranslation();
  const { openModelSlot } = useNavigation();
  return (
    <div className="mb-6 flex flex-wrap items-center gap-x-4 gap-y-3 rounded-2xl border border-warning/25 bg-warning/[0.06] px-5 py-3.5">
      <span className="h-2 w-2 shrink-0 rounded-full bg-warning" />
      <p className="min-w-0 flex-1 text-sm font-medium text-ink">
        {t("home.needsModel.title")}
      </p>
      <Button size="sm" onClick={() => openModelSlot("stt")}>
        {t("home.needsModel.action")}
        <ArrowRight className="h-3.5 w-3.5 rtl:rotate-180" aria-hidden="true" />
      </Button>
    </div>
  );
};

/**
 * Home: the one shortcut that matters, what you have said, and how it is set
 * up.
 *
 * The hero carries the dictation keys at the size of real keys — and they are
 * the button that changes them. Below it, the other shortcuts beside the
 * models doing each job; then the last few things you said. Usage numbers
 * live on Insights.
 */
export const HomePage: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const slots = useModelSlots();
  const holdToTalk = getSetting("push_to_talk") ?? true;
  // With cleanup moved onto the dictation shortcut, these keys clean up too,
  // so the hero says so instead of listing a second shortcut below.
  const cleansUp =
    (getSetting("post_process_enabled") ?? false) &&
    (getSetting("post_process_on_dictation") ?? false);
  const greeting = t(`home.greeting.${partOfDay(new Date().getHours())}`);

  return (
    <Page>
      {!slots.stt.ready && <NeedsSpeechModel />}

      {/* No line art here: Home is read at a glance (and the art is off by
          default — only Dictionary draws it). */}
      <Hero
        title={greeting}
        subtitle={holdToTalk ? t("home.hero.hold") : t("home.hero.tap")}
        aside={
          <HeroShortcut
            label={
              cleansUp
                ? t("home.shortcuts.cleanup.title")
                : t("home.shortcuts.dictate.title")
            }
            hint={holdToTalk ? t("home.hero.holdHint") : t("home.hero.tapHint")}
          >
            <ShortcutInput
              shortcutId="transcribe"
              bare
              finish="glass"
              size="lg"
              showReset="never"
            />
          </HeroShortcut>
        }
      />

      <div className="mt-8 grid grid-cols-1 gap-10 @4xl:grid-cols-[minmax(0,1fr)_22rem] @4xl:gap-8">
        <ShortcutsCard />
        <ModelsCard />
      </div>

      <div className="mt-10">
        <RecentCard />
      </div>
    </Page>
  );
};
