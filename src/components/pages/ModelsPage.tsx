import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Page, PageHeader } from "@/components/ui/Page";
import { Tabs } from "@/components/ui/Tabs";
import { InfoTip } from "@/components/ui/InfoTip";
import { Freeze } from "@/components/shell/Freeze";
import {
  MODEL_SLOTS,
  useNavigation,
  type ModelSlot,
} from "@/components/shell/navigation";
import { useModelSlots } from "@/components/shell/useModelSlots";
import { MODEL_TABS } from "./models/ModelTabs";
import { USED_BY_KEYS } from "./models/usedBy";

/** "Used by dictation, the assistant and meetings." — behind the tab's (i). */
const UsedByTip: React.FC<{ slot: ModelSlot }> = ({ slot }) => {
  const { t } = useTranslation();
  const features = USED_BY_KEYS[slot]
    .map((key) => t(`modelsHub.usedBy.${key}`))
    .join(", ");
  return (
    <p className="flex items-center gap-1 text-[0.8125rem] text-muted">
      <span>{t(`modelsHub.slots.${slot}.description`)}</span>
      <InfoTip text={t("models.usedBy", { features })} />
    </p>
  );
};

/**
 * Models: one tab per job — speech to text, AI cleanup, assistant, voice.
 *
 * Tabs keep every job one click from the others, and arriving here from
 * another page (Home's model list, AI cleanup's picker) leaves a real Back
 * link to that page in the header. Adding a model file lives with the lists it
 * adds to, not in the header of every tab (it meant nothing on Voice).
 *
 * Tabs stay mounted once opened (frozen while hidden) so flipping between them
 * does not refetch hardware info or rebuild a model list.
 */
export const ModelsPage: React.FC = () => {
  const { t } = useTranslation();
  const { modelSlot, setModelTab } = useNavigation();
  const slots = useModelSlots();
  const tab: ModelSlot = modelSlot ?? "stt";
  const [visited, setVisited] = useState<ModelSlot[]>([tab]);
  const topRef = useRef<HTMLDivElement>(null);
  if (!visited.includes(tab)) setVisited([...visited, tab]);

  // A tab switch starts the new tab at its top, not mid-way down the last one.
  const previousTab = useRef(tab);
  useEffect(() => {
    if (previousTab.current === tab) return;
    previousTab.current = tab;
    const layer = topRef.current?.closest("[data-page]");
    if (layer && layer.scrollTop > (topRef.current?.offsetTop ?? 0)) {
      layer.scrollTo({ top: 0 });
    }
  }, [tab]);

  return (
    <Page>
      <div ref={topRef} />
      <PageHeader
        title={t("nav.models")}
        description={t("modelsHub.privacyNote")}
      />

      <Tabs
        label={t("nav.models")}
        value={tab}
        onChange={setModelTab}
        items={MODEL_SLOTS.map((slot) => ({
          id: slot,
          label: t(`modelsHub.slots.${slot}.name`),
          badge:
            slots[slot].active && !slots[slot].ready ? (
              <span
                className="h-1.5 w-1.5 rounded-full bg-warning"
                aria-label={t("modelsHub.needsSetup")}
              />
            ) : undefined,
        }))}
      />

      <div className="mt-6">
        {visited.map((slot) => {
          const Content = MODEL_TABS[slot];
          const active = slot === tab;
          return (
            <div
              key={slot}
              role="tabpanel"
              hidden={!active}
              className={active ? "tab-reveal" : undefined}
            >
              <div className="mb-6">
                <UsedByTip slot={slot} />
              </div>
              <Freeze freeze={!active}>
                <Content />
              </Freeze>
            </div>
          );
        })}
      </div>
    </Page>
  );
};
