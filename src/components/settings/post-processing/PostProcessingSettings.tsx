import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pencil, RefreshCcw } from "lucide-react";
import { commands, type PostProcessTone } from "@/bindings";

import { Alert } from "../../ui/Alert";
import { Dropdown, SettingContainer, Textarea } from "@/components/ui";
import { Button } from "../../ui/Button";
import { ResetButton } from "../../ui/ResetButton";
import { Input } from "../../ui/Input";
import { useModelStore } from "@/stores/modelStore";
import { getModelCategory } from "@/lib/utils/modelCategory";

import { ProviderSelect } from "../PostProcessingSettingsApi/ProviderSelect";
import { BaseUrlField } from "../PostProcessingSettingsApi/BaseUrlField";
import { ApiKeyField } from "../PostProcessingSettingsApi/ApiKeyField";
import { ModelSelect } from "../PostProcessingSettingsApi/ModelSelect";
import { usePostProcessProviderState } from "../PostProcessingSettingsApi/usePostProcessProviderState";
import { useSettings } from "../../../hooks/useSettings";

const PostProcessingSettingsApiComponent: React.FC = () => {
  const { t } = useTranslation();
  const state = usePostProcessProviderState();

  // Built-in (Local) provider: no API key, and the model is picked from the
  // LLMs already downloaded in the Models tab — never a hand-typed name or an
  // API key. Mirrors the Assistant provider UI so the two behave identically.
  const isBuiltin = state.selectedProvider?.id === "builtin";
  const { models } = useModelStore();
  const llmModels = useMemo(
    () =>
      models.filter((m) => getModelCategory(m) === "llm" && m.is_downloaded),
    [models],
  );

  return (
    <>
      <SettingContainer
        title={t("settings.postProcessing.api.provider.title")}
        description={t("settings.postProcessing.api.provider.description")}
        descriptionMode="tooltip"
        layout="horizontal"
        grouped={true}
      >
        <div className="flex items-center gap-2">
          <ProviderSelect
            options={state.providerOptions}
            value={state.selectedProviderId}
            onChange={state.handleProviderSelect}
          />
        </div>
      </SettingContainer>

      {state.isAppleProvider ? (
        state.appleIntelligenceUnavailable ? (
          <Alert variant="error" contained>
            {t("settings.postProcessing.api.appleIntelligence.unavailable")}
          </Alert>
        ) : null
      ) : (
        <>
          {state.selectedProvider?.allow_base_url_edit && (
            <SettingContainer
              title={t("settings.postProcessing.api.baseUrl.title")}
              description={t("settings.postProcessing.api.baseUrl.description")}
              descriptionMode="tooltip"
              layout="horizontal"
              grouped={true}
            >
              <div className="flex items-center gap-2">
                <BaseUrlField
                  value={state.baseUrl}
                  onBlur={state.handleBaseUrlChange}
                  placeholder={t(
                    "settings.postProcessing.api.baseUrl.placeholder",
                  )}
                  disabled={state.isBaseUrlUpdating}
                  className="min-w-[380px]"
                />
              </div>
            </SettingContainer>
          )}

          {!isBuiltin && (
            <SettingContainer
              title={t("settings.postProcessing.api.apiKey.title")}
              description={t("settings.postProcessing.api.apiKey.description")}
              descriptionMode="tooltip"
              layout="horizontal"
              grouped={true}
            >
              <div className="flex items-center gap-2">
                <ApiKeyField
                  value={state.apiKey}
                  onBlur={state.handleApiKeyChange}
                  placeholder={t(
                    "settings.postProcessing.api.apiKey.placeholder",
                  )}
                  disabled={state.isApiKeyUpdating}
                  className="min-w-[320px]"
                />
              </div>
            </SettingContainer>
          )}
        </>
      )}

      {!state.isAppleProvider &&
        (isBuiltin ? (
          <SettingContainer
            title={t("settings.postProcessing.api.model.title")}
            description={t(
              "settings.postProcessing.api.builtin.modelDescription",
            )}
            descriptionMode="tooltip"
            layout="horizontal"
            grouped={true}
          >
            <div className="flex flex-col items-end gap-1">
              {llmModels.length > 0 ? (
                <Dropdown
                  options={llmModels.map((m) => ({
                    value: m.id,
                    label: m.name,
                  }))}
                  selectedValue={state.model}
                  onSelect={(value) => state.handleModelSelect(value)}
                  placeholder={t(
                    "settings.postProcessing.api.builtin.modelPlaceholder",
                  )}
                  className="min-w-[320px]"
                />
              ) : (
                <span className="text-xs text-mid-gray/70 max-w-[360px] text-right">
                  {t("settings.postProcessing.api.builtin.noModels")}
                </span>
              )}
              <span className="text-xs text-mid-gray/70 max-w-[360px] text-right">
                {t("settings.postProcessing.api.builtin.ready")}
              </span>
            </div>
          </SettingContainer>
        ) : (
          <SettingContainer
            title={t("settings.postProcessing.api.model.title")}
            description={
              state.isCustomProvider
                ? t("settings.postProcessing.api.model.descriptionCustom")
                : t("settings.postProcessing.api.model.descriptionDefault")
            }
            descriptionMode="tooltip"
            layout="stacked"
            grouped={true}
          >
            <div className="flex items-center gap-2">
              <ModelSelect
                value={state.model}
                options={state.modelOptions}
                disabled={state.isModelUpdating}
                isLoading={state.isFetchingModels}
                placeholder={
                  state.modelOptions.length > 0
                    ? t(
                        "settings.postProcessing.api.model.placeholderWithOptions",
                      )
                    : t(
                        "settings.postProcessing.api.model.placeholderNoOptions",
                      )
                }
                onSelect={state.handleModelSelect}
                onCreate={state.handleModelCreate}
                onBlur={() => {}}
                className="flex-1 min-w-[380px]"
              />
              <ResetButton
                onClick={state.handleRefreshModels}
                disabled={state.isFetchingModels}
                ariaLabel={t("settings.postProcessing.api.model.refreshModels")}
                className="flex h-10 w-10 items-center justify-center"
              >
                <RefreshCcw
                  className={`h-4 w-4 ${state.isFetchingModels ? "animate-spin" : ""}`}
                />
              </ResetButton>
            </div>
          </SettingContainer>
        ))}
    </>
  );
};

