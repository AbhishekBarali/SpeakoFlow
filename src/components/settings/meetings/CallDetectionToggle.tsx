import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
import { Switch } from "@/components/ui/Switch";
import { InfoTip } from "@/components/ui/InfoTip";
import { getCallDetectionStatus, setMeetingAutoDetect } from "./api";

/**
 * "Offer to record calls."
 *
 * Read and written through `invoke` rather than `useSettings`, matching the rest of
 * the meetings surface. `updateSetting` is an allowlist keyed on the generated
 * `AppSettings` type, and `bindings.ts` is only regenerated while the app runs in
 * dev — so routing a newly added key through it would type-check on this machine and
 * break a fresh checkout.
 *
 * Hidden entirely where detection cannot work, rather than shown disabled: a switch
 * that does nothing is worse than no switch.
 */
const useCallDetection = () => {
  const [supported, setSupported] = useState<boolean | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void getCallDetectionStatus()
      .then((status) => {
        setSupported(status.supported);
        setEnabled(status.enabled);
      })
      .catch(() => setSupported(false));
  }, []);

  const set = (next: boolean) => {
    // Optimistic, then reverted on failure — a switch that shows one state
    // while the backend holds another is worse than one that snaps back.
    setEnabled(next);
    setSaving(true);
    void setMeetingAutoDetect(next)
      .catch(() => setEnabled(!next))
      .finally(() => setSaving(false));
  };

  return { supported, enabled, saving, set };
};

/** As a settings row. */
export const CallDetectionToggle: React.FC = () => {
  const { t } = useTranslation();
  const { supported, enabled, saving, set } = useCallDetection();
  if (!supported) return null;
  return (
    <div className="rounded-2xl border border-hairline bg-surface elev-card">
      <ToggleSwitch
        checked={enabled}
        onChange={set}
        isUpdating={saving}
        label={t("meetings.autoDetect.title")}
        description={t("meetings.autoDetect.description")}
        grouped={true}
      />
    </div>
  );
};

/** As a line on the Meetings hero. */
export const CallDetectionHeroSwitch: React.FC = () => {
  const { t } = useTranslation();
  const { supported, enabled, saving, set } = useCallDetection();
  if (!supported) return null;
  return (
    <div className="flex items-center gap-2.5">
      <Switch
        checked={enabled}
        onChange={set}
        disabled={saving}
        label={t("meetings.autoDetect.title")}
        tone="onHero"
      />
      <span className="text-sm text-white/85">
        {t("meetings.autoDetect.title")}
      </span>
      <InfoTip text={t("meetings.autoDetect.description")} tone="onHero" />
    </div>
  );
};
