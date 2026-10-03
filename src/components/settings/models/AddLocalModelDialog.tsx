import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { FilePlus2, FolderOpen, Loader2, RefreshCw, X } from "lucide-react";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { useModelStore } from "@/stores/modelStore";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { InfoTip } from "@/components/ui/InfoTip";

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

  // Through the shared portal Dialog: a `fixed` overlay drawn inside a page is
  // trapped by the page's container query (it becomes the containing block),
  // which is how this "modal" used to cover one column of the window.
  return (
    <Dialog
      open={isOpen}
      onClose={onClose}
      size="lg"
      title={t("settings.models.localModel.title")}
      description={t("settings.models.localModel.subtitle")}
    >
      <div className="space-y-6">
        <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
          <ChoiceButton
            icon={
              pickingFiles ? (
                <Loader2
                  className="h-4 w-4 animate-spin motion-reduce:animate-none"
                  aria-hidden="true"
                />
              ) : (
                <FilePlus2 className="h-4 w-4" aria-hidden="true" />
              )
            }
            title={t("settings.models.localModel.pickFiles")}
            info={t("settings.models.localModel.pickFilesHint")}
            onClick={handlePickFiles}
            disabled={busy}
          />
          <ChoiceButton
            icon={
              pickingFolder ? (
                <Loader2
                  className="h-4 w-4 animate-spin motion-reduce:animate-none"
                  aria-hidden="true"
                />
              ) : (
                <FolderOpen className="h-4 w-4" aria-hidden="true" />
              )
            }
            title={t("settings.models.localModel.linkFolder")}
            info={t("settings.models.localModel.linkFolderHint")}
            onClick={handleLinkFolder}
            disabled={busy}
          />
        </div>

        <div className="space-y-2.5">
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-1">
              <h3 className="text-[0.8125rem] font-semibold text-muted">
                {t("settings.models.localModel.foldersTitle")}
              </h3>
              <InfoTip text={t("settings.models.localModel.footnote")} />
            </div>
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
            <p className="rounded-xl border border-dashed border-hairline-strong px-4 py-5 text-center text-[0.8125rem] text-muted">
              {t("settings.models.localModel.foldersEmptyShort")}
            </p>
          ) : (
            <ul className="divide-y divide-hairline overflow-hidden rounded-xl border border-hairline">
              {folders.map((folder) => (
                <li
                  key={folder}
                  className="flex items-center gap-3 bg-surface px-3.5 py-2.5"
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
                    className="grid h-7 w-7 shrink-0 cursor-pointer place-items-center rounded-lg text-muted transition-colors hover:bg-error/10 hover:text-error focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-50"
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
        </div>
      </div>
    </Dialog>
  );
};

/** One of the two ways in: a short title, with the detail behind its (i). */
const ChoiceButton: React.FC<{
  icon: React.ReactNode;
  title: string;
  info: string;
  onClick: () => void;
  disabled?: boolean;
}> = ({ icon, title, info, onClick, disabled }) => (
  <div className="relative">
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="flex w-full cursor-pointer items-center gap-3 rounded-xl border border-hairline-strong bg-surface px-4 py-3.5 pe-10 text-start transition-colors hover:bg-surface-muted focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-60"
    >
      <span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-surface-strong text-ink/70">
        {icon}
      </span>
      <span className="text-sm font-medium text-ink">{title}</span>
    </button>
    <span className="absolute end-3 top-1/2 -translate-y-1/2">
      <InfoTip text={info} />
    </span>
  </div>
);
