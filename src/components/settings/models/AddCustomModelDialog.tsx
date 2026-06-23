import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronLeft,
  Download,
  Eye,
  Heart,
  Loader2,
  Search,
  X,
} from "lucide-react";
import { toast } from "sonner";
import {
  commands,
  type HfGgufFile,
  type HfModelSummary,
  type HfRepoFiles,
} from "@/bindings";
import { useModelStore } from "@/stores/modelStore";
import { formatModelSize } from "@/lib/utils/format";
import { Button } from "@/components/ui/Button";

interface AddCustomModelDialogProps {
  open: boolean;
  onClose: () => void;
}

// Compact popularity formatter: 3812636 -> "3.8M", 12100 -> "12.1K".
const formatCount = (value: number): string => {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return `${value}`;
};

const bytesToMb = (bytes: number): number =>
  Math.max(1, Math.round(bytes / (1024 * 1024)));

export const AddCustomModelDialog: React.FC<AddCustomModelDialogProps> = ({
  open,
  onClose,
}) => {
  const { t } = useTranslation();
  const { loadModels, downloadModel } = useModelStore();

  const [query, setQuery] = useState("");
  const [results, setResults] = useState<HfModelSummary[]>([]);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState(false);

  // When a repo is selected we switch to the "pick a quantization" view.
  const [selectedRepo, setSelectedRepo] = useState<HfModelSummary | null>(null);
  const [repoFiles, setRepoFiles] = useState<HfRepoFiles | null>(null);
  const [loadingFiles, setLoadingFiles] = useState(false);
  const [filesError, setFilesError] = useState(false);
  const [enableVision, setEnableVision] = useState(true);
  const [addingFile, setAddingFile] = useState<string | null>(null);

  // Monotonic request id so a slow earlier search can't overwrite a newer one.
  const searchSeq = useRef(0);
  const searchInputRef = useRef<HTMLInputElement>(null);

  const runSearch = useCallback(async (q: string) => {
    const seq = ++searchSeq.current;
    setSearching(true);
    setSearchError(false);
    try {
      const res = await commands.searchHuggingfaceModels(q);
      if (seq !== searchSeq.current) return; // a newer search superseded us
      if (res.status === "ok") {
        setResults(res.data);
      } else {
        setSearchError(true);
      }
    } catch {
      if (seq === searchSeq.current) setSearchError(true);
    } finally {
      if (seq === searchSeq.current) setSearching(false);
    }
  }, []);

  // Reset everything when the dialog opens, then load the popular list.
  useEffect(() => {
    if (!open) return;
    setQuery("");
    setSelectedRepo(null);
    setRepoFiles(null);
    setFilesError(false);
    setResults([]);
    runSearch("");
    // Focus the search field shortly after the dialog mounts.
    const id = window.setTimeout(() => searchInputRef.current?.focus(), 50);
    return () => window.clearTimeout(id);
  }, [open, runSearch]);

  // Debounced search as the user types (only while browsing the result list).
  useEffect(() => {
    if (!open || selectedRepo) return;
    const id = window.setTimeout(() => runSearch(query), 400);
    return () => window.clearTimeout(id);
  }, [query, open, selectedRepo, runSearch]);

  // Close on Escape.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const handleSelectRepo = async (repo: HfModelSummary) => {
    setSelectedRepo(repo);
    setRepoFiles(null);
    setFilesError(false);
    setLoadingFiles(true);
    setEnableVision(true);
    try {
      const res = await commands.listHuggingfaceGgufFiles(repo.id);
      if (res.status === "ok") {
        setRepoFiles(res.data);
        // Default vision on only when a projector is actually available.
        setEnableVision(res.data.mmproj_files.length > 0);
      } else {
        setFilesError(true);
      }
    } catch {
      setFilesError(true);
    } finally {
      setLoadingFiles(false);
    }
  };

  const handleBack = () => {
    setSelectedRepo(null);
    setRepoFiles(null);
    setFilesError(false);
  };

  const handleAdd = async (file: HfGgufFile) => {
    if (!selectedRepo || !repoFiles) return;
    setAddingFile(file.filename);

    const useVision = enableVision && repoFiles.mmproj_files.length > 0;
    const mmproj = useVision ? repoFiles.mmproj_files[0] : null;
    const totalMb = bytesToMb(file.size_bytes + (mmproj?.size_bytes ?? 0));

    try {
      const res = await commands.addCustomLlmModel(
        selectedRepo.id,
        file.filename,
        totalMb,
        mmproj?.filename ?? null,
      );
      if (res.status !== "ok") {
        toast.error(res.error || t("settings.models.customModel.addError"));
        return;
      }
      // Refresh the catalog so the new entry shows, then kick off the download
      // without awaiting it — the store tracks progress and the model card
      // renders it, so we can close the dialog right away.
      await loadModels();
      void downloadModel(res.data.id);
      onClose();
    } catch (err) {
      toast.error(`${err}`);
    } finally {
      setAddingFile(null);
    }
  };

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="bg-surface border border-hairline rounded-2xl shadow-xl w-full max-w-xl max-h-[80vh] flex flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={t("settings.models.customModel.title")}
      >
        {/* Header */}
        <div className="flex items-start justify-between gap-4 px-5 pt-4 pb-3 border-b border-hairline">
          <div className="flex flex-col gap-0.5">
            <h2 className="text-base font-semibold text-ink">
              {t("settings.models.customModel.title")}
            </h2>
            <p className="text-xs text-muted">
              {t("settings.models.customModel.subtitle")}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="text-muted hover:text-ink transition-colors rounded-full p-1 -mr-1"
            aria-label={t("common.close")}
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-5 py-4">
          {selectedRepo ? (
            // ── Quantization picker for the selected repo ──────────────
            <div className="space-y-3">
              <button
                type="button"
                onClick={handleBack}
                className="flex items-center gap-1 text-sm text-muted hover:text-ink transition-colors"
              >
                <ChevronLeft className="w-4 h-4" />
                {t("settings.models.customModel.back")}
              </button>

              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-sm font-medium text-ink break-all">
                  {selectedRepo.id}
                </span>
                {selectedRepo.is_vision && (
                  <span className="flex items-center gap-1 text-xs text-muted">
                    <Eye className="w-3.5 h-3.5" />
                    {t("settings.models.customModel.visionBadge")}
                  </span>
                )}
              </div>

              {loadingFiles && (
                <div className="flex items-center justify-center gap-2 py-10 text-muted">
                  <Loader2 className="w-4 h-4 animate-spin" />
                  <span className="text-sm">
                    {t("settings.models.customModel.loadingFiles")}
                  </span>
                </div>
              )}

              {filesError && !loadingFiles && (
                <p className="text-sm text-error py-6 text-center">
                  {t("settings.models.customModel.repoFilesError")}
                </p>
              )}

              {!loadingFiles &&
                !filesError &&
                repoFiles &&
                repoFiles.gguf_files.length === 0 && (
                  <p className="text-sm text-muted py-6 text-center">
                    {t("settings.models.customModel.noGgufFiles")}
                  </p>
                )}

              {!loadingFiles &&
                !filesError &&
                repoFiles &&
                repoFiles.gguf_files.length > 0 && (
                  <>
                    {repoFiles.mmproj_files.length > 0 && (
                      <label className="flex items-start gap-2 p-2.5 rounded-lg bg-surface-strong cursor-pointer">
                        <input
                          type="checkbox"
                          checked={enableVision}
                          onChange={(e) => setEnableVision(e.target.checked)}
                          className="mt-0.5 accent-ink"
                        />
                        <span className="flex flex-col">
                          <span className="text-sm text-ink">
                            {t("settings.models.customModel.enableVision")}
                          </span>
                          <span className="text-xs text-muted">
                            {t("settings.models.customModel.enableVisionHint")}
                          </span>
                        </span>
                      </label>
                    )}

                    <p className="text-[11px] font-semibold text-muted uppercase tracking-[0.1em] pt-1">
                      {t("settings.models.customModel.selectQuant")}
                    </p>

                    <div className="space-y-1.5">
                      {repoFiles.gguf_files.map((file) => {
                        const isAdding = addingFile === file.filename;
                        const anyAdding = addingFile !== null;
                        return (
                          <div
                            key={file.filename}
                            className="flex items-center gap-3 p-2.5 rounded-lg border border-hairline hover:border-ink/30 transition-colors"
                          >
                            <div className="flex flex-col min-w-0 flex-1">
                              <span className="text-sm font-medium text-ink truncate">
                                {file.quant || file.filename}
                              </span>
                              <span className="text-xs text-muted truncate">
                                {file.filename}
                              </span>
                            </div>
                            <span className="text-xs text-muted whitespace-nowrap">
                              {formatModelSize(bytesToMb(file.size_bytes))}
                            </span>
                            <Button
                              variant="secondary"
                              size="sm"
                              disabled={anyAdding}
                              onClick={() => handleAdd(file)}
                            >
                              {isAdding ? (
                                <Loader2 className="w-3.5 h-3.5 animate-spin" />
                              ) : (
                                <Download className="w-3.5 h-3.5" />
                              )}
                              {isAdding
                                ? t("settings.models.customModel.adding")
                                : t("settings.models.customModel.add")}
                            </Button>
                          </div>
                        );
                      })}
                    </div>
                  </>
                )}
            </div>
          ) : (
            // ── Search + results list ──────────────────────────────────
            <div className="space-y-3">
              <div className="relative">
                <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-soft pointer-events-none" />
                <input
                  ref={searchInputRef}
                  type="text"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder={t(
                    "settings.models.customModel.searchPlaceholder",
                  )}
                  className="w-full ps-9 pe-3 py-2 text-sm bg-surface border border-hairline-strong rounded-lg text-ink placeholder:text-muted-soft transition-colors duration-150 hover:border-ink/40 focus:outline-none focus:border-ink"
                />
              </div>

              {searching && (
                <div className="flex items-center justify-center gap-2 py-10 text-muted">
                  <Loader2 className="w-4 h-4 animate-spin" />
                  <span className="text-sm">
                    {t("settings.models.customModel.searching")}
                  </span>
                </div>
              )}

              {searchError && !searching && (
                <p className="text-sm text-error py-6 text-center">
                  {t("settings.models.customModel.searchError")}
                </p>
              )}

              {!searching && !searchError && results.length === 0 && (
                <p className="text-sm text-muted py-6 text-center">
                  {t("settings.models.customModel.noResults")}
                </p>
              )}

              {!searching && !searchError && results.length > 0 && (
                <div className="space-y-1.5">
                  {results.map((repo) => (
                    <button
                      key={repo.id}
                      type="button"
                      onClick={() => handleSelectRepo(repo)}
                      className="w-full flex items-center gap-3 p-2.5 rounded-lg border border-hairline hover:border-ink/30 hover:bg-surface-strong transition-colors text-left"
                    >
                      <div className="flex flex-col min-w-0 flex-1">
                        <span className="text-sm font-medium text-ink truncate">
                          {repo.id}
                        </span>
                        <span className="flex items-center gap-3 text-xs text-muted">
                          <span className="flex items-center gap-1">
                            <Download className="w-3 h-3" />
                            {t("settings.models.customModel.downloads", {
                              value: formatCount(repo.downloads),
                            })}
                          </span>
                          <span className="flex items-center gap-1">
                            <Heart className="w-3 h-3" />
                            {formatCount(repo.likes)}
                          </span>
                        </span>
                      </div>
                      {repo.is_vision && (
                        <span className="flex items-center gap-1 text-xs text-muted whitespace-nowrap">
                          <Eye className="w-3.5 h-3.5" />
                          {t("settings.models.customModel.visionBadge")}
                        </span>
                      )}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
