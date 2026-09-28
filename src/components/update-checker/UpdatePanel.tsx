import React from "react";
import { useTranslation } from "react-i18next";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { type } from "@tauri-apps/plugin-os";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  CircleAlert,
  CircleCheck,
  Download,
  ExternalLink,
  FolderOpen,
  LoaderCircle,
  RotateCw,
} from "lucide-react";
import { Button } from "../ui/Button";
import { Alert } from "../ui/Alert";
import { useUpdateStore } from "./updateStore";
import { safeExternalHref } from "./updateLogic";

/**
 * Settings → About's side of the update flow: the button and one-line status
 * in the app card (`UpdateHeaderAction`, `UpdateStatusLine`), and the card that
 * appears under it once there is something to install (`UpdateDetails`).
 */

export const UpdateHeaderAction: React.FC = () => {
  const { t } = useTranslation();
  const phase = useUpdateStore((s) => s.phase);
  const checkNow = useUpdateStore((s) => s.check);
  const checking = phase === "checking";
  // Once there is an update the card below owns the actions.
  if (!["idle", "checking", "upToDate"].includes(phase)) return null;
  return (
    <Button
      variant="secondary"
      size="sm"
      disabled={checking}
      onClick={() => void checkNow({ manual: true })}
    >
      {checking && (
        <LoaderCircle
          className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none"
          aria-hidden="true"
        />
      )}
      {checking ? t("updates.checking") : t("updates.check")}
    </Button>
  );
};

export const UpdateStatusLine: React.FC = () => {
  const { t } = useTranslation();
  const phase = useUpdateStore((s) => s.phase);
  const error = useUpdateStore((s) => s.error);

  if (error && (error.kind === "check" || error.kind === "noBuild")) {
    return (
      <p
        role="status"
        className="mt-1 flex items-center gap-1.5 text-[0.8125rem] text-error"
      >
        <CircleAlert className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        {error.kind === "noBuild"
          ? t("updates.noBuild")
          : t("updates.checkFailed")}
      </p>
    );
  }
  if (phase === "upToDate") {
    return (
      <p
        role="status"
        className="mt-1 flex items-center gap-1.5 text-[0.8125rem] text-muted"
      >
        <CircleCheck
          className="h-3.5 w-3.5 shrink-0 text-success"
          aria-hidden="true"
        />
        {t("updates.upToDate")}
      </p>
    );
  }
  return null;
};

const NOTES_COMPONENTS = {
  a: ({ href, children }: { href?: string; children?: React.ReactNode }) => {
    const safe = safeExternalHref(href);
    if (!safe) return <span>{children}</span>;
    return (
      <a
        href={safe}
        onClick={(event) => {
          // Release notes must never navigate the app's own window.
          event.preventDefault();
          void openUrl(safe);
        }}
        className="text-accent-strong underline decoration-accent/40 underline-offset-2 hover:decoration-accent"
      >
        {children}
      </a>
    );
  },
  h1: ({ children }: { children?: React.ReactNode }) => (
    <p className="mt-3 mb-1 font-semibold text-ink first:mt-0">{children}</p>
  ),
  h2: ({ children }: { children?: React.ReactNode }) => (
    <p className="mt-3 mb-1 font-semibold text-ink first:mt-0">{children}</p>
  ),
  h3: ({ children }: { children?: React.ReactNode }) => (
    <p className="mt-3 mb-1 font-medium text-ink first:mt-0">{children}</p>
  ),
  p: ({ children }: { children?: React.ReactNode }) => (
    <p className="mb-2 last:mb-0">{children}</p>
  ),
  ul: ({ children }: { children?: React.ReactNode }) => (
    <ul className="mb-2 list-disc space-y-1 ps-5 last:mb-0">{children}</ul>
  ),
  ol: ({ children }: { children?: React.ReactNode }) => (
    <ol className="mb-2 list-decimal space-y-1 ps-5 last:mb-0">{children}</ol>
  ),
  code: ({ children }: { children?: React.ReactNode }) => (
    <code className="rounded bg-ink/[0.06] px-1 py-0.5 font-mono text-[0.8em]">
      {children}
    </code>
  ),
  img: () => null,
};

