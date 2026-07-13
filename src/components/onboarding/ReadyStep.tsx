import React, { useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Mic, MessageCircle } from "lucide-react";
import { platform } from "@tauri-apps/plugin-os";
import { commands, type ModelInfo } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { formatKeyCombination, type OSType } from "@/lib/utils/keyboard";
import OnboardingLayout from "./OnboardingLayout";
import { Button } from "../ui/Button";
import { TONE_TILE_VIVID, type SettingTone } from "../ui/tones";
import { useModelStore } from "../../stores/modelStore";

interface ReadyStepProps {
  /** Enter the main app. */
  onComplete: () => void;
}

/** Decorative separator between keycaps (kept as a constant so it isn't flagged
 *  as a translatable literal in JSX markup). */
const KEY_SEPARATOR = "+";

const resolveOsType = (): OSType => {
  try {
    const p = platform();
    if (p === "macos") return "macos";
    if (p === "windows") return "windows";
    if (p === "linux") return "linux";
  } catch {
    /* platform() can throw outside Tauri; fall through */
  }
  return "unknown";
};

/** One shortcut showcase: icon tile + what it does + the actual keycaps. */
const ShortcutCard: React.FC<{
  icon: React.ReactNode;
  tone: SettingTone;
  label: string;
  caption: string;
  parts: string[];
}> = ({ icon, tone, label, caption, parts }) => (
  <div className="flex items-center gap-4 rounded-2xl border border-hairline bg-surface elev-card px-5 py-4">
    <span
      className={`flex h-11 w-11 shrink-0 items-center justify-center rounded-xl ${TONE_TILE_VIVID[tone]}`}
    >
      {icon}
    </span>
    <div className="min-w-0 flex-1">
      <p className="text-[14.5px] font-semibold text-ink">{label}</p>
      <p className="mt-0.5 text-[12.5px] leading-snug text-muted">{caption}</p>
    </div>
    <div className="flex items-center gap-1.5 shrink-0">
      {parts.map((part, i) => (
        <React.Fragment key={`${part}-${i}`}>
          {i > 0 && (
            <span aria-hidden className="text-muted-soft text-sm">
              {KEY_SEPARATOR}
            </span>
          )}
          <kbd className="px-3 py-2 rounded-lg border border-hairline-strong bg-surface-strong text-ink text-[13px] font-semibold not-italic elev-chip whitespace-nowrap">
            {part}
          </kbd>
        </React.Fragment>
      ))}
    </div>
  </div>
);

/**
 * Step 3 of the welcome flow: "You're ready."
 *
 * Shows the two shortcuts that matter — dictation and the assistant — as real
 * keycaps with a one-line explanation each, plus warm status lines for anything
 * still downloading in the background. No live try-it here: the voice model may
 * still be on its way down, so the promise is the shortcuts, not an instant
 * demo. Enigo + global shortcuts are initialized on mount so the keys already
 * work if the model is ready.
 */
