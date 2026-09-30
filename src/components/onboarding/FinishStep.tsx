import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Check, RotateCcw } from "lucide-react";
import { commands } from "@/bindings";
import { Button } from "@/components/ui/Button";
import { ShortcutInput } from "@/components/settings/ShortcutInput";
import { getModelBrand } from "@/components/icons/BrandLogos";
import { useSettings } from "@/hooks/useSettings";
import { useModelStore } from "@/stores/modelStore";
import { formatBytes } from "@/lib/utils/format";
import { OnboardingFrame, StepHeading } from "./OnboardingFrame";
import { useSetupQueue, useTaskProgress, type SetupTask } from "./setupQueue";

type SpeechState = "ready" | "downloading" | "failed" | "none";

/** Where dictation stands: usable now, on its way, stopped, or never chosen. */
function useSpeechState(): { state: SpeechState; task: SetupTask | undefined } {
  const task = useSetupQueue((s) =>
    s.tasks.find((entry) => entry.job === "stt"),
  );
  const installed = useModelStore((s) =>
    s.models.some(
      (model) => model.id === s.currentModel && model.is_downloaded,
    ),
  );
  const cloud = useSettings().settings?.stt_engine_mode === "cloud";
  if (task?.state === "ready") return { state: "ready", task };
  if (task?.state === "failed" || task?.state === "cancelled")
    return { state: "failed", task };
  if (task) return { state: "downloading", task };
  // No download from this setup: whatever the machine already had decides, so
  // "Set up later" on a machine with a working model is still ready to go.
  return { state: installed || cloud ? "ready" : "none", task };
}

/**
 * The speech model arriving, as the whole of the screen: its mark, a large
 * percentage and a bar. A locked text box beside a small bar read as the app
 * being broken; this reads as the app working.
 */
const DownloadPanel: React.FC<{ task: SetupTask; failed: boolean }> = ({
  task,
  failed,
}) => {
  const { t } = useTranslation();
  const percent = useTaskProgress(task);
  const model = useModelStore((s) =>
    s.models.find((m) => m.id === task.modelId),
  );
  const bytes = useModelStore((s) =>
    task.modelId ? s.downloadProgress[task.modelId] : undefined,
  );
  const speed = useModelStore((s) =>
    task.modelId ? s.downloadStats[task.modelId]?.speed : undefined,
  );
  const settling = useModelStore(
    (s) =>
      !!task.modelId &&
      (task.modelId in s.verifyingModels || task.modelId in s.extractingModels),
  );
  const cancel = useSetupQueue((s) => s.cancel);
  const brand = model ? getModelBrand(model) : null;
  const installing = settling || task.state === "switching";
  const remaining =
    bytes && speed && speed > 0 && bytes.total > bytes.downloaded
      ? (bytes.total - bytes.downloaded) / (speed * 1024 * 1024)
      : null;

  const detail = failed
    ? null
    : installing
      ? t("onboarding.finish.status.installing")
      : task.state === "waiting"
        ? t("onboarding.finish.status.waiting")
        : [
            bytes && bytes.total > 0
              ? t("onboarding.start.progress.of", {
                  done: formatBytes(bytes.downloaded),
                  total: formatBytes(bytes.total),
                })
              : null,
            remaining === null
              ? null
              : remaining < 60
                ? t("onboarding.start.progress.underMinute")
                : t("onboarding.start.progress.minutes", {
                    count: Math.ceil(remaining / 60),
                  }),
          ]
            .filter(Boolean)
            .join(" · ");

  return (
    <section className="mt-8 rounded-2xl border border-hairline bg-surface px-7 py-7 elev-card">
      <div className="flex items-center gap-4">
        {brand && (
          <span
            aria-hidden="true"
            className={`grid h-12 w-12 shrink-0 place-items-center overflow-hidden rounded-xl [&_svg]:h-7 [&_svg]:w-7 ${brand.tileClass}`}
          >
            {brand.icon}
          </span>
        )}
        <span className="min-w-0 flex-1 truncate text-lg font-semibold text-ink">
          {task.label}
        </span>
        {!failed && (
          <span
            className="font-display text-[2.5rem] leading-none text-ink tabular-nums"
            aria-hidden="true"
          >
            {installing ? "" : `${percent}%`}
          </span>
        )}
      </div>

      {failed ? (
        <div className="mt-6 flex flex-wrap items-center justify-between gap-3">
          <p className="text-sm text-error" role="alert">
            {t("onboarding.start.progress.failed")}
          </p>
          <Button
            variant="secondary"
            onClick={() => useSetupQueue.getState().start([task])}
          >
            <RotateCcw className="h-4 w-4" aria-hidden="true" />
            {t("onboarding.finish.status.retry")}
          </Button>
        </div>
      ) : (
        <>
          <div
            className="mt-6 h-2 overflow-hidden rounded-full bg-surface-strong"
            role="progressbar"
            aria-label={task.label}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={installing ? undefined : percent}
          >
            <span
              className={`block h-full rounded-full bg-accent-fill transition-[width] duration-500 ${installing ? "ob-indeterminate" : ""}`}
              style={installing ? undefined : { width: `${percent}%` }}
            />
          </div>
          <div className="mt-3 flex items-center justify-between gap-3">
            <span className="text-[0.8125rem] text-muted tabular-nums">
              {detail}
            </span>
            {(task.state === "downloading" || task.state === "waiting") &&
              !installing && (
                <button
                  type="button"
                  onClick={() => void cancel("stt")}
                  className="cursor-pointer rounded-md px-2 py-1 text-[0.8125rem] font-medium text-muted transition-colors hover:bg-ink/[0.05] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
                >
                  {t("common.cancel")}
                </button>
              )}
          </div>
        </>
      )}
    </section>
  );
};

