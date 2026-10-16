import React from "react";
import { useTranslation } from "react-i18next";
import { Page } from "@/components/ui/Page";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { MeetingsSection } from "@/components/settings/meetings/MeetingsSection";
import {
  CallDetectionToggle,
  IndicatorToggle,
} from "@/components/settings/meetings/CallDetectionToggle";
import { LlmModelPicker, SttModelPicker } from "@/components/shell/ModelPicker";
import {
  AutoNotesToggle,
  NotesLanguageSetting,
  useMeetingNotesPrefs,
} from "@/components/settings/meetings/NotesPrefs";

/**
 * Meetings. The banner is for starting one; how it is set up is an ordinary
 * settings group under it, laid out like the Assistant page's: which model
 * writes the transcript and which writes the notes (each a picker, so
 * changing one is a click rather than a trip to Models), whether notes are
 * written at all and in which language, then the two switches.
 */
export const MeetingsPage: React.FC = () => {
  const { t } = useTranslation();
  const notesPrefs = useMeetingNotesPrefs();
  return (
    <Page>
      <MeetingsSection
        settings={
          <SettingsGroup>
            <SettingContainer
              title={t("meetingsPage.transcribedBy")}
              description={t("meetingsPage.transcribedByTip")}
              grouped={true}
            >
              <SttModelPicker />
            </SettingContainer>
            <AutoNotesToggle model={notesPrefs} />
            <SettingContainer
              title={t("meetingsPage.notesBy")}
              description={t("meetingsPage.notesByTip")}
              grouped={true}
            >
              <LlmModelPicker role="assistant" />
            </SettingContainer>
            <NotesLanguageSetting model={notesPrefs} />
            <CallDetectionToggle />
            <IndicatorToggle />
          </SettingsGroup>
        }
      />
    </Page>
  );
};
