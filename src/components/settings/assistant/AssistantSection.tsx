import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight, Users, Notebook } from "lucide-react";
import { AssistantSettings } from "./AssistantSettings";
import { CharactersSettings } from "./CharactersSettings";
import { MemorySettings } from "./MemorySettings";
import { LlmCatalog } from "./LlmCatalog";
import { SubPage } from "../../ui/SubPage";
import { SectionHeader } from "../../ui/SectionHeader";
import {
  TONE_TILE_VIVID,
  type SettingIcon,
  type SettingTone,
} from "../../ui/tones";

/** Which drill-down page is open, if any. */
type AssistantSubPage = "characters" | "memory" | "llm-catalog" | null;

interface SubPageRowProps {
  icon: SettingIcon;
  tone: SettingTone;
  title: string;
  description?: string;
  onClick: () => void;
}

/** A prominent tappable card that opens a sub-page — icon tile, title, caption,
 *  trailing chevron. Used for the Profiles/Memory entries at the top of the
 *  Assistant page. */
const NavCard: React.FC<SubPageRowProps> = ({
  icon: Icon,
  tone,
  title,
  description,
  onClick,
}) => (
  <button
    type="button"
    onClick={onClick}
    className="flex w-full items-center gap-3 rounded-2xl border border-hairline bg-surface elev-card px-4 py-3.5 text-start transition-colors hover:bg-surface-strong hover:border-hairline-strong cursor-pointer"
  >
    <span
      className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-xl ${TONE_TILE_VIVID[tone]}`}
    >
      <Icon size={18} />
    </span>
    <span className="min-w-0 flex-1">
      <span className="block truncate text-[13.5px] font-medium text-ink">
        {title}
      </span>
      {description && (
        <span className="mt-0.5 block truncate text-xs text-muted">
          {description}
        </span>
      )}
    </span>
    <ChevronRight width={16} height={16} className="shrink-0 text-muted-soft" />
  </button>
);

/**
 * Assistant section shell.
 *
 * Renders the Assistant settings page (`AssistantSettings`), then exposes three
 * drill-down sub-pages via the shared `SubPage` primitive: the on-device model
 * catalog (opened from the brain picker's "Download a model…" row), Profiles
 * ("characters"), and Memory. The parent owns which sub-page is open so every
 * deeper page stacks the same way.
 */
export const AssistantSection: React.FC = () => {
  const { t } = useTranslation();
  const [subPage, setSubPage] = useState<AssistantSubPage>(null);

  if (subPage === "llm-catalog") {
    return (
      <SubPage
        title={t("settings.assistant.brain.catalogTitle")}
        description={t("settings.assistant.brain.catalogDescription")}
        onBack={() => setSubPage(null)}
      >
        <LlmCatalog />
      </SubPage>
    );
  }

  if (subPage === "characters") {
    return (
      <SubPage
        title={t("sidebar.characters")}
        description={t("settings.assistant.subpages.profilesCaption")}
        onBack={() => setSubPage(null)}
      >
        <CharactersSettings />
      </SubPage>
    );
  }

  if (subPage === "memory") {
    return (
      <SubPage
        title={t("sidebar.memory")}
        description={t("settings.assistant.subpages.memoryCaption")}
        onBack={() => setSubPage(null)}
      >
        <MemorySettings />
      </SubPage>
    );
  }

  return (
    <div className="w-full flex flex-col items-center gap-8">
      <SectionHeader
        title={t("sidebar.assistant")}
        description={t("sectionSubtitles.assistant")}
      />
      {/* Profiles + Memory — prominent entry cards at the top so they're
          impossible to miss. */}
      <div className="max-w-3xl w-full mx-auto grid grid-cols-1 sm:grid-cols-2 gap-3">
        <NavCard
          icon={Users}
          tone="violet"
          title={t("sidebar.characters")}
          description={t("settings.assistant.subpages.profilesCaption")}
          onClick={() => setSubPage("characters")}
        />
        <NavCard
          icon={Notebook}
          tone="emerald"
          title={t("sidebar.memory")}
          description={t("settings.assistant.subpages.memoryCaption")}
          onClick={() => setSubPage("memory")}
        />
      </div>
      <AssistantSettings onOpenLlmCatalog={() => setSubPage("llm-catalog")} />
    </div>
  );
};
