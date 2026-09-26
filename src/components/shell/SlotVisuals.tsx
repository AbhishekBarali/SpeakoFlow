import React from "react";
import { useTranslation } from "react-i18next";
import {
  AudioLines,
  Cloud,
  Cpu,
  MessageCircle,
  Mic,
  Volume2,
  Wand2,
} from "lucide-react";
import type { ModelInfo } from "@/bindings";
import { getModelBrand } from "@/components/icons/BrandLogos";
import { ProviderTile } from "@/components/icons/ProviderLogos";
import type { SettingIcon } from "@/components/ui/tones";
import type { ModelSlot } from "./navigation";
import type { SlotSummary } from "./useModelSlots";

/** Fixed identity for each model job: its glyph and copy key. */
export const SLOT_META: Record<ModelSlot, { icon: SettingIcon; key: string }> =
  {
    stt: { icon: Mic, key: "stt" },
    cleanup: { icon: Wand2, key: "cleanup" },
    assistant: { icon: MessageCircle, key: "assistant" },
    voice: { icon: Volume2, key: "voice" },
  };

type MarkSize = "sm" | "md" | "lg";

const BOX: Record<MarkSize, string> = {
  sm: "h-6 w-6 rounded-md [&_svg]:h-3.5 [&_svg]:w-3.5",
  md: "h-8 w-8 rounded-lg [&_svg]:h-[1.0625rem] [&_svg]:w-[1.0625rem]",
  lg: "h-10 w-10 rounded-xl [&_svg]:h-5 [&_svg]:w-5",
};

/** A catalog model's family mark (NVIDIA, Qwen, Gemma, ours) in its chip. */
export const ModelMark: React.FC<{
  model: ModelInfo;
  size?: MarkSize;
  className?: string;
}> = ({ model, size = "md", className = "" }) => {
  const brand = getModelBrand(model);
  return (
    <span
      aria-hidden="true"
      className={`grid shrink-0 place-items-center overflow-hidden ${BOX[size]} ${brand.tileClass} ${className}`}
    >
      {brand.icon}
    </span>
  );
};

/** The glyph for a job, in the same neutral chip, for when nothing is chosen. */
export const SlotGlyph: React.FC<{
  slot: ModelSlot;
  size?: MarkSize;
  className?: string;
}> = ({ slot, size = "md", className = "" }) => {
  const Icon = slot === "voice" ? AudioLines : SLOT_META[slot].icon;
  return (
    <span
      aria-hidden="true"
      className={`grid shrink-0 place-items-center bg-surface-strong text-muted ${BOX[size]} ${className}`}
    >
      <Icon size={16} strokeWidth={1.8} />
    </span>
  );
};

/**
 * The mark for whatever is doing a slot's job: the model family's mark for an
 * on-device model, the provider's for a hosted one, or the job's own glyph
 * when nothing is chosen yet. All three share one neutral chip.
 */
export const SlotLogo: React.FC<{
  summary: SlotSummary;
  size?: MarkSize;
  className?: string;
}> = ({ summary, size = "md", className = "" }) => {
  if (summary.localModel) {
    return (
      <ModelMark model={summary.localModel} size={size} className={className} />
    );
  }
  if (summary.providerId && summary.providerId !== "builtin") {
    return (
      <ProviderTile
        id={summary.providerId}
        kind={summary.providerKind}
        size={size}
        className={className}
      />
    );
  }
  return <SlotGlyph slot={summary.slot} size={size} className={className} />;
};

/** "On this computer" / "OpenRouter" — where the work happens, as plain text. */
export const WhereBadge: React.FC<{
  where: "device" | "cloud";
  label?: string | null;
  className?: string;
}> = ({ where, label, className = "" }) => {
  const { t } = useTranslation();
  const Icon = where === "device" ? Cpu : Cloud;
  const text =
    where === "device"
      ? t("modelsHub.where.device")
      : label || t("modelsHub.where.cloud");
  return (
    <span
      className={`inline-flex max-w-full items-center gap-1 text-xs text-muted ${className}`}
    >
      <Icon className="h-3 w-3 shrink-0" aria-hidden="true" />
      <span className="truncate">{text}</span>
    </span>
  );
};

/** One line of status copy for a slot: its model, or what is missing. */
export const useSlotStatusText = () => {
  const { t } = useTranslation();
  return (summary: SlotSummary): string => {
    if (summary.slot === "voice" && !summary.active) {
      return t("modelsHub.status.voiceOff");
    }
    if (summary.slot === "assistant" && !summary.active) {
      return t("modelsHub.status.assistantOff");
    }
    switch (summary.issue) {
      case "no_model":
        return t("modelsHub.status.noModel");
      case "not_downloaded":
        return t("modelsHub.status.notDownloaded");
      case "no_key":
        return t("modelsHub.status.noKey");
      default:
        return summary.modelLabel ?? summary.providerLabel ?? "";
    }
  };
};

/** "Off" / "Needs setup" / where it runs — whichever the user needs to know. */
export const SlotStateLine: React.FC<{ summary: SlotSummary }> = ({
  summary,
}) => {
  const { t } = useTranslation();
  if (!summary.active) {
    return <span className="text-xs text-muted">{t("common.off")}</span>;
  }
  if (!summary.ready) {
    return (
      <span className="inline-flex items-center gap-1.5 text-xs font-medium text-warning">
        <span className="h-1.5 w-1.5 rounded-full bg-warning" />
        {t("modelsHub.needsSetup")}
      </span>
    );
  }
  return (
    <WhereBadge
      where={summary.where}
      label={summary.where === "cloud" ? summary.providerLabel : null}
    />
  );
};
