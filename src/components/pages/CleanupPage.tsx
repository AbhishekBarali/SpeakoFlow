import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pencil } from "lucide-react";
import { useSettings } from "@/hooks/useSettings";
import { Page, PageHeader, SectionTitle } from "@/components/ui/Page";
import { Switch } from "@/components/ui/Switch";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Callout } from "@/components/ui/Callout";
import { Dialog } from "@/components/ui/Dialog";
import { Segmented } from "@/components/ui/Segmented";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { ShortcutInput } from "@/components/settings/ShortcutInput";
import { PostProcessingSettingsPrompts } from "@/components/settings/post-processing/PostProcessingSettings";
import { useNavigation } from "@/components/shell/navigation";
import { useModelSlots } from "@/components/shell/useModelSlots";
import { LlmModelPicker } from "@/components/shell/ModelPicker";
import { WritingStyleCard } from "./cleanup/WritingStyle";
import { useCleanupSpecialist } from "./cleanup/useCleanupSpecialist";

/** The base cleanup prompt: long-form text, so it is edited in a window. */
const InstructionsRow: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const [open, setOpen] = useState(false);

  const prompts = getSetting("post_process_prompts") ?? [];
  const selectedId = getSetting("post_process_selected_prompt_id") ?? "";
  const selectedName =
    selectedId === "none"
      ? t("settings.postProcessing.prompts.nonePrompt")
      : (prompts.find((prompt) => prompt.id === selectedId)?.name ??
        t("settings.postProcessing.prompts.selectPrompt"));

  // A cleanup fine-tune was trained on one exact prompt; say so where the
  // prompt is edited, so nobody "improves" it into worse output.
  const specialist = useCleanupSpecialist() !== null;

  return (
    <SettingContainer
      title={t("cleanup.instructions.title")}
      description={t("cleanup.instructions.description")}
      grouped={true}
    >
      <div className="flex items-center gap-3">
        <span className="max-w-[14rem] truncate text-sm text-muted">
          {selectedName}
        </span>
        <Button variant="secondary" size="sm" onClick={() => setOpen(true)}>
          <Pencil className="h-3.5 w-3.5" aria-hidden="true" />
          {t("common.edit")}
        </Button>
      </div>
      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        size="lg"
        title={t("cleanup.instructions.title")}
        description={t("cleanup.instructions.description")}
      >
        <div className="space-y-4">
          {specialist && (
            <Callout tone="info">
              {t("settings.dictation.aiCleanup.tunedModelNotice")}
            </Callout>
          )}
          <div className="overflow-visible rounded-xl border border-hairline bg-surface">
            <PostProcessingSettingsPrompts />
          </div>
        </div>
      </Dialog>
    </SettingContainer>
  );
};

/**
 * Which shortcut runs cleanup: its own, or the dictation shortcut. The second
 * means one set of keys for everything, with every dictation cleaned up; the
 * backend then releases the separate combo so it stops swallowing those keys
 * from other apps.
 */
const ShortcutModeRow: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const onDictation = getSetting("post_process_on_dictation") ?? false;
  return (
    <SettingContainer
      title={t("cleanup.shortcut.title")}
      description={t("cleanup.shortcut.info")}
      grouped={true}
    >
      <Segmented
        label={t("cleanup.shortcut.title")}
        value={onDictation ? "dictation" : "separate"}
        onChange={(mode) =>
          void updateSetting("post_process_on_dictation", mode === "dictation")
        }
        disabled={isUpdating("post_process_on_dictation")}
        options={[
          { value: "separate", label: t("cleanup.shortcut.separate") },
          { value: "dictation", label: t("cleanup.shortcut.dictation") },
        ]}
      />
    </SettingContainer>
  );
};

/**
 * The keys that actually clean up: the dictation shortcut once cleanup rides
 * on it, the separate one otherwise. Clicking them changes them, as on Home.
 */
const CleanupKeysRow: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const onDictation = getSetting("post_process_on_dictation") ?? false;
  const shortcutId = onDictation
    ? "transcribe"
    : "transcribe_with_post_process";
  return (
    <SettingContainer
      title={t("home.shortcuts.cleanup.title")}
      description={t("home.shortcuts.cleanup.what")}
      grouped={true}
    >
      <ShortcutInput
        // Keyed so switching modes mounts a fresh editor instead of
        // re-pointing one mid-edit at a different binding.
        key={shortcutId}
        shortcutId={shortcutId}
        bare
        size="md"
      />
    </SettingContainer>
  );
};

/** Must match `MIN_POST_PROCESS_WORDS` / `MAX_POST_PROCESS_WORDS` in
 *  `src-tauri/src/settings.rs`. */
const MIN_WORDS = 2;
const MAX_WORDS = 50;
const DEFAULT_WORDS = 15;

