import React from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight } from "lucide-react";
import { Page, PageHeader } from "@/components/ui/Page";
import { Button } from "@/components/ui/Button";
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
 * Home: the shortcuts, the models doing each job, and the last few things you
 * said. Usage numbers live on Insights.
 *
 * There is no banner. Home used to open on a dark stage with a waveform and
 * the dictation keys drawn large, and it was the most-visited decoration in
 * the app: the art said nothing the keys did not, and on the light theme it
 * was the darkest, highest-contrast thing on screen, so it pulled the eye off
 * the content under it. Dictate is now the first row of the shortcuts list,
 * beside the other three.
 */
export const HomePage: React.FC = () => {
  const { t } = useTranslation();
  const slots = useModelSlots();
  const greeting = t(`home.greeting.${partOfDay(new Date().getHours())}`);

  return (
    <Page>
      <PageHeader title={greeting} />

      {!slots.stt.ready && <NeedsSpeechModel />}

      <div className="grid grid-cols-1 gap-10 @4xl:grid-cols-[minmax(0,1fr)_22rem] @4xl:gap-8">
        <ShortcutsCard />
        <ModelsCard />
      </div>

      <div className="mt-10">
        <RecentCard />
      </div>
    </Page>
  );
};
