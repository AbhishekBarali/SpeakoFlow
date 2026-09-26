import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus } from "lucide-react";
import { Page, PageHeader } from "@/components/ui/Page";
import { Tabs } from "@/components/ui/Tabs";
import { Button } from "@/components/ui/Button";
import { Hero } from "@/components/ui/Hero";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { CustomWords } from "@/components/settings/CustomWords";
import { AutoLearnCorrections } from "@/components/settings/AutoLearnCorrections";
import { TextReplacements } from "@/components/settings/TextReplacements";
import { useDismissibleNotice } from "@/hooks/useDismissibleNotice";

type DictionaryTab = "words" | "replacements";

const WORD_INPUT_ID = "dictionary-new-word";

/**
 * Dictionary: the words SpeakoFlow should always get right, and the phrases
 * it should expand, as two tabs. The introduction is a closable hero rather
 * than a paragraph that stays on the page forever.
 */
export const DictionaryPage: React.FC = () => {
  const { t } = useTranslation();
  const [tab, setTab] = useState<DictionaryTab>("words");
  const intro = useDismissibleNotice("dictionary-hero");

  const addWord = () => {
    setTab("words");
    window.requestAnimationFrame(() =>
      document.getElementById(WORD_INPUT_ID)?.focus(),
    );
  };

  return (
    <Page>
      <PageHeader
        title={t("nav.dictionary")}
        description={t("dictionary.description")}
        actions={
          <Button onClick={addWord}>
            <Plus className="h-4 w-4" aria-hidden="true" />
            {t("dictionary.addWord")}
          </Button>
        }
      />

      <Tabs
        label={t("nav.dictionary")}
        value={tab}
        onChange={setTab}
        items={[
          { id: "words", label: t("dictionary.words.title") },
          { id: "replacements", label: t("dictionary.replacements.title") },
        ]}
      />

      <div className="mt-6">
        {tab === "words" ? (
          <div className="tab-reveal space-y-5">
            {intro.visible && (
              <Hero
                art
                title={t("dictionary.hero.title")}
                subtitle={t("dictionary.hero.subtitle")}
                onDismiss={intro.dismiss}
              />
            )}
            <CustomWords inputId={WORD_INPUT_ID} />
            <div className="rounded-2xl border border-hairline bg-surface elev-card">
              <AutoLearnCorrections grouped={true} />
            </div>
          </div>
        ) : (
          <div className="tab-reveal">
            <SettingsGroup>
              <TextReplacements grouped={true} />
            </SettingsGroup>
          </div>
        )}
      </div>
    </Page>
  );
};
