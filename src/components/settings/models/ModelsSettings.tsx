import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import {
  ChevronDown,
  Globe,
  HardDrive,
  Languages,
  Plus,
  Radio,
  RefreshCw,
  Search,
  Trash2,
  X,
  type LucideIcon,
} from "lucide-react";
import type { ModelCardStatus } from "@/components/onboarding";
import { useModelStore } from "@/stores/modelStore";
import { useSettings } from "@/hooks/useSettings";
import { languageLabel, languageMatches } from "@/lib/constants/languages.ts";
import {
  getModelCategory,
  type ModelCategory,
} from "@/lib/utils/modelCategory";
import { formatModelSize } from "@/lib/utils/format";
import { splitLocalModelName } from "@/lib/utils/prettyModelName";
import {
  getTranslatedModelDescription,
  getTranslatedModelName,
} from "@/lib/utils/modelTranslation";
import { commands, type ModelInfo } from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Tooltip } from "@/components/ui/Tooltip";
import { AddCustomModelDialog } from "./AddCustomModelDialog";
import { AddLocalModelDialog } from "./AddLocalModelDialog";
import {
  ModelRow,
  ModelTag,
  ScoreDots,
  type ModelRowAction,
  type ModelRowProgress,
} from "./ModelRow";
import { ModelLanguages } from "./ModelLanguages";
import {
  compareCatalogOrder,
  filterLanguages,
  hasFilters,
  isOffered,
  isStreamingModel,
  matchesFilters,
  NO_FILTERS,
  type SpeechFilters,
} from "./speechCatalog";
import { useReplaceRetired } from "./useReplaceRetired";

const CATEGORY_TABS: ModelCategory[] = ["stt", "llm", "tts"];

type BusyStatus = ModelRowProgress["state"];

const isBusy = (status: ModelCardStatus): status is BusyStatus =>
  status === "downloading" || status === "verifying" || status === "extracting";

/** What a row can ask the catalog to do. One stable object for every row. */
interface RowActions {
  select: (modelId: string) => void;
  download: (modelId: string) => void;
  remove: (modelId: string) => void;
  cancel: (modelId: string) => void;
  /** Fetch a retired model's replacement, or switch to it once it is here. */
  replace: (model: ModelInfo) => void;
}

/** A row's live state, kept to primitives so unchanged rows skip rendering. */
interface RowState {
  status: ModelCardStatus;
  percent?: number;
  speed?: number;
}

/**
 * What the row of an older model says about its replacement, and what doing
 * something about it looks like.
 */
interface ReplacementState extends RowState {
  model: ModelInfo;
}

interface ModelsSettingsProps {
  /** When set, the catalog shows ONLY this category — no category tabs. Used
   * by pages that open the catalog for one purpose (e.g. Dictation → stt). */
  lockedCategory?: ModelCategory;
}