const ReadyStep: React.FC<ReadyStepProps> = ({ onComplete }) => {
  const { t } = useTranslation();
  const { settings } = useSettings();

  const models = useModelStore((s) => s.models);
  const downloadingModels = useModelStore((s) => s.downloadingModels);
  const verifyingModels = useModelStore((s) => s.verifyingModels);
  const extractingModels = useModelStore((s) => s.extractingModels);
  const downloadProgress = useModelStore((s) => s.downloadProgress);

  // Make the hotkeys live immediately. This mirrors the init the main app runs
  // on "done"; calling it here too is safe.
  useEffect(() => {
    Promise.all([
      commands.initializeEnigo(),
      commands.initializeShortcuts(),
    ]).catch((e) => {
      console.warn("Failed to initialize shortcuts:", e);
    });
  }, []);

  const osType = useMemo(resolveOsType, []);
  const transcribeBinding =
    settings?.bindings?.transcribe?.current_binding ?? "";
  const assistantBinding = settings?.bindings?.assistant?.current_binding ?? "";

  const dictateKeys = useMemo(() => {
    const formatted = formatKeyCombination(transcribeBinding, osType);
    return formatted ? formatted.split(" + ") : [];
  }, [transcribeBinding, osType]);
  const assistantKeys = useMemo(() => {
    const formatted = formatKeyCombination(assistantBinding, osType);
    return formatted ? formatted.split(" + ") : [];
  }, [assistantBinding, osType]);

  const activeIds = useMemo(
    () =>
      Array.from(
        new Set([
          ...Object.keys(downloadingModels),
          ...Object.keys(verifyingModels),
          ...Object.keys(extractingModels),
        ]),
      ),
    [downloadingModels, verifyingModels, extractingModels],
  );

  const statusLineFor = (id: string): string => {
    const model = models.find((m: ModelInfo) => m.id === id);
    const pct = Math.max(
      0,
      Math.min(100, Math.round(downloadProgress[id]?.percentage ?? 0)),
    );
    const category = model ? getModelCategory(model) : "stt";
    if (category === "llm") {
      return t("onboarding.ready.downloadingAssistant", { percentage: pct });
    }
    if (category === "stt") {
      return t("onboarding.ready.downloadingVoice", { percentage: pct });
    }
    return t("onboarding.ready.downloadingGeneric", { percentage: pct });
  };

  const footer = (
    <>
      <p className="text-xs text-muted-soft max-w-[55%]">
        {t("onboarding.ready.signOff")}
      </p>
      <Button variant="primary" size="lg" onClick={onComplete}>
        {t("onboarding.ready.openApp")}
      </Button>
    </>
  );

  return (
    <OnboardingLayout
      step={3}
      totalSteps={3}
      title={t("onboarding.ready.title")}
      subtitle={t("onboarding.ready.subtitle")}
      footer={footer}
      showDownloadProgress={false}
    >
      <div className="flex flex-col items-center gap-5 w-full pt-3">
        <div className="w-full max-w-[520px] flex flex-col gap-3">
          {dictateKeys.length > 0 && (
            <div className="anim-rise anim-delay-1">
              <ShortcutCard
                icon={<Mic size={19} />}
                tone="teal"
                label={t("onboarding.ready.dictateLabel")}
                caption={t("onboarding.ready.dictateCaption")}
                parts={dictateKeys}
              />
            </div>
          )}
          {assistantKeys.length > 0 && (
            <div className="anim-rise anim-delay-2">
              <ShortcutCard
                icon={<MessageCircle size={19} />}
                tone="violet"
                label={t("onboarding.ready.assistantKeysLabel")}
                caption={t("onboarding.ready.assistantKeysCaption")}
                parts={assistantKeys}
              />
            </div>
          )}
          <p className="anim-rise anim-delay-3 pt-1 text-center text-xs text-muted-soft">
            {t("onboarding.ready.changeAnytime")}
          </p>
        </div>

        {/* Warm status for anything still downloading in the background. */}
        {activeIds.length > 0 && (
          <div
            className="anim-rise anim-delay-4 w-full max-w-[520px] flex flex-col gap-2"
            role="status"
            aria-live="polite"
          >
            {activeIds.map((id) => {
              const pct = Math.max(
                0,
                Math.min(
                  100,
                  Math.round(downloadProgress[id]?.percentage ?? 0),
                ),
              );
              return (
                <div
                  key={id}
                  className="w-full rounded-lg border border-hairline bg-surface px-3 py-2"
                >
                  <p className="flex items-center gap-1.5 text-xs text-text/70">
                    <Loader2 className="w-3.5 h-3.5 animate-spin text-accent shrink-0" />
                    <span>{statusLineFor(id)}</span>
                  </p>
                  <div className="mt-1.5 w-full h-1.5 bg-mid-gray/20 rounded-full overflow-hidden">
                    <div
                      className="h-full bg-logo-primary rounded-full transition-all duration-300"
                      style={{ width: `${pct}%` }}
                    />
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </OnboardingLayout>
  );
};

export default ReadyStep;
