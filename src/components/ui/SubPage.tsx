import React from "react";
import { useTranslation } from "react-i18next";
import { PageHeader } from "./Page";

interface SubPageProps {
  /** Sub-page title, shown under the back affordance. */
  title: string;
  /** Optional one-line caption under the title. */
  description?: string;
  /** Called when the user taps the back button to return to the parent page. */
  onBack: () => void;
  /** Label for the back button. Defaults to "Back". */
  backLabel?: string;
  /** Optional controls at the right of the title row. */
  actions?: React.ReactNode;
  children: React.ReactNode;
}

/**
 * A drill-down page inside a section: a back link + title header, then the
 * page content. The parent owns which sub-page (if any) is open; this is purely
 * presentational so every section stacks its deeper pages the same way.
 */
export const SubPage: React.FC<SubPageProps> = ({
  title,
  description,
  onBack,
  backLabel,
  actions,
  children,
}) => {
  const { t } = useTranslation();

  return (
    <div className="w-full">
      <PageHeader
        title={title}
        description={description}
        onBack={onBack}
        backLabel={backLabel ?? t("common.back")}
        actions={actions}
      />
      <div>{children}</div>
    </div>
  );
};