/**
 * Skip cleanup for short dictations. A search or a quick reply rarely has
 * anything to clean, and waiting on the model is the only delay left in it.
 * Only offered while cleanup rides on the dictation shortcut: the separate
 * cleanup shortcut is a request for cleanup, so it always cleans.
 */
const SkipShortRows: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const onDictation = getSetting("post_process_on_dictation") ?? false;
  const skip = getSetting("post_process_skip_short") ?? true;
  const stored = getSetting("post_process_min_words") ?? DEFAULT_WORDS;
  const [draft, setDraft] = useState(String(stored));

  useEffect(() => setDraft(String(stored)), [stored]);

  if (!onDictation) return null;

  const commit = () => {
    const parsed = Number.parseInt(draft, 10);
    if (!Number.isFinite(parsed)) {
      setDraft(String(stored));
      return;
    }
    const next = Math.min(MAX_WORDS, Math.max(MIN_WORDS, parsed));
    setDraft(String(next));
    if (next !== stored) void updateSetting("post_process_min_words", next);
  };

  // The unit follows the number being typed, so a language that inflects it
  // ("2 слова", "5 слов") reads right before the value is saved.
  const typed = Number.parseInt(draft, 10);
  const count = Number.isFinite(typed) ? typed : stored;

  return (
    <>
      <SettingContainer
        title={t("cleanup.skipShort.title")}
        description={t("cleanup.skipShort.info")}
        grouped={true}
      >
        <Switch
          checked={skip}
          onChange={(value) =>
            void updateSetting("post_process_skip_short", value)
          }
          disabled={isUpdating("post_process_skip_short")}
          label={t("cleanup.skipShort.title")}
        />
      </SettingContainer>
      {skip && (
        <SettingContainer
          title={t("cleanup.skipShort.limit")}
          description={t("cleanup.skipShort.limitInfo")}
          grouped={true}
        >
          <div className="flex items-center gap-2">
            <Input
              type="number"
              min={String(MIN_WORDS)}
              max={String(MAX_WORDS)}
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              onBlur={commit}
              onKeyDown={(event) => {
                // Enter saves, so a typed number needs no click elsewhere.
                if (event.key === "Enter") {
                  event.preventDefault();
                  event.currentTarget.blur();
                }
              }}
              disabled={isUpdating("post_process_min_words")}
              className="w-20"
              aria-label={t("cleanup.skipShort.limit")}
            />
            <span className="text-sm text-ink">
              {t("cleanup.skipShort.words", { count })}
            </span>
          </div>
        </SettingContainer>
      )}
    </>
  );
};

/**
 * AI cleanup: a switch, the shortcut that uses it (click the keys to change
 * them), the model, and the writing styles — all of them visible, with the one
 * you pick shown on a real sentence. Only the long base prompt opens a window.
 *
 * No banner: the one it had repeated the shortcut the rows below already show
 * and drew a paragraph of word-bars that read as a loading skeleton. The
 * writing-style card's "You say / It types" example explains the feature
 * with a real sentence instead.
 */
export const CleanupPage: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const { openModelSlot } = useNavigation();
  const summary = useModelSlots().cleanup;
  const enabled = getSetting("post_process_enabled") ?? false;

  return (
    <Page>
      <PageHeader
        title={t("nav.cleanup")}
        description={t("cleanup.description")}
        actions={
          <div className="inline-flex items-center gap-2.5 text-sm font-medium text-ink">
            <span aria-hidden="true">
              {enabled ? t("common.on") : t("common.off")}
            </span>
            <Switch
              checked={enabled}
              onChange={(value) =>
                void updateSetting("post_process_enabled", value)
              }
              disabled={isUpdating("post_process_enabled")}
              label={t("cleanup.toggle")}
            />
          </div>
        }
      />

      {enabled && !summary.ready && (
        <div className="mb-6 flex flex-wrap items-center gap-x-4 gap-y-3 rounded-2xl border border-warning/25 bg-warning/[0.06] px-5 py-3.5">
          <span className="h-2 w-2 shrink-0 rounded-full bg-warning" />
          <p className="min-w-0 flex-1 text-sm font-medium text-ink">
            {t("cleanup.needsModel.short")}
          </p>
          <Button size="sm" onClick={() => openModelSlot("cleanup")}>
            {t("cleanup.needsModel.action")}
          </Button>
        </div>
      )}

      <SettingsGroup>
        <ShortcutModeRow />
        <CleanupKeysRow />
        <SkipShortRows />
        <SettingContainer
          title={t("cleanup.model.title")}
          description={t("cleanup.model.info")}
          grouped={true}
        >
          <LlmModelPicker role="cleanup" />
        </SettingContainer>
        <InstructionsRow />
      </SettingsGroup>

      <section className="mt-10">
        <SectionTitle
          title={t("cleanup.styles.title")}
          description={t("cleanup.styles.description")}
        />
        <div className="rounded-2xl border border-hairline bg-surface p-5 elev-card">
          <WritingStyleCard />
        </div>
      </section>
    </Page>
  );
};
