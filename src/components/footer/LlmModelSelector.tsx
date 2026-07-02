import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Bot, ChevronDown } from "lucide-react";
import { commands } from "@/bindings";
import { useModelStore } from "@/stores/modelStore";
import { useSettings } from "@/hooks/useSettings";
import { getModelCategory } from "@/lib/utils/modelCategory";
import { getTranslatedModelName } from "@/lib/utils/modelTranslation";
import { ModelDropdown } from "../model-selector";

/**
 * Footer control for picking the local (built-in) assistant LLM, mirroring the
 * transcription ModelSelector. Lists downloaded LLM models and writes the
 * choice to the built-in provider's model. It only renders when at least one
 * local LLM model is downloaded — remote providers use a free-text model field
 * in the Assistant settings instead.
 */
const BUILTIN_PROVIDER_ID = "builtin";

const LlmModelSelector: React.FC = () => {
  const { t } = useTranslation();
  const { models } = useModelStore();
  const { settings, refreshSettings } = useSettings();

  const [open, setOpen] = useState(false);
  // Optimistic selection so the label updates immediately on click.
  const [pendingModelId, setPendingModelId] = useState<string | null>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const llmModels = useMemo(
    () =>
      models.filter((m) => getModelCategory(m) === "llm" && m.is_downloaded),
    [models],
  );

  const storedModelId = settings?.assistant_models?.[BUILTIN_PROVIDER_ID] ?? "";
  const currentModelId = pendingModelId ?? storedModelId;
  const currentModel = llmModels.find((m) => m.id === currentModelId);

  // Clear the optimistic value once settings catch up.
  useEffect(() => {
    if (pendingModelId && storedModelId === pendingModelId) {
      setPendingModelId(null);
    }
  }, [storedModelId, pendingModelId]);

  // Close the dropdown when clicking outside of it.
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (
        dropdownRef.current &&
        !dropdownRef.current.contains(event.target as Node)
      ) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  if (llmModels.length === 0) {
    return null;
  }

  const handleModelSelect = async (modelId: string) => {
    setPendingModelId(modelId);
    setOpen(false);
    await commands.changeAssistantModelSetting(BUILTIN_PROVIDER_ID, modelId);
    // Activate the local provider so the selection actually takes effect,
    // matching how picking a transcription model switches the active engine.
    if (settings?.assistant_provider_id !== BUILTIN_PROVIDER_ID) {
      await commands.setAssistantProvider(BUILTIN_PROVIDER_ID);
    }
    await refreshSettings();
  };

  const label = currentModel
    ? getTranslatedModelName(currentModel, t)
    : t("modelSelector.llmSelectModel");

  return (
    <div className="relative" ref={dropdownRef}>
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 hover:text-text/80 transition-colors"
        title={t("modelSelector.llmModelTitle", { model: label })}
      >
        <Bot className="w-3.5 h-3.5 shrink-0" />
        <span className="max-w-28 truncate">{label}</span>
        <ChevronDown
          className={`w-3 h-3 transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>

      {open && (
        <ModelDropdown
          models={llmModels}
          currentModelId={currentModelId}
          onModelSelect={handleModelSelect}
        />
      )}
    </div>
  );
};

export default LlmModelSelector;
