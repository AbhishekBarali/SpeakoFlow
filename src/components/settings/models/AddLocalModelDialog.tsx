import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import {
  FilePlus2,
  FolderOpen,
  FolderSearch,
  Loader2,
  RefreshCw,
  X,
} from "lucide-react";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { useModelStore } from "@/stores/modelStore";
import { Button } from "@/components/ui/Button";

interface AddLocalModelDialogProps {
  open: boolean;
  onClose: () => void;
}

/**
 * Register models the user already has on disk.
 *
 * Two paths, because people arrive with two different situations: a single file
 * they just produced (a fine-tune), or an existing collection they don't want to
 * move. Linking a folder is the durable option — it is rescanned, so models
 * added to it later appear on their own.
 *
 * Everything here is non-destructive. Files are read where they are, never
 * copied into the app, and unlinking only forgets a path.
 */
export const AddLocalModelDialog: React.FC<AddLocalModelDialogProps> = ({
  open: isOpen,
  onClose,
}) => {
  const { t } = useTranslation();
  const { loadModels } = useModelStore();

  const [folders, setFolders] = useState<string[]>([]);
  const [pickingFiles, setPickingFiles] = useState(false);
  const [pickingFolder, setPickingFolder] = useState(false);
  const [rescanning, setRescanning] = useState(false);
  const [removingFolder, setRemovingFolder] = useState<string | null>(null);

  const busy = pickingFiles || pickingFolder || rescanning;

  const refreshFolders = useCallback(async () => {
    const res = await commands.getModelFolders();
    if (res.status === "ok") setFolders(res.data);
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    void refreshFolders();
  }, [isOpen, refreshFolders]);

  // Close on Escape, matching the Hugging Face dialog.
  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [isOpen, onClose]);

  const handlePickFiles = async () => {
    setPickingFiles(true);
    try {
      const picked = await open({
        multiple: true,
        filters: [
          {
            name: t("settings.models.localModel.fileFilter"),
            extensions: ["gguf", "bin"],
          },
        ],
      });
      const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
      if (paths.length === 0) return;

      const res = await commands.addLocalModels(paths);
      if (res.status !== "ok") {
        toast.error(res.error);
        return;
      }

      const { added, failed } = res.data;
      if (added.length > 0) {
        await loadModels();
        toast.success(
          t("settings.models.localModel.filesAdded", { count: added.length }),
        );
      }
      // One unusable file among several must not hide the ones that worked, so
      // report each rejection with the reason the backend gave.
      for (const failure of failed) {
        toast.error(failure.message);
      }
    } catch (err) {
      toast.error(`${err}`);
    } finally {
      setPickingFiles(false);
    }
  };

  const handleLinkFolder = async () => {
    setPickingFolder(true);
    try {
      const picked = await open({ directory: true, multiple: true });
      const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
      if (paths.length === 0) return;

      let anyLinked = false;
      for (const path of paths) {
        const res = await commands.addModelFolder(path);
        if (res.status !== "ok") {
          toast.error(res.error);
          continue;
        }
        anyLinked = true;
        toast.success(
          t("settings.models.localModel.folderAdded", { count: res.data }),
        );
      }

      if (anyLinked) {
        await Promise.all([refreshFolders(), loadModels()]);
      }
    } catch (err) {
      toast.error(`${err}`);
    } finally {
      setPickingFolder(false);
    }
  };

  const handleRemoveFolder = async (path: string) => {
    setRemovingFolder(path);
    try {
      const res = await commands.removeModelFolder(path);
      if (res.status !== "ok") {
        toast.error(res.error);
        return;
      }
      await Promise.all([refreshFolders(), loadModels()]);
      toast.success(t("settings.models.localModel.folderRemoved"));
    } catch (err) {
      toast.error(`${err}`);
    } finally {
      setRemovingFolder(null);
    }
  };

  const handleRescan = async () => {
    setRescanning(true);
    try {
      const res = await commands.rescanLocalModels();
      if (res.status !== "ok") {
        toast.error(res.error);
        return;
      }
      await loadModels();
      toast.success(
        t("settings.models.localModel.rescanDone", { count: res.data }),
      );
    } catch (err) {
      toast.error(`${err}`);
    } finally {
      setRescanning(false);
    }
  };

  if (!isOpen) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-[2px]"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="flex max-h-[86vh] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-hairline-strong bg-surface shadow-[0_24px_80px_-24px_rgba(0,0,0,0.55)]"
        onClick={(event) => event.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-labelledby="local-model-dialog-title"
      >
        <div className="flex items-start justify-between gap-4 border-b border-hairline px-6 py-5">
          <div className="flex min-w-0 items-start gap-3">
            <span className="grid h-10 w-10 shrink-0 place-items-center rounded-xl bg-accent/12 text-accent">
              <FolderSearch className="h-[18px] w-[18px]" aria-hidden="true" />
            </span>
            <div className="min-w-0">
              <h2
                id="local-model-dialog-title"
                className="text-base font-semibold tracking-tight text-ink"
              >
                {t("settings.models.localModel.title")}
              </h2>
              <p className="mt-0.5 max-w-[62ch] text-xs leading-relaxed text-muted">
                {t("settings.models.localModel.subtitle")}
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="-me-1 grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-transparent text-muted transition-colors hover:border-hairline hover:bg-surface-strong hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 cursor-pointer"
            aria-label={t("common.close")}
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </div>

        <div className="flex-1 space-y-6 overflow-y-auto px-6 py-5">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <button
              type="button"
              onClick={handlePickFiles}
              disabled={busy}
              className="group flex flex-col items-start gap-2 rounded-2xl border border-hairline bg-surface p-4 text-start transition-[background-color,border-color,transform] duration-150 hover:border-accent/30 hover:bg-surface-strong/65 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 active:scale-[0.99] disabled:cursor-not-allowed disabled:opacity-60 cursor-pointer"
            >
              <span className="grid h-9 w-9 place-items-center rounded-xl bg-surface-strong text-muted transition-colors group-hover:text-accent">
                {pickingFiles ? (
                  <Loader2
                    className="h-4 w-4 animate-spin motion-reduce:animate-none"
                    aria-hidden="true"
                  />
                ) : (
                  <FilePlus2 className="h-4 w-4" aria-hidden="true" />
                )}
              </span>
              <span className="text-sm font-semibold text-ink">
                {t("settings.models.localModel.pickFiles")}
              </span>
              <span className="text-xs leading-relaxed text-muted">
                {t("settings.models.localModel.pickFilesHint")}
              </span>
            </button>

            <button
              type="button"
              onClick={handleLinkFolder}
              disabled={busy}
              className="group flex flex-col items-start gap-2 rounded-2xl border border-hairline bg-surface p-4 text-start transition-[background-color,border-color,transform] duration-150 hover:border-accent/30 hover:bg-surface-strong/65 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 active:scale-[0.99] disabled:cursor-not-allowed disabled:opacity-60 cursor-pointer"
            >
              <span className="grid h-9 w-9 place-items-center rounded-xl bg-surface-strong text-muted transition-colors group-hover:text-accent">
                {pickingFolder ? (
                  <Loader2
                    className="h-4 w-4 animate-spin motion-reduce:animate-none"
                    aria-hidden="true"
                  />
                ) : (
                  <FolderOpen className="h-4 w-4" aria-hidden="true" />
                )}
              </span>
              <span className="text-sm font-semibold text-ink">
                {t("settings.models.localModel.linkFolder")}
              </span>
              <span className="text-xs leading-relaxed text-muted">
                {t("settings.models.localModel.linkFolderHint")}
              </span>
            </button>
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <h3 className="text-sm font-semibold text-ink">
                {t("settings.models.localModel.foldersTitle")}
              </h3>
              {folders.length > 0 && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={handleRescan}
                  disabled={busy}
                >
                  {rescanning ? (
                    <Loader2
                      className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none"
                      aria-hidden="true"
                    />
                  ) : (
                    <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
                  )}
                  {rescanning
                    ? t("settings.models.localModel.rescanning")
                    : t("settings.models.localModel.rescan")}
                </Button>
              )}
            </div>

            {folders.length === 0 ? (
              <p className="rounded-2xl border border-dashed border-hairline-strong px-4 py-6 text-center text-xs leading-relaxed text-muted">
                {t("settings.models.localModel.foldersEmpty")}
              </p>
            ) : (
              <ul className="divide-y divide-hairline overflow-hidden rounded-2xl border border-hairline">
                {folders.map((folder) => (
                  <li
                    key={folder}
                    className="flex items-center gap-3 bg-surface px-3.5 py-3"
                  >
                    <FolderOpen
                      className="h-4 w-4 shrink-0 text-muted"
                      aria-hidden="true"
                    />
                    <span
                      className="min-w-0 flex-1 truncate font-mono text-xs text-ink"
                      title={folder}
                      dir="ltr"
                    >
                      {folder}
                    </span>
                    <button
                      type="button"
                      onClick={() => handleRemoveFolder(folder)}
                      disabled={removingFolder !== null || busy}
                      className="grid h-7 w-7 shrink-0 place-items-center rounded-lg border border-transparent text-muted transition-colors hover:border-hairline hover:bg-surface-strong hover:text-error focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-50 cursor-pointer"
                      aria-label={t("settings.models.localModel.removeFolder", {
                        folder,
                      })}
                    >
                      {removingFolder === folder ? (
                        <Loader2
                          className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none"
                          aria-hidden="true"
                        />
                      ) : (
                        <X className="h-3.5 w-3.5" aria-hidden="true" />
                      )}
                    </button>
                  </li>
                ))}
              </ul>
            )}

            <p className="text-xs leading-relaxed text-muted">
              {t("settings.models.localModel.footnote")}
            </p>
          </div>
        </div>
      </div>
    </div>
  );
};
