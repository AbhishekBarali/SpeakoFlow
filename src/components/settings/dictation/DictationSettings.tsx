import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { SubPage } from "@/components/ui/SubPage";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SectionHeader } from "@/components/ui/SectionHeader";
import { ModelsSettings } from "../models/ModelsSettings";
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
 * Dictation — everything about turning voice into text, visible at a glance:
 *   1. Hero: the active speech-to-text model (+ its language options when the
 *      model has any). "Change model" opens the transcription-only catalog.
 *   2. AI cleanup: the optional post-dictation cleanup pass, laid out in
 *      clean groups (no folds).
 *   3. Output: how the transcribed text is typed/pasted and refined.
 *
 * No accordions — the page scrolls, and only the model catalog lives one
 * level deeper.
 */
export const DictationSettings: React.FC = () => {
  const { t } = useTranslation();
  const [showCatalog, setShowCatalog] = useState(false);

  // The catalog opens locked to transcription models — picking a dictation
  // model should never show assistant or speech models.
  if (showCatalog) {
    return (
      <SubPage
        title={t("settings.dictation.catalog.title")}
        description={t("settings.dictation.catalog.description")}
        onBack={() => setShowCatalog(false)}
      >
        <ModelsSettings lockedCategory="stt" />
      </SubPage>
    );
  }

  return (
    <div className="w-full max-w-3xl mx-auto space-y-6">
      <SectionHeader
        title={t("sidebar.dictation")}
        description={t("sectionSubtitles.dictation")}
      />
      <DictationModelCard onChangeModel={() => setShowCatalog(true)} />

      {/* Language / translate rows — only for models that support them. */}
      <ModelSettingsCard />

      <AiCleanupGroup />

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
