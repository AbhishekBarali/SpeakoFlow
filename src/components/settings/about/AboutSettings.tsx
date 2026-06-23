import React, { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Button } from "../../ui/Button";
import { AppDataDirectory } from "../AppDataDirectory";
import { AppLanguageSelector } from "../AppLanguageSelector";
import { LogDirectory } from "../debug";

/** Projects SpeakoFlow is built on. Shown one-at-a-time in a pager so the
 * About page stays uncluttered as the list grows. */
const ACKNOWLEDGMENTS = ["handy", "whisper", "llamacpp", "kokoro"] as const;

export const AboutSettings: React.FC = () => {
  const { t } = useTranslation();
  const [version, setVersion] = useState("");
  const [ackIndex, setAckIndex] = useState(0);
  const currentAck = ACKNOWLEDGMENTS[ackIndex];

  useEffect(() => {
    const fetchVersion = async () => {
      try {
        const appVersion = await getVersion();
        setVersion(appVersion);
      } catch (error) {
        console.error("Failed to get app version:", error);
        setVersion("0.1.2");
      }
    };

    fetchVersion();
  }, []);

  return (
    <div className="max-w-3xl w-full mx-auto space-y-8">
      <SettingsGroup title={t("settings.about.title")}>
        <AppLanguageSelector descriptionMode="tooltip" grouped={true} />
        <SettingContainer
          title={t("settings.about.version.title")}
          description={t("settings.about.version.description")}
          grouped={true}
        >
          {/* eslint-disable-next-line i18next/no-literal-string */}
          <span className="text-sm font-mono">v{version}</span>
        </SettingContainer>

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

        <AppDataDirectory descriptionMode="tooltip" grouped={true} />
        <LogDirectory grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settings.about.acknowledgments.title")}>
        <div className="px-4 py-4">
          <h3 className="text-sm font-medium text-ink">
            {t(`settings.about.acknowledgments.${currentAck}.title`)}
          </h3>
          <p className="mt-0.5 text-xs text-muted">
            {t(`settings.about.acknowledgments.${currentAck}.description`)}
          </p>
          <p className="mt-2 text-sm text-body leading-relaxed">
            {t(`settings.about.acknowledgments.${currentAck}.details`)}
          </p>

          {ACKNOWLEDGMENTS.length > 1 && (
            <div className="mt-4 flex items-center justify-between">
              <div className="flex items-center gap-1.5">
                {ACKNOWLEDGMENTS.map((ack, i) => (
                  <span
                    key={ack}
                    className={`h-1.5 rounded-full transition-all ${
                      i === ackIndex ? "w-5 bg-ink" : "w-1.5 bg-mid-gray/40"
                    }`}
                  />
                ))}
              </div>
              <div className="flex items-center gap-1">
                <button
                  type="button"
                  onClick={() =>
                    setAckIndex(
                      (i) =>
                        (i - 1 + ACKNOWLEDGMENTS.length) %
                        ACKNOWLEDGMENTS.length,
                    )
                  }
                  className="p-1.5 rounded-lg text-muted hover:text-ink hover:bg-surface-strong transition-colors cursor-pointer"
                  title={t("settings.about.acknowledgments.previous")}
                >
                  <ChevronLeft width={16} height={16} />
                </button>
                <button
                  type="button"
                  onClick={() =>
                    setAckIndex((i) => (i + 1) % ACKNOWLEDGMENTS.length)
                  }
                  className="p-1.5 rounded-lg text-muted hover:text-ink hover:bg-surface-strong transition-colors cursor-pointer"
                  title={t("settings.about.acknowledgments.next")}
                >
                  <ChevronRight width={16} height={16} />
                </button>
              </div>
            </div>
          )}
        </div>
      </SettingsGroup>
    </div>
  );
};