/** The first dictation: the keys, and a box for the words to land in. */
const TryPanel: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const hold = settings?.push_to_talk ?? true;
  const [text, setText] = useState("");
  const field = useRef<HTMLTextAreaElement>(null);

  // Put the caret in the box, so the very next press of the shortcut has
  // somewhere to type — the dictation arrives through `insertDictation`.
  useEffect(() => {
    field.current?.focus({ preventScroll: true });
  }, []);

  return (
    <section className="ob-reveal mt-8">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <ShortcutInput
          shortcutId="transcribe"
          bare
          size="lg"
          showReset="never"
        />
        <span className="text-sm text-muted">
          {hold
            ? t("onboarding.tour.keys.hold")
            : t("onboarding.tour.keys.tap")}
        </span>
      </div>
      <textarea
        ref={field}
        rows={4}
        value={text}
        onChange={(event) => setText(event.target.value)}
        aria-label={t("onboarding.start.try.label")}
        placeholder={t("onboarding.start.try.placeholder")}
        className="mt-5 block w-full resize-none rounded-2xl border border-hairline-strong bg-surface px-5 py-4 text-lg leading-relaxed text-ink select-text elev-card placeholder:text-muted-soft focus:border-accent focus:outline-none focus:ring-4 focus:ring-accent/15"
      />
      <p
        className={`mt-3 flex h-5 items-center gap-1.5 text-sm font-medium text-accent transition-opacity duration-300 ${text.trim() ? "opacity-100" : "opacity-0"}`}
        role="status"
      >
        {text.trim() && (
          <>
            <Check className="h-4 w-4" aria-hidden="true" />
            {t("onboarding.start.try.success")}
          </>
        )}
      </p>
    </section>
  );
};

/**
 * The end of setup. One thing on screen at a time: the speech model arriving
 * (big, so it is obvious the app is working), then — the moment it lands — a
 * box to dictate into. The backend refuses a voice shortcut pressed before the
 * model is ready and says why on the overlay (`speech_readiness.rs`).
 */
export function FinishStep({
  onDone,
}: {
  /** `chooseModels`: nothing can dictate yet, so open the app on Models. */
  onDone: (chooseModels?: boolean) => void;
}) {
  const { t } = useTranslation();
  const { state, task } = useSpeechState();
  const chooseModels = state === "none" || state === "failed";

  useEffect(() => {
    Promise.all([
      commands.initializeEnigo(),
      commands.initializeShortcuts(),
    ]).catch((error) => console.warn("Failed to initialize shortcuts:", error));
  }, []);

  return (
    <OnboardingFrame
      step="finish"
      footer={
        <>
          <span />
          <Button size="lg" onClick={() => onDone(chooseModels)}>
            {t(
              chooseModels
                ? "onboarding.start.choose"
                : "onboarding.start.open",
            )}
            <ArrowRight className="h-4 w-4 rtl:rotate-180" aria-hidden="true" />
          </Button>
        </>
      }
    >
      <StepHeading live title={t(`onboarding.start.${state}.title`)} />
      {state === "ready" ? (
        <TryPanel />
      ) : task && (state === "downloading" || state === "failed") ? (
        <DownloadPanel task={task} failed={state === "failed"} />
      ) : null}
    </OnboardingFrame>
  );
}

export default FinishStep;
