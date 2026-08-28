import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pencil, RotateCcw } from "lucide-react";
import { toast } from "sonner";
import { commands, type CustomPostProcessTone } from "@/bindings";

import { Alert } from "../../ui/Alert";
import { Dropdown, SettingContainer, Textarea } from "@/components/ui";
import { Button } from "../../ui/Button";
import { Input } from "../../ui/Input";
import { useModelStore } from "@/stores/modelStore";
import { getModelCategory } from "@/lib/utils/modelCategory";

import { ProviderModeToggle } from "../PostProcessingSettingsApi/ProviderModeToggle";
import { ProviderSelect } from "../PostProcessingSettingsApi/ProviderSelect";
import { BaseUrlField } from "../PostProcessingSettingsApi/BaseUrlField";
import { ApiKeyField } from "../PostProcessingSettingsApi/ApiKeyField";
import { ModelCombo } from "../../ui/ModelCombo";
import { CleanupModelRow } from "../dictation/CleanupModelRow";
import { usePostProcessProviderState } from "../PostProcessingSettingsApi/usePostProcessProviderState";
import { useSettings } from "../../../hooks/useSettings";

/**
 * Where cleanup runs, and which model it uses.
 *
 * `onBrowseModels` is required for the device side: picking an on-device model is
 * a browse-and-download flow, not a dropdown, and the page that owns this
 * component owns the sub-page it opens.
 */
const PostProcessingSettingsApiComponent: React.FC<{
  onBrowseModels: () => void;
}> = ({ onBrowseModels }) => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const state = usePostProcessProviderState();
  const isBuiltin = state.selectedProvider?.id === "builtin";
  const providerMode = isBuiltin ? "device" : "cloud";
  const cloudProviderOptions = useMemo(
    () => state.providerOptions.filter((option) => option.value !== "builtin"),
    [state.providerOptions],
  );

  // Which cloud provider to return to, read from settings rather than local
  // state. It used to be `useState`, which meant the choice survived a toggle
  // but not a navigation away and back: the user returned, switched to cloud,
  // and landed on a different provider — so their model, base URL, and key all
  // looked lost even though they were still stored per provider.
  const lastCloudProviderId =
    settings?.post_process_last_cloud_provider_id ?? null;

  const handleProviderModeChange = (mode: "device" | "cloud") => {
    if (mode === "device") {
      if (!isBuiltin) state.handleProviderSelect("builtin");
      return;
    }
    if (!isBuiltin) return;

    const target =
      lastCloudProviderId &&
      cloudProviderOptions.some(
        (option) => option.value === lastCloudProviderId,
      )
        ? lastCloudProviderId
        : cloudProviderOptions[0]?.value;
    if (target) state.handleProviderSelect(target);
  };

  const { models } = useModelStore();
  // The on-device cleanup model, resolved to a real catalog entry. Only a model
  // that is actually on disk counts as active — a selection whose file is gone
  // has to read as "nothing selected" or the card would claim cleanup is ready
  // when it cannot run.
  const activeLocalModel = useMemo(() => {
    const id = settings?.post_process_models?.builtin ?? "";
    if (!id) return undefined;
    return models.find(
      (model) =>
        model.id === id &&
        getModelCategory(model) === "llm" &&
        model.is_downloaded,
    );
  }, [models, settings?.post_process_models]);

  return (
    <>
      <SettingContainer
        title={t("settings.postProcessing.api.location.title")}
        description={t("settings.postProcessing.api.location.description")}
        descriptionMode="tooltip"
        // Horizontal: a two-option segmented control is narrow enough to sit
        // beside its label, and stacking it cost a whole row of height for
        // nothing.
        layout="horizontal"
        grouped={true}
      >
        <ProviderModeToggle
          mode={providerMode}
          onChange={handleProviderModeChange}
          disabled={state.isProviderUpdating}
        />
      </SettingContainer>

      {providerMode === "cloud" && (
        <SettingContainer
          title={t("settings.postProcessing.api.provider.title")}
          description={t("settings.postProcessing.api.provider.description")}
          descriptionMode="tooltip"
          layout="horizontal"
          grouped={true}
        >
          <ProviderSelect
            options={cloudProviderOptions}
            value={state.selectedProviderId}
            onChange={state.handleProviderSelect}
            disabled={state.isProviderUpdating}
          />
        </SettingContainer>
      )}

      {state.isAppleProvider ? (
        state.appleIntelligenceUnavailable ? (
          <Alert variant="error" contained>
            {t("settings.postProcessing.api.appleIntelligence.unavailable")}
          </Alert>
        ) : null
      ) : providerMode === "cloud" ? (
        <>
          {state.selectedProvider?.allow_base_url_edit && (
            <SettingContainer
              title={t("settings.postProcessing.api.baseUrl.title")}
              description={t("settings.postProcessing.api.baseUrl.description")}
              descriptionMode="tooltip"
              layout="horizontal"
              grouped={true}
            >
              <BaseUrlField
                value={state.baseUrl}
                onBlur={state.handleBaseUrlChange}
                placeholder={t(
                  "settings.postProcessing.api.baseUrl.placeholder",
                )}
                disabled={state.isBaseUrlUpdating}
                className="min-w-[380px]"
              />
            </SettingContainer>
          )}

          <SettingContainer
            title={t("settings.postProcessing.api.apiKey.title")}
            description={t("settings.postProcessing.api.apiKey.description")}
            descriptionMode="tooltip"
            layout="horizontal"
            grouped={true}
          >
            <ApiKeyField
              value={state.apiKey}
              onBlur={state.handleApiKeyChange}
              placeholder={t("settings.postProcessing.api.apiKey.placeholder")}
              disabled={state.isApiKeyUpdating}
              className="min-w-[320px]"
            />
          </SettingContainer>
        </>
      ) : null}

      {!state.isAppleProvider &&
        (providerMode === "device" ? (
          <CleanupModelRow
            model={activeLocalModel}
            onChangeModel={onBrowseModels}
          />
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
            <ModelCombo
              value={state.model}
              options={state.modelOptions}
              onCommit={state.handleModelChange}
              onLoad={state.handleRefreshModels}
              loading={state.isFetchingModels}
              disabled={state.isModelUpdating}
              placeholder={
                state.modelOptions.length > 0
                  ? t(
                      "settings.postProcessing.api.model.placeholderWithOptions",
                    )
                  : t("settings.postProcessing.api.model.placeholderNoOptions")
              }
              loadLabel={t("settings.postProcessing.api.model.refreshModels")}
              className="flex flex-col gap-1"
              inputClassName="min-w-0 flex-1"
            />
          </SettingContainer>
        ))}
    </>
  );
};

