import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Box,
  ChevronLeft,
  ChevronRight,
  Download,
  Eye,
  Heart,
  Loader2,
  RefreshCw,
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

const isRecommendedQuant = (file: HfGgufFile): boolean =>
  (file.quant || file.filename).toUpperCase().includes("Q4_K_M");

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

  const hasQuery = query.trim().length > 0;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-[2px]"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="flex max-h-[86vh] w-full max-w-3xl flex-col overflow-hidden rounded-2xl border border-hairline-strong bg-surface shadow-[0_24px_80px_-24px_rgba(0,0,0,0.55)]"
        onClick={(event) => event.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-labelledby="hugging-face-dialog-title"
      >
        <div className="flex items-start justify-between gap-4 border-b border-hairline px-6 py-5">
          <div className="flex min-w-0 items-start gap-3">
            <span className="grid h-10 w-10 shrink-0 place-items-center rounded-xl bg-accent/12 text-accent">
              <Search className="h-[18px] w-[18px]" aria-hidden="true" />
            </span>
            <div className="min-w-0">
              <h2
                id="hugging-face-dialog-title"
                className="text-base font-semibold tracking-tight text-ink"
              >
                {t("settings.models.customModel.title")}
              </h2>
              <p className="mt-0.5 max-w-[60ch] text-xs leading-relaxed text-muted">
                {t("settings.models.customModel.subtitle")}
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

        <div className="flex-1 overflow-y-auto px-6 py-5">
          {selectedRepo ? (
            <div className="space-y-5">
              <button
                type="button"
                onClick={handleBack}
                className="group -ms-1.5 flex items-center gap-1 rounded-lg px-1.5 py-1 text-[13px] text-muted transition-colors hover:bg-surface-strong hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 cursor-pointer"
              >
                <ChevronLeft
                  className="h-4 w-4 transition-transform group-hover:-translate-x-0.5 motion-reduce:transition-none"
                  aria-hidden="true"
                />
                {t("settings.models.customModel.back")}
              </button>

              <div className="flex items-center gap-3 rounded-2xl border border-hairline bg-surface-strong/55 p-4">
                <span className="grid h-10 w-10 shrink-0 place-items-center rounded-xl bg-surface text-accent elev-chip">
                  <Box className="h-[18px] w-[18px]" aria-hidden="true" />
                </span>
                <div className="min-w-0 flex-1">
                  <p className="break-all text-sm font-semibold text-ink">
                    {selectedRepo.id}
                  </p>
                  <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted">
                    <span className="flex items-center gap-1">
                      <Download className="h-3.5 w-3.5" aria-hidden="true" />
                      {t("settings.models.customModel.downloads", {
                        value: formatCount(selectedRepo.downloads),
                      })}
                    </span>
                    <span className="flex items-center gap-1">
                      <Heart className="h-3.5 w-3.5" aria-hidden="true" />
                      {formatCount(selectedRepo.likes)}
                    </span>
                    {selectedRepo.is_vision && (
                      <span className="flex items-center gap-1 text-accent">
                        <Eye className="h-3.5 w-3.5" aria-hidden="true" />
                        {t("settings.models.customModel.visionBadge")}
                      </span>
                    )}
                  </div>
                </div>
              </div>

              {loadingFiles && (
                <div
                  className="space-y-2"
                  role="status"
                  aria-label={t("settings.models.customModel.loadingFiles")}
                >
                  {[0, 1, 2].map((item) => (
                    <div
                      key={item}
                      className="flex items-center gap-3 rounded-xl border border-hairline p-3"
                      aria-hidden="true"
                    >
                      <div className="min-w-0 flex-1 space-y-2">
                        <div className="h-3 w-24 animate-pulse rounded bg-surface-strong motion-reduce:animate-none" />
                        <div className="h-2.5 w-3/4 animate-pulse rounded bg-surface-strong motion-reduce:animate-none" />
                      </div>
                      <div className="h-8 w-24 animate-pulse rounded-lg bg-surface-strong motion-reduce:animate-none" />
                    </div>
                  ))}
                </div>
              )}

              {filesError && !loadingFiles && (
                <div className="rounded-2xl border border-error/25 bg-error/[0.06] px-4 py-5 text-center">
                  <p className="text-sm text-error">
                    {t("settings.models.customModel.repoFilesError")}
                  </p>
                  <Button
                    variant="secondary"
                    size="sm"
                    className="mt-3"
                    onClick={() => handleSelectRepo(selectedRepo)}
                  >
                    <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
                    {t("settings.models.customModel.tryAgain")}
                  </Button>
                </div>
              )}

              {!loadingFiles &&
                !filesError &&
                repoFiles &&
                repoFiles.gguf_files.length === 0 && (
                  <div className="rounded-2xl border border-dashed border-hairline-strong px-4 py-6 text-center">
                    <p className="text-sm text-muted">
                      {t("settings.models.customModel.noGgufFiles")}
                    </p>
                  </div>
                )}

              {!loadingFiles &&
                !filesError &&
                repoFiles &&
                repoFiles.gguf_files.length > 0 && (
                  <div className="space-y-4">
                    {repoFiles.mmproj_files.length > 0 && (
                      <label className="flex items-center gap-3 rounded-2xl border border-hairline bg-surface-strong/45 p-3.5 cursor-pointer">
                        <span className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-surface text-accent">
                          <Eye className="h-4 w-4" aria-hidden="true" />
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="block text-sm font-medium text-ink">
                            {t("settings.models.customModel.enableVision")}
                          </span>
                          <span className="mt-0.5 block text-xs leading-relaxed text-muted">
                            {t("settings.models.customModel.enableVisionHint")}
                          </span>
                        </span>
                        <input
                          type="checkbox"
                          checked={enableVision}
                          onChange={(event) =>
                            setEnableVision(event.target.checked)
                          }
                          className="h-4 w-4 shrink-0 accent-accent"
                        />
                      </label>
                    )}

                    <div>
                      <h3 className="text-sm font-semibold text-ink">
                        {t("settings.models.customModel.selectQuant")}
                      </h3>
                      <p className="mt-0.5 max-w-[62ch] text-xs leading-relaxed text-muted">
                        {t("settings.models.customModel.selectQuantHint")}
                      </p>
                    </div>

                    <div className="space-y-2">
                      {repoFiles.gguf_files.map((file) => {
                        const isAdding = addingFile === file.filename;
                        const anyAdding = addingFile !== null;
                        const recommended = isRecommendedQuant(file);
                        return (
                          <div
                            key={file.filename}
                            className={`flex flex-col gap-3 rounded-xl border p-3 transition-colors sm:flex-row sm:items-center ${
                              recommended
                                ? "border-accent/35 bg-accent/[0.045]"
                                : "border-hairline hover:border-hairline-strong"
                            }`}
                          >
                            <div className="min-w-0 flex-1">
                              <div className="flex flex-wrap items-center gap-2">
                                <span className="text-sm font-semibold text-ink">
                                  {file.quant || file.filename}
                                </span>
                                {recommended && (
                                  <span className="rounded-md border border-accent/25 bg-accent/10 px-1.5 py-0.5 text-[11px] font-medium text-accent">
                                    {t("onboarding.recommended")}
                                  </span>
                                )}
                              </div>
                              <span className="mt-0.5 block break-all text-xs text-muted">
                                {file.filename}
                              </span>
                            </div>
                            <div className="flex shrink-0 items-center justify-between gap-3 sm:justify-end">
                              <span className="whitespace-nowrap text-xs tabular-nums text-muted">
                                {formatModelSize(bytesToMb(file.size_bytes))}
                              </span>
                              <Button
                                variant={recommended ? "primary" : "secondary"}
                                size="sm"
                                disabled={anyAdding}
                                onClick={() => handleAdd(file)}
                              >
                                {isAdding ? (
                                  <Loader2
                                    className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none"
                                    aria-hidden="true"
                                  />
                                ) : (
                                  <Download
                                    className="h-3.5 w-3.5"
                                    aria-hidden="true"
                                  />
                                )}
                                {isAdding
                                  ? t("settings.models.customModel.adding")
                                  : t("settings.models.customModel.add")}
                              </Button>
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                )}
            </div>
          ) : (
            <div className="space-y-5">
              <div className="relative">
                <Search
                  className="pointer-events-none absolute start-4 top-1/2 h-[18px] w-[18px] -translate-y-1/2 text-muted"
                  aria-hidden="true"
                />
                <input
                  ref={searchInputRef}
                  type="search"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder={t(
                    "settings.models.customModel.searchPlaceholder",
                  )}
                  className="h-12 w-full rounded-xl border border-hairline-strong bg-surface ps-11 pe-4 text-sm text-ink transition-[border-color,box-shadow,background-color] duration-150 placeholder:text-muted hover:border-ink/35 focus:border-accent focus:outline-none focus:ring-2 focus:ring-accent/20"
                />
              </div>

              <div className="flex items-center justify-between gap-3 px-0.5">
                <h3 className="text-sm font-semibold text-ink">
                  {hasQuery
                    ? t("settings.models.customModel.resultsTitle")
                    : t("settings.models.customModel.popularTitle")}
                </h3>
                {!searching && results.length > 0 && (
                  <span className="text-xs tabular-nums text-muted">
                    {t("settings.models.customModel.resultCount", {
                      count: results.length,
                    })}
                  </span>
                )}
              </div>

              {searching && (
                <div
                  className="grid grid-cols-1 gap-2 sm:grid-cols-2"
                  role="status"
                  aria-label={t("settings.models.customModel.searching")}
                >
                  {[0, 1, 2, 3].map((item) => (
                    <div
                      key={item}
                      className="flex min-h-[88px] items-center gap-3 rounded-xl border border-hairline p-3.5"
                      aria-hidden="true"
                    >
                      <div className="h-9 w-9 shrink-0 animate-pulse rounded-xl bg-surface-strong motion-reduce:animate-none" />
                      <div className="min-w-0 flex-1 space-y-2">
                        <div className="h-3 w-3/4 animate-pulse rounded bg-surface-strong motion-reduce:animate-none" />
                        <div className="h-2.5 w-1/2 animate-pulse rounded bg-surface-strong motion-reduce:animate-none" />
                      </div>
                    </div>
                  ))}
                </div>
              )}

              {searchError && !searching && (
                <div className="rounded-2xl border border-error/25 bg-error/[0.06] px-4 py-6 text-center">
                  <p className="text-sm text-error">
                    {t("settings.models.customModel.searchError")}
                  </p>
                  <Button
                    variant="secondary"
                    size="sm"
                    className="mt-3"
                    onClick={() => runSearch(query)}
                  >
                    <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
                    {t("settings.models.customModel.tryAgain")}
                  </Button>
                </div>
              )}

              {!searching && !searchError && results.length === 0 && (
                <div className="rounded-2xl border border-dashed border-hairline-strong px-4 py-8 text-center">
                  <span className="mx-auto grid h-10 w-10 place-items-center rounded-xl bg-surface-strong text-muted">
                    <Search className="h-[18px] w-[18px]" aria-hidden="true" />
                  </span>
                  <p className="mt-3 text-sm text-muted">
                    {t("settings.models.customModel.noResults")}
                  </p>
                </div>
              )}

              {!searching && !searchError && results.length > 0 && (
                <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                  {results.map((repo) => (
                    <button
                      key={repo.id}
                      type="button"
                      onClick={() => handleSelectRepo(repo)}
                      className="group flex min-h-[88px] items-center gap-3 rounded-xl border border-hairline bg-surface p-3.5 text-start transition-[background-color,border-color,transform] duration-150 hover:border-accent/30 hover:bg-surface-strong/65 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 active:scale-[0.99] cursor-pointer"
                    >
                      <span className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-surface-strong text-muted transition-colors group-hover:text-accent">
                        <Box className="h-4 w-4" aria-hidden="true" />
                      </span>
                      <span className="min-w-0 flex-1">
                        <span
                          className="block truncate text-sm font-semibold text-ink"
                          title={repo.id}
                        >
                          {repo.id}
                        </span>
                        <span className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted">
                          <span className="flex items-center gap-1">
                            <Download className="h-3 w-3" aria-hidden="true" />
                            {t("settings.models.customModel.downloads", {
                              value: formatCount(repo.downloads),
                            })}
                          </span>
                          <span className="flex items-center gap-1">
                            <Heart className="h-3 w-3" aria-hidden="true" />
                            {formatCount(repo.likes)}
                          </span>
                          {repo.is_vision && (
                            <span className="flex items-center gap-1 text-accent">
                              <Eye className="h-3 w-3" aria-hidden="true" />
                              {t("settings.models.customModel.visionBadge")}
                            </span>
                          )}
                        </span>
                      </span>
                      <ChevronRight
                        className="h-4 w-4 shrink-0 text-muted transition-transform group-hover:translate-x-0.5 motion-reduce:transition-none"
                        aria-hidden="true"
                      />
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
