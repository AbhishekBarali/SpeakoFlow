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

/**
 * Meetings. The banner is for starting one; how it is set up is an ordinary
 * settings group under it, laid out like the Assistant page's: which model
 * writes the transcript and which writes the notes (each a picker, so
 * changing one is a click rather than a trip to Models), then the two
 * switches.
 */
export const MeetingsPage: React.FC = () => {
  const { t } = useTranslation();
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
            <SettingContainer
              title={t("meetingsPage.notesBy")}
              description={t("meetingsPage.notesByTip")}
              grouped={true}
            >
              <LlmModelPicker role="assistant" />
            </SettingContainer>
            <CallDetectionToggle />
            <IndicatorToggle />
          </SettingsGroup>
        }
      />
    </Page>
  );
};