const PostProcessingSettingsPromptsComponent: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating, refreshSettings } =
    useSettings();
  const [isCreating, setIsCreating] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draftName, setDraftName] = useState("");
  const [draftText, setDraftText] = useState("");

  const prompts = getSetting("post_process_prompts") || [];
  const selectedPromptId = getSetting("post_process_selected_prompt_id") || "";
  const selectedPrompt =
    prompts.find((prompt) => prompt.id === selectedPromptId) || null;

  useEffect(() => {
    if (isCreating) return;

    if (selectedPrompt) {
      setDraftName(selectedPrompt.name);
      setDraftText(selectedPrompt.prompt);
    } else {
      setDraftName("");
      setDraftText("");
    }
  }, [
    isCreating,
    selectedPromptId,
    selectedPrompt?.name,
    selectedPrompt?.prompt,
  ]);

  const handlePromptSelect = (promptId: string | null) => {
    if (!promptId) return;
    updateSetting("post_process_selected_prompt_id", promptId);
    setIsCreating(false);
  };

  const handleCreatePrompt = async () => {
    if (!draftName.trim() || !draftText.trim()) return;

    try {
      const result = await commands.addPostProcessPrompt(
        draftName.trim(),
        draftText.trim(),
      );
      if (result.status === "ok") {
        await refreshSettings();
        updateSetting("post_process_selected_prompt_id", result.data.id);
        setIsCreating(false);
      }
    } catch (error) {
      console.error("Failed to create prompt:", error);
    }
  };

  const handleUpdatePrompt = async () => {
    if (!selectedPromptId || !draftName.trim() || !draftText.trim()) return;

    try {
      await commands.updatePostProcessPrompt(
        selectedPromptId,
        draftName.trim(),
        draftText.trim(),
      );
      await refreshSettings();
      setEditing(false);
    } catch (error) {
      console.error("Failed to update prompt:", error);
    }
  };

  const handleDeletePrompt = async (promptId: string) => {
    if (!promptId) return;

    try {
      await commands.deletePostProcessPrompt(promptId);
      await refreshSettings();
      setIsCreating(false);
      setEditing(false);
    } catch (error) {
      console.error("Failed to delete prompt:", error);
    }
  };

  const handleCancelCreate = () => {
    setIsCreating(false);
    if (selectedPrompt) {
      setDraftName(selectedPrompt.name);
      setDraftText(selectedPrompt.prompt);
    } else {
      setDraftName("");
      setDraftText("");
    }
  };

  const handleStartCreate = () => {
    setIsCreating(true);
    setDraftName("");
    setDraftText("");
  };

  const hasPrompts = prompts.length > 0;
  const isDirty =
    !!selectedPrompt &&
    (draftName.trim() !== selectedPrompt.name ||
      draftText.trim() !== selectedPrompt.prompt.trim());

  const fieldLabelClasses =
    "block text-[11px] font-medium uppercase tracking-wide text-muted";
  // The full editor (label + big instructions textarea) only appears when the
  // user explicitly asks for it — the default view is just the picker row.
  const showEditor = isCreating || (editing && hasPrompts && !!selectedPrompt);

  return (
    <SettingContainer
      title={t("settings.postProcessing.prompts.selectedPrompt.title")}
      description={t(
        "settings.postProcessing.prompts.selectedPrompt.description",
      )}
      layout="stacked"
      grouped={true}
    >
      <div className="space-y-4">
        <div className="flex gap-2">
          <Dropdown
            selectedValue={selectedPromptId || null}
            options={prompts.map((p) => ({
              value: p.id,
              label: p.name,
            }))}
            onSelect={(value) => handlePromptSelect(value)}
            placeholder={
              prompts.length === 0
                ? t("settings.postProcessing.prompts.noPrompts")
                : t("settings.postProcessing.prompts.selectPrompt")
            }
            disabled={
              isUpdating("post_process_selected_prompt_id") || isCreating
            }
            className="flex-1"
          />
          {!isCreating && selectedPrompt && (
            <Button
              onClick={() => setEditing((v) => !v)}
              variant="secondary"
              size="md"
            >
              <Pencil size={14} />
              {editing
                ? t("settings.postProcessing.prompts.closeEditor")
                : t("settings.postProcessing.prompts.edit")}
            </Button>
          )}
          <Button
            onClick={handleStartCreate}
            variant="secondary"
            size="md"
            disabled={isCreating}
          >
            {t("settings.postProcessing.prompts.createNew")}
          </Button>
        </div>

        {showEditor && (
          <div className="space-y-4">
            <div className="space-y-1.5">
              <label className={fieldLabelClasses}>
                {t("settings.postProcessing.prompts.promptLabel")}
              </label>
              <Input
                type="text"
                value={draftName}
                onChange={(e) => setDraftName(e.target.value)}
                placeholder={t(
                  "settings.postProcessing.prompts.promptLabelPlaceholder",
                )}
                variant="compact"
                className="w-full"
              />
            </div>

            <div className="space-y-1.5">
              <label className={fieldLabelClasses}>
                {t("settings.postProcessing.prompts.promptInstructions")}
              </label>
              <Textarea
                value={draftText}
                onChange={(e) => setDraftText(e.target.value)}
                rows={8}
                className="w-full"
                placeholder={t(
                  "settings.postProcessing.prompts.promptInstructionsPlaceholder",
                )}
              />
            </div>

            <div className="flex items-center gap-2">
              {isCreating ? (
                <>
                  <Button
                    onClick={handleCreatePrompt}
                    variant="primary"
                    size="md"
                    disabled={!draftName.trim() || !draftText.trim()}
                  >
                    {t("settings.postProcessing.prompts.createPrompt")}
                  </Button>
                  <Button
                    onClick={handleCancelCreate}
                    variant="secondary"
                    size="md"
                  >
                    {t("settings.postProcessing.prompts.cancel")}
                  </Button>
                </>
              ) : (
                <>
                  <Button
                    onClick={handleUpdatePrompt}
                    variant="primary"
                    size="md"
                    disabled={
                      !draftName.trim() || !draftText.trim() || !isDirty
                    }
                  >
                    {t("settings.postProcessing.prompts.updatePrompt")}
                  </Button>
                  <Button
                    onClick={() => handleDeletePrompt(selectedPromptId)}
                    variant="secondary"
                    size="md"
                    disabled={!selectedPromptId || prompts.length <= 1}
                  >
                    {t("settings.postProcessing.prompts.deletePrompt")}
                  </Button>
                </>
              )}
            </div>
          </div>
        )}

        {!isCreating && !selectedPrompt && (
          <p className="text-[13px] text-muted">
            {hasPrompts
              ? t("settings.postProcessing.prompts.selectToEdit")
              : t("settings.postProcessing.prompts.createFirst")}
          </p>
        )}
      </div>
    </SettingContainer>
  );
};

