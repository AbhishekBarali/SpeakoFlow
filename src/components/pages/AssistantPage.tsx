import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useSettingCommand } from "@/hooks/useSettingCommand";
import { Page, PageHeader } from "@/components/ui/Page";
import { Switch } from "@/components/ui/Switch";
import { Dialog } from "@/components/ui/Dialog";
import { Hero, HeroShortcut } from "@/components/ui/Hero";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { ShortcutInput } from "@/components/settings/ShortcutInput";
import {
  Avatar,
  CharactersSettings,
} from "@/components/settings/assistant/CharactersSettings";
import { MemorySettings } from "@/components/settings/assistant/MemorySettings";
import { LlmModelPicker, VoicePicker } from "@/components/shell/ModelPicker";
import {
  MemoryRow,
  ProfilesRow,
  RemindersRow,
  RepliesRows,
  VisionRows,
  WebSearchRows,
} from "./assistant/AssistantRows";
import { PanelCard } from "./assistant/PanelCard";

type Editor = "memory" | "profiles" | null;

/**
 * The assistant as a feature rather than a settings page: how to call it up
 * (the hero, where the keys are also the way to change them), what it thinks
 * and speaks with, then plain rows — a name, a short (i), and the control.
 * Extra detail appears under a row only while its feature is on. Two things
 * open a window, because they are long-form editing: memory and profiles.
 */
export const AssistantPage: React.FC = () => {
  const { t } = useTranslation();
  const { settings, getSetting } = useSettings();
  const run = useSettingCommand();
  const [editor, setEditor] = useState<Editor>(null);
  const [switching, setSwitching] = useState(false);
  const enabled = settings?.assistant_enabled ?? true;
  const ttsEnabled = settings?.assistant_tts_enabled ?? false;
  const holdToTalk = getSetting("push_to_talk") ?? true;

  const setEnabled = async (value: boolean) => {
    setSwitching(true);
    await run(commands.setAssistantEnabled(value));
    setSwitching(false);
  };

  return (
    <Page>
      <PageHeader
        title={t("nav.assistant")}
        description={t("assistantPage.description")}
        actions={
          <div className="inline-flex items-center gap-2.5 text-sm font-medium text-ink">
            <span aria-hidden="true">
              {enabled ? t("common.on") : t("common.off")}
            </span>
            <Switch
              checked={enabled}
              onChange={(value) => void setEnabled(value)}
              disabled={switching}
              label={t("settings.assistant.enable.label")}
            />
          </div>
        }
      />

      {!enabled ? (
        <p className="rounded-2xl border border-hairline bg-surface-muted px-5 py-4 text-sm text-body">
          {t("assistantPage.offNotice")}
        </p>
      ) : (
        <>
          <Hero
            title={t("assistantPage.hero.title")}
            subtitle={t("assistantPage.hero.subtitle")}
            aside={
              <div className="flex flex-wrap gap-x-8 gap-y-5">
                <HeroShortcut
                  label={t("assistantPage.hero.ask")}
                  hint={
                    holdToTalk
                      ? t("assistantPage.hero.askHoldShort")
                      : t("assistantPage.hero.askTapShort")
                  }
                >
                  <ShortcutInput
                    shortcutId="assistant"
                    bare
                    finish="glass"
                    size="lg"
                    showReset="never"
                  />
                </HeroShortcut>
                <HeroShortcut
                  label={t("assistantPage.hero.callLabel")}
                  hint={t("assistantPage.hero.callShort")}
                >
                  <ShortcutInput
                    shortcutId="assistant_call"
                    bare
                    finish="glass"
                    size="lg"
                    showReset="never"
                  />
                </HeroShortcut>
              </div>
            }
          />

          <SettingsGroup className="mt-6">
            <SettingContainer
              title={t("assistantPage.models.assistant")}
              description={t("assistantPage.tips.thinksWith")}
              grouped={true}
            >
              <LlmModelPicker role="assistant" />
            </SettingContainer>
            <SettingContainer
              title={t("assistantPage.models.voice")}
              description={t("assistantPage.tips.speaks")}
              grouped={true}
            >
              <div className="flex items-center gap-3">
                {ttsEnabled && <VoicePicker className="w-[15rem]" />}
                <Switch
                  checked={ttsEnabled}
                  onChange={(value) =>
                    void run(commands.setAssistantTtsEnabled(value))
                  }
                  label={t("settings.assistant.tts.enableLabel")}
                />
              </div>
            </SettingContainer>
          </SettingsGroup>

          <section className="mt-10 space-y-8">
            <SettingsGroup title={t("assistantPage.features.title")}>
              <VisionRows />
              <WebSearchRows />
              <RemindersRow />
            </SettingsGroup>
            <SettingsGroup title={t("assistantPage.groups.personal")}>
              <MemoryRow onManage={() => setEditor("memory")} />
              <ProfilesRow
                onEdit={() => setEditor("profiles")}
                avatar={(character, size) => (
                  <Avatar character={character} size={size} />
                )}
              />
            </SettingsGroup>
            <SettingsGroup title={t("assistantPage.groups.replies")}>
              <RepliesRows />
            </SettingsGroup>
            <section className="space-y-2.5">
              <h2 className="px-1 text-[0.8125rem] font-semibold text-muted">
                {t("assistantPage.cards.panel.title")}
              </h2>
              <PanelCard />
            </section>
          </section>
        </>
      )}

      <Dialog
        open={editor === "memory"}
        onClose={() => setEditor(null)}
        size="xl"
        title={t("assistantPage.features.memory.title")}
        description={t("assistantPage.features.memory.description")}
      >
        <MemorySettings />
      </Dialog>
      <Dialog
        open={editor === "profiles"}
        onClose={() => setEditor(null)}
        size="xl"
        title={t("assistantPage.features.profiles.title")}
        description={t("assistantPage.features.profiles.description")}
      >
        <CharactersSettings />
      </Dialog>
    </Page>
  );
};
