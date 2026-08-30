import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { SubPage } from "@/components/ui/SubPage";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { ModelsSettings } from "../models/ModelsSettings";
import { LlmCatalog } from "../assistant/LlmCatalog";
import { DictationModelCard } from "./DictationModelCard";
import { AiCleanupGroup } from "./AiCleanupGroup";
import { GenerateWithFlowGroup } from "./GenerateWithFlowGroup";
import { SpokenEmojiToggle } from "./SpokenEmojiToggle";
import { ModelSettingsCard } from "../general/ModelSettingsCard";
// Dictation-output rows: how transcribed text lands in the active app.
import { PasteMethodSetting } from "../PasteMethod";
import { TypingToolSetting } from "../TypingTool";
import { ClipboardHandlingSetting } from "../ClipboardHandling";
import { AppendTrailingSpace } from "../AppendTrailingSpace";
import { AutoSubmit } from "../AutoSubmit";
import { AlwaysOnMicrophone } from "../AlwaysOnMicrophone";
import { CustomWords } from "../CustomWords";
import { TextReplacements } from "../TextReplacements";

/**
 * Which one-level-deeper page is open, if any. Both model catalogs are reachable
 * from here on purpose: dictation has two models (the one that hears you and the
 * one that cleans up after it), and sending the user to the Assistant page for
 * the second one meant configuring dictation started somewhere else entirely.
 */
type DictationSubPage = "transcription" | "cleanup";

/**
 * Dictation — everything about turning voice into text, visible at a glance:
 *   1. Hero: the active speech-to-text model (+ its language options when the
 *      model has any). "Change model" opens the transcription-only catalog.
 *   2. AI cleanup: the optional post-dictation cleanup pass — on/off, which
 *      model runs it, and the two prompt layers it follows.
 *   3. Output: how the transcribed text is typed/pasted and refined.
 *
 * No accordions — the page scrolls, and only the two model catalogs live one
 * level deeper.
 */
export const DictationSettings: React.FC = () => {
  const { t } = useTranslation();
  const [subPage, setSubPage] = useState<DictationSubPage | null>(null);

  // Each catalog opens locked to what it is for: picking a dictation model should
  // never show assistant models, and picking a cleanup model should never show
  // speech models.
  if (subPage === "transcription") {
    return (
      <SubPage
        title={t("settings.dictation.catalog.title")}
        description={t("settings.dictation.catalog.description")}
        onBack={() => setSubPage(null)}
      >
        <ModelsSettings lockedCategory="stt" />
      </SubPage>
    );
  }

  if (subPage === "cleanup") {
    return (
      <SubPage
        title={t("settings.dictation.aiCleanup.catalog.title")}
        description={t("settings.dictation.aiCleanup.catalog.description")}
        onBack={() => setSubPage(null)}
      >
        <LlmCatalog role="cleanup" />
      </SubPage>
    );
  }

  return (
    <div className="w-full max-w-3xl mx-auto space-y-6">
      <SectionHeader
        title={t("sidebar.dictation")}
        description={t("sectionSubtitles.dictation")}
      />
      <DictationModelCard onChangeModel={() => setSubPage("transcription")} />

      {/* Language / translate rows — only for models that support them. */}
      <ModelSettingsCard />

      <AiCleanupGroup onBrowseCleanupModels={() => setSubPage("cleanup")} />

      <GenerateWithFlowGroup />

      <SettingsGroup title={t("settings.dictation.output.title")}>
        <SpokenEmojiToggle grouped={true} />
        <PasteMethodSetting grouped={true} />
        <TypingToolSetting grouped={true} />
        <ClipboardHandlingSetting grouped={true} />
        <AppendTrailingSpace grouped={true} />
        <AutoSubmit grouped={true} />
        <AlwaysOnMicrophone grouped={true} />
        <CustomWords grouped={true} />
        <TextReplacements grouped={true} />
      </SettingsGroup>
    </div>
  );
};
