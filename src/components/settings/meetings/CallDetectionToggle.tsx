import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { PhoneCall } from "lucide-react";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
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
 * that does nothing is worse than no switch, and the platform note says what to do
 * instead.
 */
export const CallDetectionToggle: React.FC = () => {
  const { t } = useTranslation();
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

  if (supported === null) return null;

  if (!supported) {
    return (
      <p className="px-1 text-xs text-muted-soft">
        {t("meetings.autoDetect.unsupported")}
      </p>
    );
  }

  return (
    <div className="rounded-2xl border border-hairline bg-surface elev-card px-4 py-1">
      <ToggleSwitch
        checked={enabled}
        onChange={(next) => {
          // Optimistic, then reverted on failure — a switch that shows one state
          // while the backend holds another is worse than a switch that snaps back.
          setEnabled(next);
          setSaving(true);
          void setMeetingAutoDetect(next)
            .catch(() => setEnabled(!next))
            .finally(() => setSaving(false));
        }}
        isUpdating={saving}
        label={t("meetings.autoDetect.title")}
        description={t("meetings.autoDetect.description")}
        descriptionMode="inline"
        icon={PhoneCall}
        tone="violet"
      />
    </div>
  );
};
