import React, { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { getVersion } from "@tauri-apps/api/app";
import { emit } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { SectionHeader } from "../../ui/SectionHeader";
import { Button } from "../../ui/Button";
import { AppDataDirectory } from "../AppDataDirectory";
import { AppLanguageSelector } from "../AppLanguageSelector";
import { LogDirectory } from "../debug";
import { useSettings } from "../../../hooks/useSettings";

export const AboutSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const [version, setVersion] = useState("");

  // The auto-check preference lives in General; here we only offer a manual
  // "check now" that reuses the footer updater (same path as the tray item).
  const updateChecksEnabled =
    (getSetting("update_checks_enabled") as boolean | undefined) ?? true;

  useEffect(() => {
    const fetchVersion = async () => {
      try {
        const appVersion = await getVersion();
        setVersion(appVersion);
      } catch (error) {
        console.error("Failed to get app version:", error);
        setVersion("");
      }
    };

    fetchVersion();
  }, []);

  return (
    <div className="max-w-3xl w-full mx-auto space-y-8">
      <SectionHeader
        title={t("sidebar.about")}
        description={t("sectionSubtitles.about")}
      />
      {/* ── Default view ─────────────────────────────────────────────────── */}
      <SettingsGroup>
        <SettingContainer
          title={t("settings.about.version.title")}
          grouped={true}
        >
          {/* eslint-disable-next-line i18next/no-literal-string */}
          {version && <span className="text-sm font-mono">v{version}</span>}
        </SettingContainer>

        <SettingContainer
          title={t("settings.about.updates.title")}
          description={
            updateChecksEnabled
              ? undefined
              : t("settings.about.updates.disabledHint")
          }
          grouped={true}
        >
          <Button
            variant="secondary"
            size="md"
            disabled={!updateChecksEnabled}
            onClick={() => void emit("check-for-updates")}
          >
            {t("settings.about.updates.button")}
          </Button>
        </SettingContainer>

        <AppLanguageSelector descriptionMode="tooltip" grouped={true} />

        <SettingContainer
          title={t("settings.about.sourceCode.title")}
          description={t("settings.about.sourceCode.description")}
          grouped={true}
        >
          <Button
            variant="secondary"
            size="md"
            onClick={() =>
              openUrl("https://github.com/AbhishekBarali/SpeakoFlow")
            }
          >
            {t("settings.about.sourceCode.button")}
          </Button>
        </SettingContainer>

        <SettingContainer
          title={t("settings.about.license.title")}
          description={t("settings.about.license.description")}
          grouped={true}
        >
          <Button
            variant="secondary"
            size="md"
            onClick={() =>
              openUrl(
                "https://github.com/AbhishekBarali/SpeakoFlow/blob/main/LICENSE",
              )
            }
          >
            {t("settings.about.license.button")}
          </Button>
        </SettingContainer>
      </SettingsGroup>

      <SettingsGroup title={t("settings.about.folders.title")}>
        <AppDataDirectory descriptionMode="tooltip" grouped={true} />
        <LogDirectory grouped={true} />
      </SettingsGroup>
    </div>
  );
};
