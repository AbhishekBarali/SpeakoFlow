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
 * "Transcribed by [logo] Scribe v2 Realtime ›" — one row of a two-row table.
 *
 * Each row is a single button laid onto its parent's columns (subgrid), so the
 * labels share one column and the logos, names and chevrons line up under
 * each other whatever the label lengths. The logo says which company; the
 * row opens that model's tab.
 */
const ModelRow: React.FC<{
  slot: ModelSlot;
  label: string;
}> = ({ slot, label }) => {
  const { openModelSlot } = useNavigation();
  const summary = useModelSlots()[slot];
  const statusText = useSlotStatusText();
  // Notes and questions use the assistant's model whether or not the assistant
  // panel itself is switched on.
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
      className="glass-ghost group col-span-2 grid cursor-pointer grid-cols-subgrid items-center rounded-lg px-2 py-1.5 text-start text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/60"
    >
      <span className="text-white/65">{label}</span>
      <span className="flex min-w-0 items-center gap-2">
        <SlotLogo summary={shown} size="sm" className="shrink-0" />
        <span className="min-w-0 flex-1 truncate font-medium text-white">
          {value}
        </span>
        <ChevronRight
          className="h-3.5 w-3.5 shrink-0 text-white/50 transition-transform group-hover:translate-x-0.5 rtl:rotate-180"
          aria-hidden="true"
        />
      </span>
    </button>
  );
};

/**
 * Meetings. Which model writes the transcript and which writes the notes is
 * on the hero, one click from changing it; the (i) beside the pair says what
 * each one does.
 */
export const MeetingsPage: React.FC = () => {
  const { t } = useTranslation();
  return (
    <Page>
      <MeetingsSection
        heroFooter={
          <div className="-ms-2 flex max-w-full items-center gap-1.5">
            <div className="grid min-w-0 grid-cols-[auto_minmax(0,auto)] gap-x-4 gap-y-0.5">
              <ModelRow slot="stt" label={t("meetingsPage.transcribedBy")} />
              <ModelRow slot="assistant" label={t("meetingsPage.notesBy")} />
            </div>
            <InfoTip tone="onHero" text={t("meetingsPage.modelsTip")} />
          </div>
        }
      />
    </Page>
  );
};