/**
 * Sentinel selection meaning "no cleanup prompt", mirrored from
 * `settings::NONE_POST_PROCESS_PROMPT_ID`. Cleanup still runs; the model just
 * receives the transcript and the final-output contract, nothing else.
 */
const NONE_PROMPT_ID = "none";

/**
 * The prompts the app ships, mirrored from `settings.rs`. Only these can be
 * restored to their original text — a user's own prompt has no "original".
 */
const SHIPPED_PROMPT_IDS = new Set([
  "default_improve_transcriptions",
  "speakoflow_mini_cleanup",
]);

const PostProcessingSettingsPromptsComponent: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating, refreshSettings } =
    useSettings();
  const [isCreating, setIsCreating] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draftName, setDraftName] = useState("");
  const [draftText, setDraftText] = useState("");
  const [isPromptBusy, setIsPromptBusy] = useState(false);
  const promptNameInputId = React.useId();
  const promptInstructionInputId = React.useId();

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
    if (!promptId || isPromptBusy) return;
    void updateSetting("post_process_selected_prompt_id", promptId);
    setIsCreating(false);
  };

  const handleCreatePrompt = async () => {
    if (!draftName.trim() || !draftText.trim() || isPromptBusy) return;

    setIsPromptBusy(true);
    try {
      const result = await commands.addPostProcessPrompt(
        draftName.trim(),
        draftText.trim(),
      );
      if (result.status !== "ok") {
        toast.error(
          t("settings.postProcessing.errors.promptCreateFailed", {
            defaultValue: "Couldn’t create the prompt.",
          }),
        );
        return;
      }
      // Select first, THEN do a single authoritative refresh. If selection
      // fails, keep the created prompt (refresh so it appears) but leave it
      // unselected and close create mode to avoid recreating it.
      const selected = await commands.setPostProcessSelectedPrompt(
        result.data.id,
      );
      if (selected.status !== "ok") {
        toast.error(
          t("settings.postProcessing.errors.promptSelectFailed", {
            defaultValue: "Created the prompt, but couldn’t select it.",
          }),
        );
      }
      await refreshSettings();
      setIsCreating(false);
    } catch (error) {
      console.error("Failed to create prompt:", error);
      toast.error(
        t("settings.postProcessing.errors.promptCreateFailed", {
          defaultValue: "Couldn’t create the prompt.",
        }),
      );
    } finally {
      setIsPromptBusy(false);
    }
  };

  const handleUpdatePrompt = async () => {
    if (
      !selectedPromptId ||
      !draftName.trim() ||
      !draftText.trim() ||
      isPromptBusy
    )
      return;

    setIsPromptBusy(true);
    try {
      const result = await commands.updatePostProcessPrompt(
        selectedPromptId,
        draftName.trim(),
        draftText.trim(),
      );
      if (result.status !== "ok") {
        toast.error(
          t("settings.postProcessing.errors.promptUpdateFailed", {
            defaultValue: "Couldn’t update the prompt.",
          }),
        );
        await refreshSettings();
        return;
      }
      await refreshSettings();
      setEditing(false);
    } catch (error) {
      console.error("Failed to update prompt:", error);
      toast.error(
        t("settings.postProcessing.errors.promptUpdateFailed", {
          defaultValue: "Couldn’t update the prompt.",
        }),
      );
    } finally {
      setIsPromptBusy(false);
    }
  };

  const handleDeletePrompt = async (promptId: string) => {
    if (!promptId || isPromptBusy) return;

    setIsPromptBusy(true);
    try {
      const result = await commands.deletePostProcessPrompt(promptId);
      if (result.status !== "ok") {
        toast.error(
          t("settings.postProcessing.errors.promptDeleteFailed", {
            defaultValue: "Couldn’t delete the prompt.",
          }),
        );
        return;
      }
      // The backend repairs the selection to the bundled prompt; just read the
      // authoritative result rather than guessing a replacement here.
      await refreshSettings();
      setIsCreating(false);
      setEditing(false);
    } catch (error) {
      console.error("Failed to delete prompt:", error);
      toast.error(
        t("settings.postProcessing.errors.promptDeleteFailed", {
          defaultValue: "Couldn’t delete the prompt.",
        }),
      );
    } finally {
      setIsPromptBusy(false);
    }
  };

  /**
   * Put a shipped prompt back to the text the app ships.
   *
   * Matters most for the SpeakoFlow Mini prompt: it is editable like any other,
   * but it is also the exact string the model was trained against, so a user who
   * experimented with it needs a way back that doesn't involve retyping it from
   * the release notes.
   */
  const handleRestorePrompt = async () => {
    if (!selectedPromptId || isPromptBusy) return;
    setIsPromptBusy(true);
    try {
      const result = await commands.restorePostProcessPrompt(selectedPromptId);
      if (result.status !== "ok") {
        toast.error(
          t("settings.postProcessing.errors.promptRestoreFailed", {
            defaultValue: "Couldn’t restore the prompt.",
          }),
        );
        return;
      }
      await refreshSettings();
      setEditing(false);
    } catch (error) {
      console.error("Failed to restore prompt:", error);
      toast.error(
        t("settings.postProcessing.errors.promptRestoreFailed", {
          defaultValue: "Couldn’t restore the prompt.",
        }),
      );
    } finally {
      setIsPromptBusy(false);
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
            options={[
              // "No prompt" — cleanup still runs, but the model receives only
              // the transcript. Useful for judging a model (a fine-tune in
              // particular) on its own behaviour rather than on ours.
              {
                value: NONE_PROMPT_ID,
                label: t("settings.postProcessing.prompts.nonePrompt"),
              },
              ...prompts.map((p) => ({
                value: p.id,
                label: p.name,
              })),
            ]}
            onSelect={(value) => handlePromptSelect(value)}
            placeholder={
              prompts.length === 0
                ? t("settings.postProcessing.prompts.noPrompts")
                : t("settings.postProcessing.prompts.selectPrompt")
            }
            disabled={
              isUpdating("post_process_selected_prompt_id") ||
              isCreating ||
              isPromptBusy
            }
            className="flex-1"
          />
          {!isCreating && selectedPrompt && (
            <Button
              onClick={() => setEditing((v) => !v)}
              variant="secondary"
              size="md"
              disabled={isPromptBusy}
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
            disabled={isCreating || isPromptBusy}
          >
            {t("settings.postProcessing.prompts.createNew")}
          </Button>
        </div>

        {showEditor && (
          <div className="space-y-4">
            <div className="space-y-1.5">
              <label htmlFor={promptNameInputId} className={fieldLabelClasses}>
                {t("settings.postProcessing.prompts.promptLabel")}
              </label>
              <Input
                id={promptNameInputId}
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
              <label
                htmlFor={promptInstructionInputId}
                className={fieldLabelClasses}
              >
                {t("settings.postProcessing.prompts.promptInstructions")}
              </label>
              <Textarea
                id={promptInstructionInputId}
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
                    disabled={
                      !draftName.trim() || !draftText.trim() || isPromptBusy
                    }
                  >
                    {t("settings.postProcessing.prompts.createPrompt")}
                  </Button>
                  <Button
                    onClick={handleCancelCreate}
                    variant="secondary"
                    size="md"
                    disabled={isPromptBusy}
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
                      !draftName.trim() ||
                      !draftText.trim() ||
                      !isDirty ||
                      isPromptBusy
                    }
                  >
                    {t("settings.postProcessing.prompts.updatePrompt")}
                  </Button>
                  {SHIPPED_PROMPT_IDS.has(selectedPromptId) && (
                    <Button
                      onClick={handleRestorePrompt}
                      variant="secondary"
                      size="md"
                      disabled={isPromptBusy}
                    >
                      <RotateCcw size={14} />
                      {t("settings.postProcessing.prompts.restorePrompt")}
                    </Button>
                  )}
                  <Button
                    onClick={() => handleDeletePrompt(selectedPromptId)}
                    variant="secondary"
                    size="md"
                    disabled={
                      // A shipped prompt is a reference point, not user content:
                      // deleting it just makes the recommended pairing
                      // unreachable, and the backend would re-seed it anyway.
                      !selectedPromptId ||
                      prompts.length <= 1 ||
                      isPromptBusy ||
                      SHIPPED_PROMPT_IDS.has(selectedPromptId)
                    }
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

const BUILTIN_TONE_IDS = [
  "none",
  "formal",
  "casual",
  "professional",
  "friendly",
  "concise",
] as const;

const PostProcessingToneComponent: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, refreshSettings } = useSettings();
  const customTones: CustomPostProcessTone[] = (
    getSetting("post_process_custom_tones") ?? []
  ).filter(
    (tone) =>
      tone.id === tone.id.trim() &&
      tone.id.length > 0 &&
      !BUILTIN_TONE_IDS.some((builtinId) => builtinId === tone.id) &&
      tone.name.trim() &&
      tone.instruction.trim(),
  );
  const selectedToneId =
    getSetting("post_process_selected_tone_id") ??
    getSetting("post_process_tone") ??
    "none";
  const selectedCustomTone =
    customTones.find((tone) => tone.id === selectedToneId) ?? null;

  const [editorMode, setEditorMode] = useState<"create" | "edit" | null>(null);
  const [draftName, setDraftName] = useState("");
  const [draftInstruction, setDraftInstruction] = useState("");
  const [isToneBusy, setIsToneBusy] = useState(false);
  const toneNameInputId = React.useId();
  const toneInstructionInputId = React.useId();

  useEffect(() => {
    if (editorMode !== null) return;
    setDraftName(selectedCustomTone?.name ?? "");
    setDraftInstruction(selectedCustomTone?.instruction ?? "");
  }, [editorMode, selectedCustomTone]);

  const options = [
    ...BUILTIN_TONE_IDS.map((value) => ({
      value,
      label: t(`settings.postProcessing.tone.options.${value}`),
    })),
    ...customTones.map((tone) => ({
      value: tone.id,
      label: tone.name,
    })),
  ];

  const showError = (key: string, defaultValue: string) => {
    toast.error(t(key, { defaultValue }));
  };

  const handleToneSelect = async (toneId: string | null) => {
    if (!toneId || isToneBusy || toneId === selectedToneId) return;
    setIsToneBusy(true);
    try {
      const result = await commands.changePostProcessToneSetting(toneId);
      if (result.status !== "ok") {
        showError(
          "settings.postProcessing.errors.toneSelectFailed",
          "Couldn’t select the writing style.",
        );
        return;
      }
      await refreshSettings();
      setEditorMode(null);
    } catch (error) {
      console.error("Failed to select writing style:", error);
      showError(
        "settings.postProcessing.errors.toneSelectFailed",
        "Couldn’t select the writing style.",
      );
    } finally {
      setIsToneBusy(false);
    }
  };

  const handleCreateTone = async () => {
    if (!draftName.trim() || !draftInstruction.trim() || isToneBusy) return;
    setIsToneBusy(true);
    try {
      const created = await commands.addPostProcessCustomTone(
        draftName.trim(),
        draftInstruction.trim(),
      );
      if (created.status !== "ok") {
        showError(
          "settings.postProcessing.errors.toneCreateFailed",
          "Couldn’t create the writing style.",
        );
        return;
      }
      const selected = await commands.changePostProcessToneSetting(
        created.data.id,
      );
      if (selected.status !== "ok") {
        showError(
          "settings.postProcessing.errors.toneSelectAfterCreateFailed",
          "Created the style, but couldn’t select it.",
        );
      }
      await refreshSettings();
      setEditorMode(null);
    } catch (error) {
      console.error("Failed to create writing style:", error);
      showError(
        "settings.postProcessing.errors.toneCreateFailed",
        "Couldn’t create the writing style.",
      );
    } finally {
      setIsToneBusy(false);
    }
  };

  const handleUpdateTone = async () => {
    if (
      !selectedCustomTone ||
      !draftName.trim() ||
      !draftInstruction.trim() ||
      isToneBusy
    )
      return;
    setIsToneBusy(true);
    try {
      const result = await commands.updatePostProcessCustomTone(
        selectedCustomTone.id,
        draftName.trim(),
        draftInstruction.trim(),
      );
      if (result.status !== "ok") {
        showError(
          "settings.postProcessing.errors.toneUpdateFailed",
          "Couldn’t update the writing style.",
        );
        return;
      }
      await refreshSettings();
      setEditorMode(null);
    } catch (error) {
      console.error("Failed to update writing style:", error);
      showError(
        "settings.postProcessing.errors.toneUpdateFailed",
        "Couldn’t update the writing style.",
      );
    } finally {
      setIsToneBusy(false);
    }
  };

  const handleDeleteTone = async () => {
    if (!selectedCustomTone || isToneBusy) return;
    setIsToneBusy(true);
    try {
      const result = await commands.deletePostProcessCustomTone(
        selectedCustomTone.id,
      );
      if (result.status !== "ok") {
        showError(
          "settings.postProcessing.errors.toneDeleteFailed",
          "Couldn’t delete the writing style.",
        );
        return;
      }
      await refreshSettings();
      setEditorMode(null);
    } catch (error) {
      console.error("Failed to delete writing style:", error);
      showError(
        "settings.postProcessing.errors.toneDeleteFailed",
        "Couldn’t delete the writing style.",
      );
    } finally {
      setIsToneBusy(false);
    }
  };

  const startCreate = () => {
    setDraftName("");
    setDraftInstruction("");
    setEditorMode("create");
  };

  const startEdit = () => {
    if (!selectedCustomTone) return;
    setDraftName(selectedCustomTone.name);
    setDraftInstruction(selectedCustomTone.instruction);
    setEditorMode("edit");
  };

  const cancelEditor = () => {
    setEditorMode(null);
    setDraftName(selectedCustomTone?.name ?? "");
    setDraftInstruction(selectedCustomTone?.instruction ?? "");
  };

  const isDirty =
    editorMode === "create" ||
    (!!selectedCustomTone &&
      (draftName.trim() !== selectedCustomTone.name ||
        draftInstruction.trim() !== selectedCustomTone.instruction));
  const canSave =
    !!draftName.trim() && !!draftInstruction.trim() && isDirty && !isToneBusy;
  const fieldLabelClasses =
    "block text-[11px] font-medium uppercase tracking-wide text-muted";

  return (
    <SettingContainer
      title={t("settings.postProcessing.tone.title")}
      description={t("settings.postProcessing.tone.description")}
      // Tooltip, not inline: the group header already states that the style is
      // layer 2 and shapes wording rather than corrections, so printing it again
      // here was the single biggest source of wall-of-text in this section.
      descriptionMode="tooltip"
      layout="stacked"
      grouped={true}
    >
      <div className="space-y-4">
        <div className="flex gap-2">
          <Dropdown
            selectedValue={selectedToneId}
            options={options}
            onSelect={handleToneSelect}
            disabled={isToneBusy || editorMode === "create"}
            className="flex-1"
          />
          {selectedCustomTone && editorMode !== "create" && (
            <Button
              onClick={() =>
                editorMode === "edit" ? setEditorMode(null) : startEdit()
              }
              variant="secondary"
              size="md"
              disabled={isToneBusy}
            >
              <Pencil size={14} />
              {editorMode === "edit"
                ? t("settings.postProcessing.tone.closeEditor")
                : t("settings.postProcessing.tone.edit")}
            </Button>
          )}
          <Button
            onClick={startCreate}
            variant="secondary"
            size="md"
            disabled={editorMode === "create" || isToneBusy}
          >
            {t("settings.postProcessing.tone.createNew")}
          </Button>
        </div>

        {editorMode && (
          <div className="space-y-4">
            <div className="space-y-1.5">
              <label htmlFor={toneNameInputId} className={fieldLabelClasses}>
                {t("settings.postProcessing.tone.nameLabel")}
              </label>
              <Input
                id={toneNameInputId}
                value={draftName}
                onChange={(event) => setDraftName(event.target.value)}
                placeholder={t("settings.postProcessing.tone.namePlaceholder")}
                variant="compact"
                className="w-full"
              />
            </div>
            <div className="space-y-1.5">
              <label
                htmlFor={toneInstructionInputId}
                className={fieldLabelClasses}
              >
                {t("settings.postProcessing.tone.instructionsLabel")}
              </label>
              <Textarea
                id={toneInstructionInputId}
                value={draftInstruction}
                onChange={(event) => setDraftInstruction(event.target.value)}
                rows={5}
                placeholder={t(
                  "settings.postProcessing.tone.instructionsPlaceholder",
                )}
                className="w-full"
              />
              <p className="text-[12px] leading-relaxed text-muted">
                {t("settings.postProcessing.tone.instructionsHint")}
              </p>
            </div>
            <div className="flex items-center gap-2">
              <Button
                onClick={
                  editorMode === "create" ? handleCreateTone : handleUpdateTone
                }
                variant="primary"
                size="md"
                disabled={!canSave}
              >
                {editorMode === "create"
                  ? t("settings.postProcessing.tone.createTone")
                  : t("settings.postProcessing.tone.updateTone")}
              </Button>
              <Button
                onClick={cancelEditor}
                variant="secondary"
                size="md"
                disabled={isToneBusy}
              >
                {t("settings.postProcessing.tone.cancel")}
              </Button>
              {editorMode === "edit" && (
                <Button
                  onClick={handleDeleteTone}
                  variant="secondary"
                  size="md"
                  disabled={isToneBusy}
                >
                  {t("settings.postProcessing.tone.deleteTone")}
                </Button>
              )}
            </div>
          </div>
        )}
      </div>
    </SettingContainer>
  );
};

export const PostProcessingTone = React.memo(PostProcessingToneComponent);
PostProcessingTone.displayName = "PostProcessingTone";
