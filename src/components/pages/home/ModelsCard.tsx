import React from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight } from "lucide-react";
import { SectionTitle, TextLink } from "@/components/ui/Page";
import { MODEL_SLOTS, useNavigation } from "@/components/shell/navigation";
import {
  useModelSlots,
  type SlotSummary,
} from "@/components/shell/useModelSlots";
import { SlotLogo, useSlotStatusText } from "@/components/shell/SlotVisuals";

/**
 * What is doing each job right now: the job, then the model in words a person
 * would use. The logo says who runs it; the provider and exact id are one
 * hover away. A row opens that job's tab on Models.
 */
const SetupRow: React.FC<{ summary: SlotSummary }> = ({ summary }) => {
  const { t } = useTranslation();
  const { openModelSlot } = useNavigation();
  const statusText = useSlotStatusText();
  const attention = summary.active && !summary.ready;
  const tooltip = [summary.modelId, summary.providerLabel]
    .filter(Boolean)
    .join(" · ");

  return (
    <li>
      <button
        type="button"
        onClick={() => openModelSlot(summary.slot)}
        title={tooltip || undefined}
        className="group flex w-full cursor-pointer items-center gap-3 px-4 py-2.5 text-start transition-colors hover:bg-surface-muted focus-visible:bg-surface-muted focus-visible:outline-none"
      >
        <SlotLogo summary={summary} size="md" />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-xs text-muted">
            {t(`modelsHub.slots.${summary.slot}.name`)}
          </span>
          <span
            className={`flex items-center gap-1.5 truncate text-sm font-medium ${
              attention
                ? "text-warning"
                : summary.active
                  ? "text-ink"
                  : "text-muted"
            }`}
          >
            {attention && (
              <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-warning" />
            )}
            <span className="truncate">
              {summary.active && summary.ready && summary.modelLabel
                ? summary.modelLabel
                : statusText(summary)}
            </span>
          </span>
        </span>
        <ChevronRight
          className="h-4 w-4 shrink-0 text-muted-soft transition-transform group-hover:translate-x-0.5 rtl:rotate-180"
          aria-hidden="true"
        />
      </button>
    </li>
  );
};

export const ModelsCard: React.FC = () => {
  const { t } = useTranslation();
  const slots = useModelSlots();
  const { navigate } = useNavigation();

  return (
    <section>
      <SectionTitle
        title={t("home.setup.title")}
        action={
          <TextLink onClick={() => navigate("models")}>
            {t("home.setup.manage")}
          </TextLink>
        }
      />
      <ul className="divide-y divide-hairline overflow-hidden rounded-2xl border border-hairline bg-surface elev-card">
        {MODEL_SLOTS.map((slot) => (
          <SetupRow key={slot} summary={slots[slot]} />
        ))}
      </ul>
    </section>
  );
};
