import React, { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Download, RotateCw } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { useNavigation } from "@/components/shell/navigation";
import { useModelSlots } from "@/components/shell/useModelSlots";
import { useSpeechUpdateReveal } from "@/components/shell/speechUpdate";
import { formatModelSize } from "@/lib/utils/format";
import { getTranslatedModelName } from "@/lib/utils/modelTranslation";
import { useReplaceRetired } from "./useReplaceRetired";
import { isReplacementBusy, type ReplacementView } from "./retiredReplacement";

/** The Models page's catalog, which "Choose another model" scrolls to there. */
export const SPEECH_CATALOG_ATTRIBUTE = "data-speech-catalog";

/**
 * Shown when the selected speech model is retired: its engine is gone, so
 * dictation cannot run until the user moves to its new version or another
 * model. At the top of Home and of the Models page's speech tab, because a
 * user who updated the app and finds dictation refused should not have to go
 * looking for why.
 *
 * One sentence saying what happened, one primary action that fixes it, and a
 * way out to the rest of the catalog. The download runs in place, so the
 * prompt is also where its progress and any failure are.
 */
export const RetiredSelectionPrompt: React.FC<{
  /** Home sends "Choose another model" to the Models page; on the Models
   *  page it scrolls to the catalog below. */
  placement: "home" | "models";
  className?: string;
}> = ({ placement, className = "" }) => {
  const { t } = useTranslation();
  const { openModelSlot } = useNavigation();
  const stt = useModelSlots().stt;
  const { replace, viewFor, cancel } = useReplaceRetired();
  const sectionRef = useRef<HTMLElement>(null);
  const requested = useSpeechUpdateReveal((state) => state.requested);

  const retired = stt.issue === "needs_update" ? stt.localModel : null;

  // The backend opened this tab because a dictation was refused: make sure
  // the prompt is what the user sees, wherever the page was scrolled to.
  useEffect(() => {
    if (placement !== "models" || !retired) return;
    if (!useSpeechUpdateReveal.getState().answer()) return;
    const frame = window.requestAnimationFrame(() =>
      sectionRef.current?.scrollIntoView({
        behavior: "smooth",
        block: "start",
      }),
    );
    return () => window.cancelAnimationFrame(frame);
  }, [placement, retired, requested]);

  if (!retired) return null;

  const view = viewFor(retired);
  const modelName = stt.modelLabel ?? getTranslatedModelName(retired, t);

  const chooseAnother = () => {
    if (placement === "home") {
      openModelSlot("stt");
      return;
    }
    document
      .querySelector(`[${SPEECH_CATALOG_ATTRIBUTE}]`)
      ?.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  return (
    <section
      ref={sectionRef}
      aria-labelledby="retired-selection-title"
      className={`scroll-mt-6 rounded-2xl border border-warning/25 bg-warning/[0.06] px-5 py-4 ${className}`}
    >
      <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
        <span
          className="h-2 w-2 shrink-0 rounded-full bg-warning"
          aria-hidden="true"
        />
        <div className="min-w-0 flex-1 basis-[16rem]">
          <p
            id="retired-selection-title"
            className="text-sm font-medium text-ink"
          >
            {t("speechUpdate.title", { model: modelName })}
          </p>
          <Detail view={view} />
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Actions
            view={view}
            onReplace={() => void replace(retired)}
            onCancel={(id) => void cancel(id)}
          />
          <Button variant="secondary" size="sm" onClick={chooseAnother}>
            {t("speechUpdate.chooseAnother")}
            {placement === "home" && (
              <ArrowRight
                className="h-3.5 w-3.5 rtl:rotate-180"
                aria-hidden="true"
              />
            )}
          </Button>
        </div>
      </div>
      {view && isReplacementBusy(view.phase) && <Progress view={view} />}
    </section>
  );
};

/**
 * The pop-up someone sees after updating from a version whose engine ran
 * their speech model. The prompt above sits on Home and the Models page; this
 * says the same thing the moment the app opens, so nobody first finds out by
 * pressing the dictation keys and getting nothing.
 *
 * Nothing downloads until they choose to: the new version is a few hundred
 * megabytes (Cohere's is 1.8 GB), and some people are on a metered connection.
 * "Download new version" starts it here, with its progress here, and the
 * model is switched to when it lands, which also closes the pop-up. "Choose
 * another model" goes to the catalog, and "Later" closes it until the next
 * launch, or until a dictation is refused for the same reason.
 */
export const RetiredSelectionDialog: React.FC = () => {
  const { t } = useTranslation();
  const { openModelSlot } = useNavigation();
  const stt = useModelSlots().stt;
  const { replace, viewFor, cancel } = useReplaceRetired();
  const closed = useSpeechUpdateReveal((state) => state.dialogClosed);
  const close = useSpeechUpdateReveal((state) => state.closeDialog);

  const retired = stt.issue === "needs_update" ? stt.localModel : null;
  if (!retired) return null;

  const view = viewFor(retired);
  const modelName = stt.modelLabel ?? getTranslatedModelName(retired, t);

  return (
    <Dialog
      open={!closed}
      onClose={close}
      size="md"
      title={t("speechUpdate.dialogTitle")}
      description={t("speechUpdate.dialogBody", { model: modelName })}
      closeOnScrim={false}
      footer={
        <div className="flex w-full flex-wrap items-center justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={close}>
            {t("speechUpdate.later")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => {
              close();
              openModelSlot("stt");
            }}
          >
            {t("speechUpdate.chooseAnother")}
          </Button>
          <Actions
            view={view}
            onReplace={() => void replace(retired)}
            onCancel={(id) => void cancel(id)}
          />
        </div>
      }
    >
      <Detail view={view} />
      {view && isReplacementBusy(view.phase) && (
        <Progress view={view} inset={false} />
      )}
    </Dialog>
  );
};

/** The line under the sentence: what the way forward is, or how it went. */
const Detail: React.FC<{ view: ReplacementView | null }> = ({ view }) => {
  const { t } = useTranslation();
  if (!view) {
    return (
      <p className="mt-0.5 text-xs text-muted">{t("speechUpdate.noVersion")}</p>
    );
  }
  const name = getTranslatedModelName(view.model, t);
  if (view.phase === "failed") {
    return (
      <p
        role="alert"
        className="mt-0.5 truncate text-xs text-error"
        title={view.error}
      >
        {t("speechUpdate.failed", { model: name })}
      </p>
    );
  }
  return (
    <p className="mt-0.5 truncate text-xs text-muted">
      {view.phase === "use"
        ? t("speechUpdate.ready", { model: name })
        : t("speechUpdate.newVersion", { model: name })}
    </p>
  );
};

/** The primary action for where the new version stands. */
const Actions: React.FC<{
  view: ReplacementView | null;
  onReplace: () => void;
  onCancel: (replacementId: string) => void;
}> = ({ view, onReplace, onCancel }) => {
  const { t } = useTranslation();
  if (!view) return null;
  const size = formatModelSize(Number(view.model.size_mb));
  switch (view.phase) {
    case "downloading":
      return (
        <Button
          variant="ghost"
          size="sm"
          onClick={() => onCancel(view.model.id)}
        >
          {t("common.cancel")}
        </Button>
      );
    case "verifying":
    case "extracting":
      return null;
    case "use":
    case "switching":
      return (
        <Button
          size="sm"
          onClick={onReplace}
          disabled={view.phase === "switching"}
        >
          {t("settings.models.retired.useNewVersion")}
        </Button>
      );
    case "failed":
      return (
        <Button size="sm" onClick={onReplace}>
          <RotateCw className="h-3.5 w-3.5" aria-hidden="true" />
          {t("speechUpdate.retry")}
        </Button>
      );
    case "download":
      return (
        <Button size="sm" onClick={onReplace}>
          <Download className="h-3.5 w-3.5" aria-hidden="true" />
          {t("speechUpdate.download", { size })}
        </Button>
      );
  }
};

/** The new version's download, in place. Same bar as a catalog row's. */
const Progress: React.FC<{
  view: ReplacementView;
  /** Line up under the prompt's text, past its dot. */
  inset?: boolean;
}> = ({ view, inset = true }) => {
  const { t } = useTranslation();
  const percent = Math.max(0, Math.min(100, view.percent ?? 0));
  const downloading = view.phase === "downloading";
  return (
    <div className={inset ? "mt-3 ps-6" : "mt-3"}>
      <div
        className="h-1.5 w-full overflow-hidden rounded-full bg-surface-strong"
        role="progressbar"
        aria-label={t("speechUpdate.progressLabel")}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={downloading ? Math.round(percent) : undefined}
      >
        <div
          className={`h-full rounded-full bg-accent ${
            downloading
              ? "transition-[width] duration-300"
              : "w-full animate-pulse"
          }`}
          style={downloading ? { width: `${percent}%` } : undefined}
        />
      </div>
      <div className="mt-1.5 flex items-center justify-between gap-3 text-xs text-muted">
        <span>
          {downloading
            ? t("modelSelector.downloading", {
                percentage: Math.round(percent),
              })
            : view.phase === "verifying"
              ? t("modelSelector.verifyingGeneric")
              : t("modelSelector.extractingGeneric")}
        </span>
        {downloading && view.speed !== undefined && view.speed > 0 && (
          <span className="tabular-nums">
            {t("modelSelector.downloadSpeed", {
              speed: view.speed.toFixed(1),
            })}
          </span>
        )}
      </div>
    </div>
  );
};