function formatDate(value: string | null, language: string): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return date.toLocaleDateString(language, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export const UpdateDetails: React.FC = () => {
  const { t, i18n } = useTranslation();
  const {
    phase,
    available,
    progress,
    error,
    mode,
    releasePage,
    install,
    downloadInstaller,
    openInstaller,
    revealInstaller,
    restart,
  } = useUpdateStore();

  const visible =
    available !== null ||
    phase === "savingInstaller" ||
    phase === "installerSaved" ||
    error?.kind === "download";
  if (!visible) return null;

  const os = type();
  const busy =
    phase === "downloading" ||
    phase === "installing" ||
    phase === "savingInstaller";
  const date = formatDate(available?.date ?? null, i18n.language);

  const progressLabel =
    phase === "installing"
      ? t("updates.installing")
      : progress !== null
        ? t("updates.downloading", { progress })
        : t("updates.downloadingUnknown");

  const hint =
    mode === "portable"
      ? t("updates.modes.portable")
      : mode === "package_manager"
        ? t("updates.modes.packageManager")
        : mode === "download"
          ? t("updates.modes.download")
          : os === "windows"
            ? t("updates.hint.windows")
            : os === "macos"
              ? t("updates.hint.macos")
              : t("updates.hint.linux");

  const primary = (() => {
    if (phase === "restartRequired") {
      return (
        <Button variant="primary" size="sm" onClick={() => void restart()}>
          <RotateCw className="h-3.5 w-3.5" aria-hidden="true" />
          {t("updates.restart")}
        </Button>
      );
    }
    if (phase === "installerSaved") {
      return (
        <>
          <Button
            variant="primary"
            size="sm"
            onClick={() => void openInstaller()}
          >
            {t("updates.openInstaller")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void revealInstaller()}
          >
            <FolderOpen className="h-3.5 w-3.5" aria-hidden="true" />
            {t("updates.showInFolder")}
          </Button>
        </>
      );
    }
    if (mode === "package_manager") return null;
    if (mode === "portable") {
      return (
        <Button
          variant="primary"
          size="sm"
          onClick={() => void openUrl(releasePage)}
        >
          <Download className="h-3.5 w-3.5" aria-hidden="true" />
          {t("updates.downloadNewVersion")}
        </Button>
      );
    }
    if (mode === "download") {
      return (
        <Button
          variant="primary"
          size="sm"
          disabled={busy}
          onClick={() => void downloadInstaller()}
        >
          <Download className="h-3.5 w-3.5" aria-hidden="true" />
          {t("updates.downloadInstaller")}
        </Button>
      );
    }
    return (
      <Button
        variant="primary"
        size="sm"
        disabled={busy}
        onClick={() => void install()}
      >
        {t("updates.installNow")}
      </Button>
    );
  })();

  return (
    <section
      aria-labelledby="update-details-title"
      className="overflow-hidden rounded-xl border border-accent/25 bg-surface elev-card"
    >
      <div className="px-5 pt-4 pb-3">
        <div className="flex items-baseline justify-between gap-3">
          <h4
            id="update-details-title"
            className="text-[0.9375rem] font-semibold text-ink"
          >
            {phase === "installerSaved"
              ? t("updates.installerSaved")
              : available
                ? t("updates.available", { version: available.version })
                : t("updates.availableShort")}
          </h4>
          {date && <span className="shrink-0 text-xs text-muted">{date}</span>}
        </div>

        {available?.notes && phase !== "installerSaved" && (
          <div className="mt-2 max-h-52 overflow-y-auto pe-1 text-[0.8125rem] leading-relaxed text-body">
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              components={NOTES_COMPONENTS}
            >
              {available.notes}
            </ReactMarkdown>
          </div>
        )}
      </div>

      <div className="space-y-3 border-t border-hairline bg-canvas/50 px-5 py-3">
        {busy ? (
          <div role="status" aria-live="polite">
            <div className="mb-1.5 flex items-center gap-2 text-[0.8125rem] text-body">
              <LoaderCircle
                className="h-3.5 w-3.5 animate-spin text-accent motion-reduce:animate-none"
                aria-hidden="true"
              />
              {progressLabel}
            </div>
            <div className="h-1.5 overflow-hidden rounded-full bg-ink/[0.07]">
              <div
                className={`h-full rounded-full bg-accent transition-[width] duration-300 ${progress === null ? "w-1/3 animate-pulse" : ""}`}
                style={
                  progress === null ? undefined : { width: `${progress}%` }
                }
              />
            </div>
          </div>
        ) : (
          <>
            {phase === "restartRequired" ? (
              <p className="text-[0.8125rem] text-body">
                {t("updates.readyToRestart")}
              </p>
            ) : phase !== "installerSaved" ? (
              <p className="text-[0.8125rem] text-muted">{hint}</p>
            ) : null}

            {error &&
              (error.kind === "install" || error.kind === "download") && (
                <Alert variant="error" className="!p-3">
                  {error.kind === "install"
                    ? t("updates.installFailed")
                    : t("updates.downloadFailed")}
                </Alert>
              )}

            <div className="flex flex-wrap items-center gap-2">
              {primary}
              {error?.kind === "install" && mode === "in_app" && (
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => void downloadInstaller()}
                >
                  <Download className="h-3.5 w-3.5" aria-hidden="true" />
                  {t("updates.downloadInstead")}
                </Button>
              )}
              <Button
                variant="ghost"
                size="sm"
                onClick={() => void openUrl(releasePage)}
              >
                {t("updates.viewRelease")}
                <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
              </Button>
            </div>
          </>
        )}
      </div>
    </section>
  );
};
