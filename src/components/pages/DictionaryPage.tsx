import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus } from "lucide-react";
import { Page, PageHeader } from "@/components/ui/Page";
import { Tabs } from "@/components/ui/Tabs";
import { Button } from "@/components/ui/Button";
import { Hero, HeroTitle } from "@/components/ui/Hero";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { CustomWords } from "@/components/settings/CustomWords";
import { AutoLearnCorrections } from "@/components/settings/AutoLearnCorrections";
import { TextReplacements } from "@/components/settings/TextReplacements";

type DictionaryTab = "words" | "replacements";

const WORD_INPUT_ID = "dictionary-new-word";

/**
 * Dictionary: the words SpeakoFlow should always get right, and the phrases
 * it should expand, as two tabs. The banner stays like every other page's;
 * its one line of how-to sits behind the lightbulb.
 */
export const DictionaryPage: React.FC = () => {
  const { t } = useTranslation();
  const [tab, setTab] = useState<DictionaryTab>("words");

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

      <Hero
        art="dictionary"
        className="mb-6"
        title={<HeroTitle i18nKey="dictionary.hero.title" />}
        subtitle={t("dictionary.hero.subtitle")}
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