export const PostProcessingSettingsApi = React.memo(
  PostProcessingSettingsApiComponent,
);
PostProcessingSettingsApi.displayName = "PostProcessingSettingsApi";

export const PostProcessingSettingsPrompts = React.memo(
  PostProcessingSettingsPromptsComponent,
);
PostProcessingSettingsPrompts.displayName = "PostProcessingSettingsPrompts";

const PostProcessingToneComponent: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();

  const tone = getSetting("post_process_tone") ?? "none";

  const options = (
    [
      "none",
      "formal",
      "casual",
      "professional",
      "friendly",
      "concise",
    ] as PostProcessTone[]
  ).map((value) => ({
    value,
    label: t(`settings.postProcessing.tone.options.${value}`),
  }));

  return (
    <SettingContainer
      title={t("settings.postProcessing.tone.title")}
      description={t("settings.postProcessing.tone.description")}
      descriptionMode="tooltip"
      layout="horizontal"
      grouped={true}
    >
      <Dropdown
        selectedValue={tone}
        options={options}
        onSelect={(value) =>
          updateSetting("post_process_tone", value as PostProcessTone)
        }
      />
    </SettingContainer>
  );
};

export const PostProcessingTone = React.memo(PostProcessingToneComponent);
PostProcessingTone.displayName = "PostProcessingTone";
