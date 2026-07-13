import React from "react";
import { ChevronLeft } from "lucide-react";
import { useTranslation } from "react-i18next";

interface SubPageProps {
  /** Sub-page title, shown next to the back affordance. */
  title: string;
  /** Optional one-line caption under the title. */
  description?: string;
  /** Called when the user taps the back button to return to the parent page. */
  onBack: () => void;
  children: React.ReactNode;
}

/**
 * A drill-down page inside a settings section: a back button + title header,
 * then the page content swapped in below. The parent section owns which
 * sub-page (if any) is open; this primitive is purely presentational so every
 * section stacks its deeper pages the same way.
 *
 * Keep it minimal — it mirrors the section header style so a sub-page reads as
 * "one level in", not a different design.
 */
export const SubPage: React.FC<SubPageProps> = ({
  title,
  description,
  onBack,
  children,
}) => {
  const { t } = useTranslation();

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <div>
        <button
          type="button"
          onClick={onBack}
          className="group -ms-1.5 flex items-center gap-1 rounded-lg px-1.5 py-1 text-[13px] text-muted transition-colors hover:bg-ink/6 hover:text-ink cursor-pointer"
        >
          <ChevronLeft
            width={16}
            height={16}
            className="transition-transform group-hover:-translate-x-0.5 motion-reduce:transition-none"
          />
          <span>{t("common.back")}</span>
        </button>
        <div className="mt-2">
          <h2 className="font-display text-xl leading-tight text-ink">
            {title}
          </h2>
          {description && (
            <p className="mt-1 text-sm leading-snug text-muted">
              {description}
            </p>
          )}
        </div>
      </div>
      <div>{children}</div>
    </div>
  );
};