export const ModelsSettings: React.FC<ModelsSettingsProps> = ({
  lockedCategory,
}) => {
  const { t, i18n } = useTranslation();
  const [switchingModelId, setSwitchingModelId] = useState<string | null>(null);
  const [categoryTab, setCategoryTab] = useState<ModelCategory>("stt");
  const categoryFilter = lockedCategory ?? categoryTab;
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [localDialogOpen, setLocalDialogOpen] = useState(false);
  const [nameSearch, setNameSearch] = useState("");
  const [filters, setFilters] = useState<SpeechFilters>(NO_FILTERS);
  const [languageDropdownOpen, setLanguageDropdownOpen] = useState(false);
  const [languageSearch, setLanguageSearch] = useState("");
  const [rescanning, setRescanning] = useState(false);
  const languageDropdownRef = useRef<HTMLDivElement>(null);
  const languageSearchInputRef = useRef<HTMLInputElement>(null);
  const {
    models,
    currentModel,
    downloadingModels,
    downloadProgress,
    downloadStats,
    verifyingModels,
    extractingModels,
    loading,
    loadModels,
    downloadModel,
    cancelDownload,
    selectModel,
    deleteModel,
  } = useModelStore();
  const { settings, refreshSettings } = useSettings();

  // The active local LLM is the model assigned to the built-in provider in the
  // Assistant settings. Used to show/select the "Active" Language Model here.
  const activeLlmId = settings?.assistant_models?.["builtin"] ?? "";
  // A cleanup fine-tune is "active" when it is the cleanup engine — it is never
  // the assistant brain, so checking the assistant slot would always say no.
  const activeCleanupLlmId = settings?.post_process_models?.["builtin"] ?? "";

  // click outside handler for language dropdown
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (
        languageDropdownRef.current &&
        !languageDropdownRef.current.contains(event.target as Node)
      ) {
        setLanguageDropdownOpen(false);
        setLanguageSearch("");
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  // focus search input when dropdown opens
  useEffect(() => {
    if (languageDropdownOpen && languageSearchInputRef.current) {
      languageSearchInputRef.current.focus();
    }
  }, [languageDropdownOpen]);

  // A model on this computer, or on its way there. Decides which group a row
  // sits in, and whether an older model is offered at all.
  const isOnDisk = useCallback(
    (model: ModelInfo) =>
      model.is_custom ||
      model.is_downloaded ||
      model.id in downloadingModels ||
      model.id in extractingModels,
    [downloadingModels, extractingModels],
  );

  const modelsById = useMemo(
    () => new Map(models.map((model) => [model.id, model])),
    [models],
  );

  // Speech models this list can show at all: an older one only while it is
  // still on disk.
  const offeredSpeechModels = useMemo(
    () =>
      models.filter(
        (model) =>
          getModelCategory(model) === "stt" &&
          isOffered(model, isOnDisk(model)),
      ),
    [models, isOnDisk],
  );

  // The language filter lists each language some offered model hears, once,
  // named in the UI language and in alphabetical order.
  const languageOptions = useMemo(() => {
    const collator = new Intl.Collator(i18n.language);
    return filterLanguages(offeredSpeechModels)
      .map((code) => ({
        value: code,
        label: languageLabel(code, i18n.language),
      }))
      .sort((a, b) => collator.compare(a.label, b.label));
  }, [offeredSpeechModels, i18n.language]);

  const filteredLanguages = useMemo(
    () =>
      languageOptions.filter((lang) => languageMatches(lang, languageSearch)),
    [languageOptions, languageSearch],
  );

  const selectedLanguageLabel =
    filters.language === "all"
      ? t("settings.models.filters.allLanguages")
      : languageLabel(filters.language, i18n.language);

  const isActiveModel = useCallback(
    (model: ModelInfo): boolean => {
      const category = getModelCategory(model);
      if (category === "llm") {
        return (
          model.id ===
          (model.is_cleanup_specialist ? activeCleanupLlmId : activeLlmId)
        );
      }
      return category === "stt" && model.id === currentModel;
    },
    [activeCleanupLlmId, activeLlmId, currentModel],
  );

  const getModelStatus = (model: ModelInfo): ModelCardStatus => {
    if (model.id in extractingModels) return "extracting";
    if (model.id in verifyingModels) return "verifying";
    if (model.id in downloadingModels) return "downloading";
    if (switchingModelId === model.id) return "switching";
    // A model that isn't on disk can never be "active" — guard this first so a
    // stale selection (e.g. an LLM download that failed after being chosen)
    // shows as downloadable again instead of a dead-end "active" card with no
    // way to (re)download it.
    if (!model.is_downloaded) return "downloadable";
    // A retired model can still be the saved selection after an upgrade, but
    // it never runs, so it never reads as in use.
    if (model.is_retired) return "available";
    return isActiveModel(model) ? "active" : "available";
  };

  const rowState = (model: ModelInfo): RowState => ({
    status: getModelStatus(model),
    percent: downloadProgress[model.id]?.percentage,
    speed: downloadStats[model.id]?.speed,
  });

  /**
   * A local model whose file has gone missing (moved, renamed, or on a drive
   * that isn't connected). There is nothing to download in that case, so say
   * what's actually wrong instead of letting a "download" fail obscurely.
   */
  const reportIfLocalFileMissing = (model: ModelInfo | undefined): boolean => {
    if (!model?.local_path || model.is_downloaded) return false;
    toast.error(
      t("settings.models.localModel.fileMissing", { path: model.local_path }),
    );
    return true;
  };

  const handleModelSelect = async (modelId: string) => {
    const model = modelsById.get(modelId);
    const category = model ? getModelCategory(model) : "stt";
    // TTS (Kokoro) has no "active" selection here — it is configured per
    // engine in the Assistant tab.
    if (category === "tts") return;
    if (reportIfLocalFileMissing(model)) return;

    setSwitchingModelId(modelId);
    try {
      if (category === "llm") {
        // Never point anything at a model that isn't on disk — that's the state
        // that used to strand users. If somehow triggered for a missing model,
        // download it instead of marking it active.
        if (!model?.is_downloaded) {
          await downloadModel(modelId);
          return;
        }
        // "Use model" has to mean the job the model can actually do. A cleanup
        // fine-tune cannot hold a conversation, so assigning it as the assistant
        // brain — which is what this page does for every other language model —
        // would quietly break the assistant. Send it to dictation cleanup
        // instead, which is the only thing it is for.
        if (model.is_cleanup_specialist) {
          await commands.setCleanupLocalModel(modelId);
          await refreshSettings();
          return;
        }
        // Assign the model to the built-in (local) assistant provider and make
        // that provider active, mirroring the footer LLM selector.
        await commands.changeAssistantModelSetting("builtin", modelId);
        if (settings?.assistant_provider_id !== "builtin") {
          await commands.setAssistantProvider("builtin");
        }
        await refreshSettings();
      } else {
        await selectModel(modelId);
      }
    } finally {
      setSwitchingModelId(null);
    }
  };

  const handleModelDownload = async (modelId: string) => {
    if (reportIfLocalFileMissing(modelsById.get(modelId))) return;
    await downloadModel(modelId);
  };

  const handleModelDelete = async (modelId: string) => {
    const model = modelsById.get(modelId);
    const modelName = model?.name || modelId;
    const isActive = model ? isActiveModel(model) : false;

    const confirmed = await ask(
      // A local model is only unregistered — saying "delete" would imply we
      // remove the user's own file, which we never do.
      model?.local_path
        ? t("settings.models.localModel.removeConfirm", {
            modelName,
            path: model.local_path,
          })
        : isActive
          ? t("settings.models.deleteActiveConfirm", { modelName })
          : t("settings.models.deleteConfirm", { modelName }),
      {
        title: model?.local_path
          ? t("settings.models.localModel.removeTitle")
          : t("settings.models.deleteTitle"),
        kind: "warning",
      },
    );

    if (confirmed) {
      try {
        await deleteModel(modelId);
      } catch (err) {
        console.error(`Failed to delete model ${modelId}:`, err);
      }
    }
  };

  const handleModelCancel = async (modelId: string) => {
    try {
      await cancelDownload(modelId);
    } catch (err) {
      console.error(`Failed to cancel download for ${modelId}:`, err);
    }
  };

  /**
   * "Get new version" on a retired model: the same rule as the prompts on
   * Home and at the top of this tab (`useReplaceRetired`). When the retired
   * model is the selected one, the store selects the replacement the moment
   * its download finishes, whichever page the user is on by then.
   */
  const { replace: replaceRetired } = useReplaceRetired();
  const handleReplace = async (retired: ModelInfo) => {
    await replaceRetired(retired);
  };

  // Rows are memoized and receive one actions object for their whole life;
  // it forwards to the latest handlers, so a download's progress ticks
  // re-render only the row that is downloading, not all 69.
  const handlersRef = useRef({
    select: handleModelSelect,
    download: handleModelDownload,
    remove: handleModelDelete,
    cancel: handleModelCancel,
    replace: handleReplace,
  });
  handlersRef.current = {
    select: handleModelSelect,
    download: handleModelDownload,
    remove: handleModelDelete,
    cancel: handleModelCancel,
    replace: handleReplace,
  };
  const actions = useMemo<RowActions>(
    () => ({
      select: (id) => void handlersRef.current.select(id),
      download: (id) => void handlersRef.current.download(id),
      remove: (id) => void handlersRef.current.remove(id),
      cancel: (id) => void handlersRef.current.cancel(id),
      replace: (model) => void handlersRef.current.replace(model),
    }),
    [],
  );

  const handleRescan = async () => {
    setRescanning(true);
    try {
      const result = await commands.rescanLocalModels();
      if (result.status !== "ok") {
        toast.error(result.error);
        return;
      }
      await loadModels();
      toast.success(
        t("settings.models.localModel.rescanDone", { count: result.data }),
      );
    } catch (err) {
      toast.error(`${err}`);
    } finally {
      setRescanning(false);
    }
  };

  // Filter by category, then (for transcription) by what may be offered and
  // the toolbar filters, then by the free-text search (the translated display
  // name and the raw catalog name, so either works).
  const filteredModels = useMemo(() => {
    const query = nameSearch.trim().toLowerCase();
    const source =
      categoryFilter === "stt"
        ? offeredSpeechModels
        : models.filter((model) => getModelCategory(model) === categoryFilter);
    return source.filter((model) => {
      if (categoryFilter === "stt" && !matchesFilters(model, filters)) {
        return false;
      }
      if (query) {
        const displayName = getTranslatedModelName(model, t).toLowerCase();
        if (
          !displayName.includes(query) &&
          !model.name.toLowerCase().includes(query)
        ) {
          return false;
        }
      }
      return true;
    });
  }, [models, offeredSpeechModels, filters, categoryFilter, nameSearch, t]);

  // Downloaded (custom and on-the-way included) above available, each in
  // catalog order, with the model doing this tab's job at the very top.
  const { downloadedModels, availableModels } = useMemo(() => {
    const downloaded: ModelInfo[] = [];
    const available: ModelInfo[] = [];
    for (const model of filteredModels) {
      (isOnDisk(model) ? downloaded : available).push(model);
    }
    downloaded.sort((a, b) => {
      const activeA = isActiveModel(a) && !a.is_retired;
      const activeB = isActiveModel(b) && !b.is_retired;
      if (activeA !== activeB) return activeA ? -1 : 1;
      return compareCatalogOrder(a, b);
    });
    available.sort(compareCatalogOrder);
    return { downloadedModels: downloaded, availableModels: available };
  }, [filteredModels, isOnDisk, isActiveModel]);

  // Retired models still on disk, for the notice above the list. Not
  // affected by the toolbar filters: they are something to act on, not
  // something to find. The selected one is left out: it has the prompt at the
  // top of this tab, and deleting the model dictation is set to belongs to
  // that prompt (move to the new version first), as upstream Handy does.
  const retiredOnDisk = useMemo(
    () =>
      categoryFilter === "stt"
        ? offeredSpeechModels.filter(
            (m) => m.is_retired && m.is_downloaded && m.id !== currentModel,
          )
        : [],
    [categoryFilter, offeredSpeechModels, currentModel],
  );

  const replacementFor = (model: ModelInfo): ReplacementState | undefined => {
    const replacement = model.replacement_id
      ? modelsById.get(model.replacement_id)
      : undefined;
    return replacement
      ? { model: replacement, ...rowState(replacement) }
      : undefined;
  };

  const handleDeleteRetired = async () => {
    const size = formatModelSize(
      retiredOnDisk.reduce((sum, model) => sum + Number(model.size_mb), 0),
    );
    const confirmed = await ask(
      t("settings.models.retired.deleteConfirm", { size }),
      { title: t("settings.models.retired.deleteTitle"), kind: "warning" },
    );
    if (!confirmed) return;
    for (const model of retiredOnDisk) {
      await deleteModel(model.id);
    }
  };

  const isSearching = nameSearch.trim() !== "";
  const filtering = categoryFilter === "stt" && hasFilters(filters);
  const hasAnyResults =
    downloadedModels.length > 0 || availableModels.length > 0;
  const clearFilters = () => {
    setFilters(NO_FILTERS);
    setNameSearch("");
  };

  const renderRow = (model: ModelInfo) => {
    const state = rowState(model);
    return (
      <CatalogRow
        key={model.id}
        model={model}
        status={state.status}
        percent={state.percent}
        speed={state.speed}
        replacement={model.is_legacy ? replacementFor(model) : undefined}
        actions={actions}
      />
    );
  };

  /** A section of the catalog: a quiet heading over one bordered list. */
  const renderSection = (title: React.ReactNode, list: ModelInfo[]) => (
    <section className="space-y-2.5">
      <h2 className="px-1 text-[0.8125rem] font-semibold text-muted">
        {title}
      </h2>
      <div className="divide-y divide-hairline rounded-2xl border border-hairline bg-surface elev-card">
        {list.map(renderRow)}
      </div>
    </section>
  );

  if (loading) {
    return (
      <div className="w-full">
        <div className="flex items-center justify-center py-16">
          <div className="w-8 h-8 border-2 border-hairline-strong border-t-ink rounded-full animate-spin" />
        </div>
      </div>
    );
  }

  return (
    <div
      className="w-full scroll-mt-6 space-y-6"
      // "Choose another model" on the speech tab's update prompt scrolls here
      // (`SPEECH_CATALOG_ATTRIBUTE`).
      data-speech-catalog={categoryFilter === "stt" ? "" : undefined}
    >
      {/* Category switcher: Transcription / Language Model / Speech — hidden
          when the catalog is locked to a single purpose. */}
      {!lockedCategory && (
        <>
          <div className="flex items-center gap-0.5 p-0.5 bg-surface-strong rounded-lg w-fit">
            {CATEGORY_TABS.map((cat) => (
              <button
                key={cat}
                type="button"
                onClick={() => setCategoryTab(cat)}
                className={`px-3.5 py-1.5 text-[13px] font-medium rounded-[7px] transition-all duration-150 cursor-pointer ${
                  categoryFilter === cat
                    ? "bg-surface text-ink shadow-[0_1px_3px_rgba(0,0,0,0.12)]"
                    : "text-muted hover:text-ink"
                }`}
              >
                {t(`settings.models.categories.${cat}`)}
              </button>
            ))}
          </div>
          <p className="text-[13px] text-muted">
            {t(`settings.models.categoryDescriptions.${categoryFilter}`)}
          </p>
        </>
      )}
      {categoryFilter === "llm" && (
        <p className="text-xs text-muted-soft">
          {t("settings.models.llmHint")}
        </p>
      )}
      {categoryFilter === "tts" && (
        <p className="text-xs text-muted-soft">
          {t("settings.models.ttsHint")}
        </p>
      )}

      {retiredOnDisk.length > 0 && (
        <RetiredNotice
          retired={retiredOnDisk}
          replacementFor={replacementFor}
          actions={actions}
          onDeleteAll={() => void handleDeleteRetired()}
        />
      )}

      {/* Toolbar: name search, then (stt) the language, streaming and
          translate filters, then the local-model buttons. Kept above the
          sections so it is always reachable, even with no results, and
          wrapping on a narrow window so nothing runs off the pane. */}
      <div className="@container">
        <div className="flex flex-wrap items-center gap-2">
          <div className="flex min-w-[12rem] flex-1 basis-[14rem] items-center gap-2 px-3 py-2 bg-surface border border-hairline rounded-lg focus-within:border-ink transition-colors">
            <Search className="w-4 h-4 shrink-0 text-muted-soft" />
            <input
              type="text"
              value={nameSearch}
              onChange={(e) => setNameSearch(e.target.value)}
              placeholder={t("settings.models.searchPlaceholder")}
              aria-label={t("settings.models.searchPlaceholder")}
              className="flex-1 min-w-0 bg-transparent text-[13px] text-ink focus:outline-none placeholder:text-muted-soft"
            />
            {nameSearch && (
              <button
                type="button"
                onClick={() => setNameSearch("")}
                aria-label={t("settings.models.clearSearch")}
                className="shrink-0 text-muted-soft hover:text-ink transition-colors cursor-pointer"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            )}
          </div>
          {categoryFilter === "stt" && (
            <>
              <div className="relative" ref={languageDropdownRef}>
                <button
                  type="button"
                  onClick={() => setLanguageDropdownOpen(!languageDropdownOpen)}
                  aria-expanded={languageDropdownOpen}
                  className={`flex h-9 items-center gap-1.5 px-3 text-[13px] font-medium rounded-lg transition-colors cursor-pointer ${
                    filters.language !== "all"
                      ? "bg-accent/10 text-accent"
                      : "bg-surface-strong text-muted hover:text-ink"
                  }`}
                >
                  <Globe className="w-3.5 h-3.5" aria-hidden="true" />
                  <span className="max-w-[120px] truncate">
                    {selectedLanguageLabel}
                  </span>
                  <ChevronDown
                    aria-hidden="true"
                    className={`w-3.5 h-3.5 transition-transform ${
                      languageDropdownOpen ? "rotate-180" : ""
                    }`}
                  />
                </button>

                {languageDropdownOpen && (
                  <div className="absolute top-full start-0 mt-1 w-56 bg-surface border border-hairline rounded-xl shadow-lg z-50 overflow-hidden">
                    <div className="p-2 border-b border-hairline">
                      <input
                        ref={languageSearchInputRef}
                        type="text"
                        value={languageSearch}
                        onChange={(e) => setLanguageSearch(e.target.value)}
                        onKeyDown={(e) => {
                          if (
                            e.key === "Enter" &&
                            filteredLanguages.length > 0
                          ) {
                            setFilters((f) => ({
                              ...f,
                              language: filteredLanguages[0].value,
                            }));
                            setLanguageDropdownOpen(false);
                            setLanguageSearch("");
                          } else if (e.key === "Escape") {
                            setLanguageDropdownOpen(false);
                            setLanguageSearch("");
                          }
                        }}
                        placeholder={t(
                          "settings.general.language.searchPlaceholder",
                        )}
                        className="w-full px-2.5 py-1.5 text-sm bg-surface border border-hairline-strong rounded-lg focus:outline-none focus:border-ink"
                      />
                    </div>
                    <div className="max-h-48 overflow-y-auto p-1">
                      <button
                        type="button"
                        onClick={() => {
                          setFilters((f) => ({ ...f, language: "all" }));
                          setLanguageDropdownOpen(false);
                          setLanguageSearch("");
                        }}
                        className={`w-full px-3 py-1.5 text-sm text-start rounded-lg transition-colors ${
                          filters.language === "all"
                            ? "bg-surface-strong text-ink font-medium"
                            : "hover:bg-surface-strong"
                        }`}
                      >
                        {t("settings.models.filters.allLanguages")}
                      </button>
                      {filteredLanguages.map((lang) => (
                        <button
                          key={lang.value}
                          type="button"
                          onClick={() => {
                            setFilters((f) => ({ ...f, language: lang.value }));
                            setLanguageDropdownOpen(false);
                            setLanguageSearch("");
                          }}
                          className={`w-full px-3 py-1.5 text-sm text-start rounded-lg transition-colors ${
                            filters.language === lang.value
                              ? "bg-surface-strong text-ink font-medium"
                              : "hover:bg-surface-strong"
                          }`}
                        >
                          {lang.label}
                        </button>
                      ))}
                      {filteredLanguages.length === 0 && (
                        <div className="px-3 py-2 text-sm text-muted-soft text-center">
                          {t("settings.general.language.noResults")}
                        </div>
                      )}
                    </div>
                  </div>
                )}
              </div>
              <FilterToggle
                icon={Radio}
                label={t("modelSelector.capabilities.streaming")}
                hint={t("settings.models.filters.streamingHint")}
                pressed={filters.streaming}
                onToggle={() =>
                  setFilters((f) => ({ ...f, streaming: !f.streaming }))
                }
              />
              <FilterToggle
                icon={Languages}
                label={t("settings.models.filters.translate")}
                hint={t("settings.models.filters.translateHint")}
                pressed={filters.translate}
                onToggle={() =>
                  setFilters((f) => ({ ...f, translate: !f.translate }))
                }
              />
            </>
          )}
          {categoryFilter !== "tts" && (
            <>
              <IconButton
                icon={RefreshCw}
                label={t("settings.models.localModel.rescan")}
                busyLabel={t("settings.models.localModel.rescanning")}
                busy={rescanning}
                onClick={() => void handleRescan()}
              />
              {/* Icon-only in a narrow pane, so the filters, rescan and this
                share one line under the search instead of three. */}
              <Button
                variant="secondary"
                size="sm"
                className="h-9 px-2.5 @[36rem]:px-3"
                aria-label={t("settings.models.localModel.addButton")}
                title={t("settings.models.localModel.addButton")}
                onClick={() => setLocalDialogOpen(true)}
              >
                <HardDrive className="w-4 h-4" aria-hidden="true" />
                <span className="hidden @[36rem]:inline">
                  {t("settings.models.localModel.addButton")}
                </span>
              </Button>
              {categoryFilter === "llm" && (
                <Button
                  variant="secondary"
                  size="sm"
                  className="h-9"
                  onClick={() => setAddDialogOpen(true)}
                >
                  <Plus className="w-4 h-4" aria-hidden="true" />
                  {t("settings.models.customModel.addButton")}
                </Button>
              )}
            </>
          )}
        </div>
      </div>

      {hasAnyResults ? (
        <div className="space-y-8">
          {downloadedModels.length > 0 &&
            renderSection(t("settings.models.yourModels"), downloadedModels)}
          {availableModels.length > 0 &&
            renderSection(
              t("settings.models.availableModels"),
              availableModels,
            )}
        </div>
      ) : (
        <div className="flex flex-col items-center gap-3 rounded-2xl border border-dashed border-hairline-strong px-6 py-10 text-center">
          <p className="text-sm text-muted">
            {isSearching && !filtering
              ? t("settings.models.noSearchResults", {
                  query: nameSearch.trim(),
                })
              : t("settings.models.filters.noMatch")}
          </p>
          {(isSearching || filtering) && (
            <Button variant="secondary" size="sm" onClick={clearFilters}>
              {t("settings.models.filters.clear")}
            </Button>
          )}
        </div>
      )}
      <AddCustomModelDialog
        open={addDialogOpen}
        onClose={() => setAddDialogOpen(false)}
      />
      <AddLocalModelDialog
        open={localDialogOpen}
        onClose={() => setLocalDialogOpen(false)}
      />
    </div>
  );
};

/* ───────────────────────────── one row ───────────────────────────── */

interface CatalogRowProps extends RowState {
  model: ModelInfo;
  /** For an older model, the catalog model that replaces it. */
  replacement?: ReplacementState;
  actions: RowActions;
}

const sameReplacement = (
  a: ReplacementState | undefined,
  b: ReplacementState | undefined,
) =>
  a === b ||
  (!!a &&
    !!b &&
    a.model === b.model &&
    a.status === b.status &&
    a.percent === b.percent &&
    a.speed === b.speed);

/**
 * One model in the catalog. Memoized on its own state, so with 69 rows a
 * download's progress ticks (ten a second) re-render one row, not the list.
 */
const CatalogRow = React.memo<CatalogRowProps>(
  ({ model, status, percent, speed, replacement, actions }) => {
    const { t } = useTranslation();
    const category = getModelCategory(model);
    const { name, quant } = splitLocalModelName(
      getTranslatedModelName(model, t),
      model.filename,
    );
    const missing = !!model.local_path && !model.is_downloaded;
    const busy = isBusy(status);
    const retired = !!model.is_retired;
    // Models from a linked folder can't be removed one at a time — the next
    // scan would just find them again. Unlinking the folder is the action that
    // works, so don't offer one that doesn't.
    const removable =
      category !== "tts" &&
      !model.local_folder &&
      (model.is_downloaded || model.is_custom || missing);

    const replacementName = replacement
      ? getTranslatedModelName(replacement.model, t)
      : "";
    const replacementSize = replacement
      ? formatModelSize(Number(replacement.model.size_mb))
      : "";
    const replacementBusy = !!replacement && isBusy(replacement.status);

    let action: ModelRowAction;
    let progress: ModelRowProgress | null = null;
    if (retired) {
      // A retired model's only way forward is its new version: fetched here,
      // shown downloading here, and switched to from here once it is on disk.
      if (replacement && replacementBusy) {
        action = { kind: "none" };
        progress = {
          state: replacement.status as BusyStatus,
          percent: replacement.percent,
          speed: replacement.speed,
          onCancel:
            replacement.status === "downloading"
              ? () => actions.cancel(replacement.model.id)
              : undefined,
        };
      } else if (replacement) {
        action = {
          kind: "use",
          primary: true,
          label: replacement.model.is_downloaded
            ? t("settings.models.retired.useNewVersion")
            : t("settings.models.retired.getNewVersion"),
          onClick: () => actions.replace(model),
        };
      } else {
        action = { kind: "none" };
      }
    } else if (busy) {
      action = { kind: "none" };
      progress = {
        state: status,
        percent,
        speed,
        onCancel:
          status === "downloading" ? () => actions.cancel(model.id) : undefined,
      };
    } else if (missing) {
      action = { kind: "missing", title: model.local_path ?? undefined };
    } else if (status === "downloadable") {
      action = { kind: "download", onClick: () => actions.download(model.id) };
    } else if (status === "switching") {
      action = { kind: "switching" };
    } else if (status === "active") {
      action = { kind: "inUse" };
    } else if (category === "tts") {
      action = { kind: "none" };
    } else {
      action = {
        kind: "use",
        onClick: () => actions.select(model.id),
        ariaLabel: t("catalog.useNamed", {
          model: name,
          job: t(`settings.models.categories.${category}`),
        }),
      };
    }

    const replacementHint = replacement
      ? t(
          retired ? "settings.models.retiredHint" : "settings.models.olderHint",
          { model: replacementName, size: replacementSize },
        )
      : undefined;

    return (
      <ModelRow
        model={model}
        name={name}
        quant={model.is_custom ? quant : null}
        subtitle={getTranslatedModelDescription(model, t)}
        size={formatModelSize(Number(model.size_mb))}
        current={status === "active"}
        badges={
          <>
            {isStreamingModel(model) && (
              <ModelTag tone="success" icon={Radio}>
                {t("modelSelector.capabilities.streaming")}
              </ModelTag>
            )}
            {retired ? (
              <ModelTag tone="warning" title={replacementHint}>
                {t("modelSelector.capabilities.retired")}
              </ModelTag>
            ) : (
              model.is_legacy && (
                <ModelTag title={replacementHint}>
                  {t("modelSelector.capabilities.legacy")}
                </ModelTag>
              )
            )}
            {model.is_recommended && status !== "active" && (
              <ModelTag>{t("onboarding.recommended")}</ModelTag>
            )}
          </>
        }
        meta={
          category === "stt" ? (
            <>
              <ModelLanguages
                codes={model.supported_languages}
                modelName={name}
              />
              {model.supports_translation && !retired && (
                <span className="inline-flex items-center gap-1">
                  <Languages className="h-3.5 w-3.5" aria-hidden="true" />
                  {t("modelSelector.capabilities.translate")}
                </span>
              )}
              {model.accuracy_score > 0 && (
                <ScoreDots
                  label={t("onboarding.modelCard.accuracy")}
                  score={model.accuracy_score}
                />
              )}
              {model.speed_score > 0 && (
                <ScoreDots
                  label={t("onboarding.modelCard.speed")}
                  score={model.speed_score}
                />
              )}
            </>
          ) : undefined
        }
        action={action}
        menu={
          removable && !busy
            ? [
                {
                  id: "delete",
                  icon: Trash2,
                  tone: "danger",
                  label: model.local_path
                    ? t("catalog.removeFromList")
                    : t("common.delete"),
                  hint:
                    status === "active"
                      ? t("settings.assistant.brain.switchBeforeDelete")
                      : undefined,
                  disabled: status === "active",
                  onSelect: () => actions.remove(model.id),
                },
              ]
            : undefined
        }
        progress={progress}
      />
    );
  },
  (prev, next) =>
    prev.model === next.model &&
    prev.status === next.status &&
    prev.percent === next.percent &&
    prev.speed === next.speed &&
    prev.actions === next.actions &&
    sameReplacement(prev.replacement, next.replacement),
);
CatalogRow.displayName = "CatalogRow";

/* ───────────────────────── retired models notice ───────────────────────── */

/**
 * Above the catalog while any retired model is still on disk: one sentence
 * on why, each model with its way forward, and one place to free the space.
 */
const RetiredNotice: React.FC<{
  retired: ModelInfo[];
  replacementFor: (model: ModelInfo) => ReplacementState | undefined;
  actions: RowActions;
  onDeleteAll: () => void;
}> = ({ retired, replacementFor, actions, onDeleteAll }) => {
  const { t } = useTranslation();
  const totalSize = formatModelSize(
    retired.reduce((sum, model) => sum + Number(model.size_mb), 0),
  );

  return (
    <section
      aria-labelledby="retired-models-notice"
      className="rounded-2xl border border-warning/25 bg-warning/[0.06] px-5 py-4"
    >
      <p id="retired-models-notice" className="text-sm text-body">
        {t("settings.models.retired.notice")}
      </p>
      <ul className="mt-3 divide-y divide-warning/15">
        {retired.map((model) => {
          const replacement = replacementFor(model);
          const busy = !!replacement && isBusy(replacement.status);
          return (
            <li
              key={model.id}
              className="flex flex-wrap items-center gap-x-4 gap-y-2 py-2.5"
            >
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium text-ink">
                  {getTranslatedModelName(model, t)}
                </p>
                {replacement && (
                  <p className="truncate text-xs text-muted">
                    {t("settings.models.retired.newVersion", {
                      model: getTranslatedModelName(replacement.model, t),
                      size: formatModelSize(Number(replacement.model.size_mb)),
                    })}
                  </p>
                )}
              </div>
              {replacement &&
                (busy ? (
                  <span className="text-xs tabular-nums text-muted">
                    {replacement.status === "downloading"
                      ? t("modelSelector.downloading", {
                          percentage: Math.round(replacement.percent ?? 0),
                        })
                      : replacement.status === "verifying"
                        ? t("modelSelector.verifyingGeneric")
                        : t("modelSelector.extractingGeneric")}
                  </span>
                ) : (
                  <Button size="sm" onClick={() => actions.replace(model)}>
                    {replacement.model.is_downloaded
                      ? t("settings.models.retired.useNewVersion")
                      : t("settings.models.retired.getNewVersion")}
                  </Button>
                ))}
            </li>
          );
        })}
      </ul>
      <div className="mt-2 flex justify-end">
        <Button variant="danger-ghost" size="sm" onClick={onDeleteAll}>
          <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
          {t("settings.models.retired.deleteOld", { size: totalSize })}
        </Button>
      </div>
    </section>
  );
};

/* ───────────────────────────── toolbar bits ───────────────────────────── */

/** Shows `hint` under `targetRef` while the control is hovered or focused. */
const useHoverHint = () => {
  const ref = useRef<HTMLButtonElement>(null);
  const [shown, setShown] = useState(false);
  const handlers = {
    onMouseEnter: () => setShown(true),
    onMouseLeave: () => setShown(false),
    onFocus: () => setShown(true),
    onBlur: () => setShown(false),
  };
  return { ref, shown, handlers };
};

/** An on/off filter in the toolbar, styled like the language filter. */
const FilterToggle: React.FC<{
  icon: LucideIcon;
  label: string;
  hint: string;
  pressed: boolean;
  onToggle: () => void;
}> = ({ icon: Icon, label, hint, pressed, onToggle }) => {
  const { ref, shown, handlers } = useHoverHint();
  return (
    <>
      <button
        ref={ref}
        type="button"
        aria-pressed={pressed}
        onClick={onToggle}
        {...handlers}
        className={`flex h-9 items-center gap-1.5 rounded-lg px-3 text-[13px] font-medium transition-colors cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${
          pressed
            ? "bg-accent/10 text-accent"
            : "bg-surface-strong text-muted hover:text-ink"
        }`}
      >
        <Icon className="h-3.5 w-3.5" aria-hidden={true} />
        {label}
      </button>
      {shown && (
        <Tooltip targetRef={ref} position="bottom">
          <span className="text-xs text-body">{hint}</span>
        </Tooltip>
      )}
    </>
  );
};

/** A square icon-only button with a hover label, for the rescan action. */
const IconButton: React.FC<{
  icon: LucideIcon;
  label: string;
  busyLabel: string;
  busy: boolean;
  onClick: () => void;
}> = ({ icon: Icon, label, busyLabel, busy, onClick }) => {
  const { ref, shown, handlers } = useHoverHint();
  const current = busy ? busyLabel : label;
  return (
    <>
      <button
        ref={ref}
        type="button"
        aria-label={current}
        aria-busy={busy}
        disabled={busy}
        onClick={onClick}
        {...handlers}
        className="grid h-9 w-9 shrink-0 cursor-pointer place-items-center rounded-lg bg-surface-strong text-muted transition-colors hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-default"
      >
        <Icon
          className={`h-3.5 w-3.5 ${busy ? "animate-spin" : ""}`}
          aria-hidden={true}
        />
      </button>
      {shown && (
        <Tooltip targetRef={ref} position="bottom">
          <span className="text-xs text-body">{current}</span>
        </Tooltip>
      )}
    </>
  );
};
