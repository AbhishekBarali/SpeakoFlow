import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "@/components/ui/ToggleSwitch";
import {
  getCallDetectionStatus,
  getMeetingIndicator,
  setMeetingAutoDetect,
  setMeetingIndicator,
} from "./api";

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

/** "Offer to record calls", as a row in a settings group. */
export const CallDetectionToggle: React.FC = () => {
  const { t } = useTranslation();
  const { supported, enabled, saving, set } = useCallDetection();
  if (!supported) return null;
  return (
    <ToggleSwitch
      checked={enabled}
      onChange={set}
      isUpdating={saving}
      label={t("meetings.autoDetect.title")}
      description={t("meetings.autoDetect.description")}
      grouped={true}
    />
  );
};

/**
 * "Show a floating indicator while recording."
 *
 * Read through `invoke` for the same reason as call detection above. Applies to
 * a meeting already recording too — the backend shows or hides the pill on the
 * spot — so every surface that offers the switch shares this one hook and
 * cannot disagree about its state.
 */
export const useMeetingIndicator = () => {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void getMeetingIndicator()
      .then(setEnabled)
      // An older backend without the command: the indicator is on there.
      .catch(() => setEnabled(true));
  }, []);

  const set = (next: boolean) => {
    setEnabled(next);
    setSaving(true);
    void setMeetingIndicator(next)
      .catch(() => setEnabled(!next))
      .finally(() => setSaving(false));
  };

  return { enabled, saving, set };
};

/** "Floating indicator", as a row in a settings group. */
export const IndicatorToggle: React.FC = () => {
  const { t } = useTranslation();
  const { enabled, saving, set } = useMeetingIndicator();
  if (enabled === null) return null;
  return (
    <ToggleSwitch
      checked={enabled}
      onChange={set}
      isUpdating={saving}
      label={t("meetings.indicator.title")}
      description={t("meetings.indicator.description")}
      grouped={true}
    />
  );
};
