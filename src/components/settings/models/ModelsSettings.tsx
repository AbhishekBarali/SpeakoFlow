import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ask } from "@tauri-apps/plugin-dialog";
import { ChevronDown, ChevronRight, Globe, Plus, Search, X } from "lucide-react";
import type { ModelCardStatus } from "@/components/onboarding";
import { isLegacyModel, ModelCard } from "@/components/onboarding";
import { useModelStore } from "@/stores/modelStore";
import { useSettings } from "@/hooks/useSettings";
import { LANGUAGES } from "@/lib/constants/languages.ts";
import {
  getModelCategory,
  type ModelCategory,
} from "@/lib/utils/modelCategory";
import { getTranslatedModelName } from "@/lib/utils/modelTranslation";
import { commands, type ModelInfo } from "@/bindings";
import { Button } from "@/components/ui/Button";
import { AddCustomModelDialog } from "./AddCustomModelDialog";

// check if model supports a language based on its supported_languages list
const modelSupportsLanguage = (model: ModelInfo, langCode: string): boolean => {
  return model.supported_languages.includes(langCode);
};

const CATEGORY_TABS: ModelCategory[] = ["stt", "llm", "tts"];

export const ModelsSettings: React.FC = () => {
  const { t } = useTranslation();
  const [switchingModelId, setSwitchingModelId] = useState<string | null>(null);
  const [categoryFilter, setCategoryFilter] = useState<ModelCategory>("stt");
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [nameSearch, setNameSearch] = useState("");
  const [showOlderModels, setShowOlderModels] = useState(false);
  const [languageFilter, setLanguageFilter] = useState("all");
  const [languageDropdownOpen, setLanguageDropdownOpen] = useState(false);
  const [languageSearch, setLanguageSearch] = useState("");
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
    downloadModel,
    cancelDownload,
    selectModel,
    deleteModel,
  } = useModelStore();
  const { settings, refreshSettings } = useSettings();

  // The active local LLM is the model assigned to the built-in provider in the
  // Assistant settings. Used to show/select the "Active" Language Model here.
  const activeLlmId = settings?.assistant_models?.["builtin"] ?? "";

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

  // filtered languages for dropdown (exclude "auto")
  const filteredLanguages = useMemo(() => {
    return LANGUAGES.filter(
      (lang) =>
        lang.value !== "auto" &&
        lang.label.toLowerCase().includes(languageSearch.toLowerCase()),
    );
  }, [languageSearch]);

  // Get selected language label
  const selectedLanguageLabel = useMemo(() => {
    if (languageFilter === "all") {
      return t("settings.models.filters.allLanguages");
    }
    return LANGUAGES.find((lang) => lang.value === languageFilter)?.label || "";
  }, [languageFilter, t]);

  const getModelStatus = (modelId: string): ModelCardStatus => {
    if (modelId in extractingModels) {
      return "extracting";
    }
    if (modelId in verifyingModels) {
      return "verifying";
    }
    if (modelId in downloadingModels) {
      return "downloading";
    }
    if (switchingModelId === modelId) {
      return "switching";
    }
    const model = models.find((m: ModelInfo) => m.id === modelId);
    const category = model ? getModelCategory(model) : "stt";
    // A model that isn't on disk can never be "active" — guard this first so a
    // stale selection (e.g. an LLM download that failed after being chosen)
    // shows as downloadable again instead of a dead-end "active" card with no
    // way to (re)download it.
    if (!model?.is_downloaded) {
      return "downloadable";
    }
    // STT uses the recording model; LLM uses the built-in assistant model.
    if (category === "stt" && modelId === currentModel) {
      return "active";
    }
    if (category === "llm" && modelId === activeLlmId) {
      return "active";
    }
    return "available";
  };

  const getDownloadProgress = (modelId: string): number | undefined => {
    const progress = downloadProgress[modelId];
    return progress?.percentage;
  };

  const getDownloadSpeed = (modelId: string): number | undefined => {
    const stats = downloadStats[modelId];
    return stats?.speed;
  };

  const handleModelSelect = async (modelId: string) => {
    const model = models.find((m: ModelInfo) => m.id === modelId);
    const category = model ? getModelCategory(model) : "stt";
    // TTS (Kokoro) has no "active" selection here — it is configured per
    // engine in the Assistant tab.
    if (category === "tts") return;

    setSwitchingModelId(modelId);
    try {
      if (category === "llm") {
        // Never point the assistant at a model that isn't on disk — that's the
        // state that used to strand users. If somehow triggered for a missing
        // model, download it instead of marking it active.
        if (!model?.is_downloaded) {
          await downloadModel(modelId);
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
    await downloadModel(modelId);
  };

  const handleModelDelete = async (modelId: string) => {
    const model = models.find((m: ModelInfo) => m.id === modelId);
    const modelName = model?.name || modelId;
    const category = model ? getModelCategory(model) : "stt";
    const isActive =
      category === "llm" ? modelId === activeLlmId : modelId === currentModel;

    const confirmed = await ask(
      isActive
        ? t("settings.models.deleteActiveConfirm", { modelName })
        : t("settings.models.deleteConfirm", { modelName }),
      {
        title: t("settings.models.deleteTitle"),
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

  // Filter models by active category, then (for transcription) language, then
  // by the free-text model-name search (matches the translated display name and
  // the raw catalog name so either works).
  const filteredModels = useMemo(() => {
    const query = nameSearch.trim().toLowerCase();
    return models.filter((model: ModelInfo) => {
      if (getModelCategory(model) !== categoryFilter) return false;
      if (categoryFilter === "stt" && languageFilter !== "all") {
        if (!modelSupportsLanguage(model, languageFilter)) return false;
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
  }, [models, languageFilter, categoryFilter, nameSearch, t]);

  // Split filtered models into: downloaded (incl. custom), available-to-download
  // (recommended / modern), and — for transcription only — an "older models"
  // group for not-yet-downloaded old-engine (transcribe-rs) models, which is
  // collapsed by default so a first-timer only sees the recommended few.
  const { downloadedModels, availableModels, olderModels } = useMemo(() => {
    const downloaded: ModelInfo[] = [];
    const available: ModelInfo[] = [];
    const older: ModelInfo[] = [];

    for (const model of filteredModels) {
      const onDisk =
        model.is_custom ||
        model.is_downloaded ||
        model.id in downloadingModels ||
        model.id in extractingModels;
      if (onDisk) {
        // Downloaded models always stay in "Downloaded" (they're relevant
        // regardless of engine); a quiet card tag marks the older ones.
        downloaded.push(model);
      } else if (categoryFilter === "stt" && isLegacyModel(model)) {
        older.push(model);
      } else {
        available.push(model);
      }
    }

    // The "active" model depends on the category: STT uses the recording
    // model, LLM uses the built-in assistant model.
    const activeIdForCategory =
      categoryFilter === "llm" ? activeLlmId : currentModel;

    // Recommendation order key: ranked-recommended first (by rank, 1 = top),
    // then recommended-without-rank, then everything else. Mirrors Handy's
    // catalog ordering so the new streaming models lead the list.
    const rankOf = (m: ModelInfo): number =>
      m.is_recommended ? (m.recommended_rank ?? 1_000) : 10_000;

    // Sort: active model first, then by recommendation order, then non-custom
    // before custom, with accuracy as the final tie-breaker.
    downloaded.sort((a, b) => {
      if (a.id === activeIdForCategory) return -1;
      if (b.id === activeIdForCategory) return 1;
      if (a.is_custom !== b.is_custom) return a.is_custom ? 1 : -1;
      const rankDiff = rankOf(a) - rankOf(b);
      if (rankDiff !== 0) return rankDiff;
      return b.accuracy_score - a.accuracy_score;
    });

    // Available (not-yet-downloaded) models: same recommendation order so the
    // recommended set surfaces at the top, then most-accurate first.
    available.sort((a, b) => {
      const rankDiff = rankOf(a) - rankOf(b);
      if (rankDiff !== 0) return rankDiff;
      return b.accuracy_score - a.accuracy_score;
    });

    // Older (legacy) models: most-accurate first — no recommendation ranking.
    older.sort((a, b) => b.accuracy_score - a.accuracy_score);

    return {
      downloadedModels: downloaded,
      availableModels: available,
      olderModels: older,
    };
  }, [
    filteredModels,
    downloadingModels,
    extractingModels,
    currentModel,
    activeLlmId,
    categoryFilter,
  ]);

  // Auto-expand the "Older models" group while searching so a name search can
  // reach a legacy model without the user first opening the section.
  const isSearching = nameSearch.trim() !== "";
  const olderModelsOpen = showOlderModels || isSearching;
  const hasAnyResults =
    downloadedModels.length > 0 ||
    availableModels.length > 0 ||
    olderModels.length > 0;

  const renderModelCard = (model: ModelInfo) => (
    <ModelCard
      key={model.id}
      model={model}
      status={getModelStatus(model.id)}
      onSelect={handleModelSelect}
      onDownload={handleModelDownload}
      onDelete={categoryFilter === "tts" ? undefined : handleModelDelete}
      onCancel={handleModelCancel}
      downloadProgress={getDownloadProgress(model.id)}
      downloadSpeed={getDownloadSpeed(model.id)}
      showRecommended={true}
    />
  );

  if (loading) {
    return (
      <div className="max-w-2xl w-full mx-auto">
        <div className="flex items-center justify-center py-16">
          <div className="w-8 h-8 border-2 border-hairline-strong border-t-ink rounded-full animate-spin" />
        </div>
      </div>
    );
  }

  return (
    <div className="max-w-2xl w-full mx-auto space-y-6">
      {/* Category switcher: Transcription / Language Model / Speech */}
      <div className="flex items-center gap-0.5 p-0.5 bg-surface-strong rounded-lg w-fit">
        {CATEGORY_TABS.map((cat) => (
          <button
            key={cat}
            type="button"
            onClick={() => setCategoryFilter(cat)}
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
      {/* Toolbar: model-name search + (stt) language filter / (llm) add custom.
          Kept above the sections so it's always reachable, even with no results. */}
      <div className="flex items-center gap-2">
        <div className="flex-1 flex items-center gap-2 px-3 py-2 bg-surface border border-hairline rounded-lg focus-within:border-ink transition-colors">
          <Search className="w-4 h-4 shrink-0 text-muted-soft" />
          <input
            type="text"
            value={nameSearch}
            onChange={(e) => setNameSearch(e.target.value)}
            placeholder={t("settings.models.searchPlaceholder")}
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
        {/* Language filter dropdown (transcription models only) */}
        {categoryFilter === "stt" && (
          <div className="relative" ref={languageDropdownRef}>
            <button
              type="button"
              onClick={() => setLanguageDropdownOpen(!languageDropdownOpen)}
              className={`flex items-center gap-1.5 px-3 py-2 text-[13px] font-medium rounded-lg transition-colors cursor-pointer ${
                languageFilter !== "all"
                  ? "bg-accent/10 text-accent"
                  : "bg-surface-strong text-muted hover:text-ink"
              }`}
            >
              <Globe className="w-3.5 h-3.5" />
              <span className="max-w-[120px] truncate">
                {selectedLanguageLabel}
              </span>
              <ChevronDown
                className={`w-3.5 h-3.5 transition-transform ${
                  languageDropdownOpen ? "rotate-180" : ""
                }`}
              />
            </button>

            {languageDropdownOpen && (
              <div className="absolute top-full end-0 mt-1 w-56 bg-surface border border-hairline rounded-xl shadow-lg z-50 overflow-hidden">
                <div className="p-2 border-b border-hairline">
                  <input
                    ref={languageSearchInputRef}
                    type="text"
                    value={languageSearch}
                    onChange={(e) => setLanguageSearch(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && filteredLanguages.length > 0) {
                        setLanguageFilter(filteredLanguages[0].value);
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
                      setLanguageFilter("all");
                      setLanguageDropdownOpen(false);
                      setLanguageSearch("");
                    }}
                    className={`w-full px-3 py-1.5 text-sm text-left rounded-lg transition-colors ${
                      languageFilter === "all"
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
                        setLanguageFilter(lang.value);
                        setLanguageDropdownOpen(false);
                        setLanguageSearch("");
                      }}
                      className={`w-full px-3 py-1.5 text-sm text-left rounded-lg transition-colors ${
                        languageFilter === lang.value
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
        )}
        {categoryFilter === "llm" && (
          <Button
            variant="secondary"
            size="sm"
            onClick={() => setAddDialogOpen(true)}
          >
            <Plus className="w-4 h-4" />
            {t("settings.models.customModel.addButton")}
          </Button>
        )}
      </div>

      {hasAnyResults ? (
        <div className="space-y-8">
          {/* Downloaded Models Section */}
          {downloadedModels.length > 0 && (
            <div className="space-y-3">
              <h2 className="text-[13px] font-semibold text-ink">
                {t("settings.models.yourModels")}
              </h2>
              {downloadedModels.map(renderModelCard)}
            </div>
          )}

          {/* Available Models Section (recommended / modern) */}
          {availableModels.length > 0 && (
            <div className="space-y-3">
              <h2 className="text-[13px] font-semibold text-ink">
                {t("settings.models.availableModels")}
              </h2>
              {availableModels.map(renderModelCard)}
            </div>
          )}

          {/* Older models (legacy engine) — collapsed by default, auto-open on search */}
          {categoryFilter === "stt" && olderModels.length > 0 && (
            <div className="space-y-3">
              <button
                type="button"
                onClick={() => setShowOlderModels((v) => !v)}
                aria-expanded={olderModelsOpen}
                className="flex items-center gap-1.5 text-[13px] font-semibold text-muted hover:text-ink transition-colors cursor-pointer"
              >
                <ChevronRight
                  className={`w-3.5 h-3.5 transition-transform ${
                    olderModelsOpen ? "rotate-90" : ""
                  }`}
                />
                {t("settings.models.olderModels")}
                <span className="font-normal text-muted-soft">
                  ({olderModels.length})
                </span>
              </button>
              {olderModelsOpen && (
                <div className="space-y-3">
                  <p className="text-xs text-muted-soft">
                    {t("settings.models.olderModelsHint")}
                  </p>
                  {olderModels.map(renderModelCard)}
                </div>
              )}
            </div>
          )}
        </div>
      ) : (
        <div className="text-center py-8 text-text/50">
          {isSearching
            ? t("settings.models.noSearchResults", { query: nameSearch.trim() })
            : t("settings.models.noModelsMatch")}
        </div>
      )}
      <AddCustomModelDialog
        open={addDialogOpen}
        onClose={() => setAddDialogOpen(false)}
      />
    </div>
  );
};
