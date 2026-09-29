import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Pencil } from "lucide-react";
import { useSettings } from "@/hooks/useSettings";
import { useModelStore } from "@/stores/modelStore";
import { isCleanupSpecialistModel } from "@/lib/utils/cleanupSpecialist";
import { Page, PageHeader, SectionTitle } from "@/components/ui/Page";
import { Switch } from "@/components/ui/Switch";
import { Button } from "@/components/ui/Button";
import { Callout } from "@/components/ui/Callout";
import { Dialog } from "@/components/ui/Dialog";
import { Hero, HeroShortcut } from "@/components/ui/Hero";
import { Segmented } from "@/components/ui/Segmented";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { ShortcutInput } from "@/components/settings/ShortcutInput";
import { PostProcessingSettingsPrompts } from "@/components/settings/post-processing/PostProcessingSettings";
import { useNavigation } from "@/components/shell/navigation";
import { useModelSlots } from "@/components/shell/useModelSlots";
import { LlmModelPicker } from "@/components/shell/ModelPicker";
import { WritingStyleCard } from "./cleanup/WritingStyle";

/** "um so I think we should uh meet tomorrow" → the cleaned sentence. The
 *  feature explained by showing it, instead of a paragraph about it. */
const BeforeAfter: React.FC = () => {
  const { t } = useTranslation();
  return (
    <div className="glass-chip max-w-md rounded-2xl px-4 py-3.5">
      <p className="text-[0.9375rem] text-white/60 line-through decoration-white/35">
        {t("cleanup.hero.before")}
      </p>
      <p className="mt-1.5 flex items-start gap-2 text-[0.9375rem] font-medium text-white">
        <ArrowRight
          className="mt-[0.3rem] h-3.5 w-3.5 shrink-0 text-white/70 rtl:rotate-180"
          aria-hidden="true"
        />
        {t("cleanup.hero.after")}
      </p>
    </div>
  );
};

/** The base cleanup prompt: long-form text, so it is edited in a window. */
const InstructionsRow: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, postProcessReadiness, settings } = useSettings();
  const models = useModelStore((state) => state.models);
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
  const resolvedModel =
    postProcessReadiness?.state === "ready"
      ? postProcessReadiness.model
      : (settings?.post_process_models?.[
          settings?.post_process_provider_id ?? ""
        ] ?? "");
  const specialist =
    isCleanupSpecialistModel(resolvedModel) ||
    models.some(
      (model) => model.id === resolvedModel && model.is_cleanup_specialist,
    );

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
        size="sm"
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
 * AI cleanup: a switch, the shortcut that uses it (click the keys to change
 * them), the model, and the writing styles — all of them visible, with the one
 * you pick shown on a real sentence. Only the long base prompt opens a window.
 */
export const CleanupPage: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const { openModelSlot } = useNavigation();
  const summary = useModelSlots().cleanup;
  const enabled = getSetting("post_process_enabled") ?? false;
  const holdToTalk = getSetting("push_to_talk") ?? true;
  // The hero shows the keys that actually clean up: the dictation shortcut
  // once cleanup rides on it, the separate one otherwise.
  const onDictation = getSetting("post_process_on_dictation") ?? false;

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

      <Hero
        title={t("cleanup.hero.title")}
        aside={
          <HeroShortcut
            label={
              onDictation
                ? t("home.shortcuts.dictate.title")
                : t("home.shortcuts.cleanup.title")
            }
            hint={
              !enabled
                ? t("cleanup.hero.offHint")
                : onDictation
                  ? t("cleanup.hero.onDictationHint")
                  : holdToTalk
                    ? t("cleanup.hero.onHint")
                    : t("cleanup.hero.onHintTap")
            }
          >
            <span className={enabled ? undefined : "opacity-60"}>
              <ShortcutInput
                // Keyed so switching modes mounts a fresh editor instead of
                // re-pointing one mid-edit at a different binding.
                key={
                  onDictation ? "transcribe" : "transcribe_with_post_process"
                }
                shortcutId={
                  onDictation ? "transcribe" : "transcribe_with_post_process"
                }
                bare
                finish="glass"
                size="lg"
                showReset="never"
              />
            </span>
          </HeroShortcut>
        }
      >
        <BeforeAfter />
      </Hero>

      {enabled && !summary.ready && (
        <div className="mt-5 flex flex-wrap items-center gap-x-4 gap-y-3 rounded-2xl border border-warning/25 bg-warning/[0.06] px-5 py-3.5">
          <span className="h-2 w-2 shrink-0 rounded-full bg-warning" />
          <p className="min-w-0 flex-1 text-sm font-medium text-ink">
            {t("cleanup.needsModel.short")}
          </p>
          <Button size="sm" onClick={() => openModelSlot("cleanup")}>
            {t("cleanup.needsModel.action")}
          </Button>
        </div>
      )}

      <SettingsGroup className="mt-6">
        <ShortcutModeRow />
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
