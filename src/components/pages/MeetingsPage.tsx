import React from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight } from "lucide-react";
import { Page } from "@/components/ui/Page";
import { InfoTip } from "@/components/ui/InfoTip";
import { MeetingsSection } from "@/components/settings/meetings/MeetingsSection";
import { useNavigation, type ModelSlot } from "@/components/shell/navigation";
import { useModelSlots } from "@/components/shell/useModelSlots";
import { SlotLogo, useSlotStatusText } from "@/components/shell/SlotVisuals";

/**
 * "Transcribed by [logo] Scribe v2 Realtime" — who does each half of a
 * meeting, as a quiet link to the model's tab. The logo says which company;
 * the (i) says which Models tab controls it, so clicking through is expected
 * rather than a surprise.
 */
const ModelLink: React.FC<{
  slot: ModelSlot;
  label: string;
}> = ({ slot, label }) => {
  const { openModelSlot } = useNavigation();
  const summary = useModelSlots()[slot];
  const statusText = useSlotStatusText();
  // Notes use the cleanup model whether or not cleanup itself is switched on.
  const shown = { ...summary, active: true };
  const value =
    shown.ready && shown.modelLabel ? shown.modelLabel : statusText(shown);
  return (
    <button
      type="button"
      onClick={() => openModelSlot(slot)}
      title={[summary.modelId, summary.providerLabel]
        .filter(Boolean)
        .join(" · ")}
      className="glass-ghost group -mx-2 inline-flex min-w-0 max-w-full cursor-pointer items-center gap-2 rounded-lg px-2 py-1 text-start text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/60"
    >
      <span className="shrink-0 text-white/65">{label}</span>
      <SlotLogo summary={shown} size="sm" className="shrink-0" />
      <span className="truncate font-medium text-white">{value}</span>
      <ChevronRight
        className="h-3.5 w-3.5 shrink-0 text-white/50 transition-transform group-hover:translate-x-0.5 rtl:rotate-180"
        aria-hidden="true"
      />
    </button>
  );
};

/**
 * Meetings. Which model writes the transcript and which writes the notes is
 * on the hero, one click from changing it.
 */
export const MeetingsPage: React.FC = () => {
  const { t } = useTranslation();
  return (
    <Page>
      <MeetingsSection
        heroFooter={
          <div className="flex flex-wrap items-center gap-x-5 gap-y-1">
            <ModelLink slot="stt" label={t("meetingsPage.transcribedBy")} />
            {/* The (i) travels with the last link, so it never wraps onto a
                line of its own. */}
            <span className="inline-flex min-w-0 max-w-full items-center gap-1">
              <ModelLink slot="cleanup" label={t("meetingsPage.notesBy")} />
              <InfoTip tone="onHero" text={t("meetingsPage.modelsTip")} />
            </span>
          </div>
        }
      />
    </Page>
  );
};
