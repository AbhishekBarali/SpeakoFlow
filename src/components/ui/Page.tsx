import React from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft } from "lucide-react";
import { useNavigation, type NavLocation } from "@/components/shell/navigation";
import { InfoTip } from "./InfoTip";

/**
 * Page layout primitives for the main window.
 *
 * Hierarchy is carried by three steps and nothing else: a page title, a
 * section title one step down, and labels inside cards — all Inter, separated
 * by size and weight. Explanations live behind an InfoTip next to the title
 * they explain, or in one short line where a feature needs introducing.
 */

type PageWidth = "narrow" | "default" | "wide";

const WIDTHS: Record<PageWidth, string> = {
  narrow: "max-w-3xl",
  default: "max-w-5xl",
  wide: "max-w-6xl",
};

export const Page: React.FC<{
  width?: PageWidth;
  children: React.ReactNode;
  className?: string;
}> = ({ width = "default", children, className = "" }) => (
  <div
    className={`@container mx-auto w-full ${WIDTHS[width]} px-6 pt-9 pb-16 sm:px-10 ${className}`}
  >
    {children}
  </div>
);

/** The label a back link shows for a location. */
export const useLocationLabel = () => {
  const { t } = useTranslation();
  return (location: NavLocation): string => t(`nav.${location.page}`);
};

/** "‹ Home" — the way back to wherever the user came from. */
export const BackLink: React.FC<{
  label: string;
  onClick: () => void;
  className?: string;
}> = ({ label, onClick, className = "" }) => (
  <button
    type="button"
    onClick={onClick}
    className={`group -ms-2 inline-flex cursor-pointer items-center gap-0.5 rounded-lg py-1 ps-1 pe-2.5 text-sm font-medium text-muted transition-colors hover:bg-ink/[0.05] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${className}`}
  >
    <ChevronLeft
      className="h-4 w-4 transition-transform duration-150 group-hover:-translate-x-0.5 motion-reduce:transition-none rtl:rotate-180"
      aria-hidden="true"
    />
    <span>{label}</span>
  </button>
);

interface PageHeaderProps {
  title: React.ReactNode;
  /** One sentence of context, shown behind an (i) next to the title. */
  description?: React.ReactNode;
  /** Controls at the right of the title row (switches, buttons). */
  actions?: React.ReactNode;
  /** An explicit back target (a drill-down inside a page). When omitted the
   *  header offers the navigation history's back step, if there is one. */
  onBack?: () => void;
  backLabel?: string;
  /** Small element after the title (a Beta badge). */
  badge?: React.ReactNode;
  className?: string;
}

export const PageHeader: React.FC<PageHeaderProps> = ({
  title,
  description,
  actions,
  onBack,
  backLabel,
  badge,
  className,
}) => {
  const { t } = useTranslation();
  const { back, goBack } = useNavigation();
  const labelFor = useLocationLabel();
  const backTarget = onBack
    ? { label: backLabel ?? t("common.back"), onClick: onBack }
    : back
      ? { label: labelFor(back), onClick: goBack }
      : null;

  return (
    <header className={className ?? "mb-8"}>
      {backTarget && (
        <BackLink
          label={backTarget.label}
          onClick={backTarget.onClick}
          className="mb-3"
        />
      )}
      <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
        <div className="flex min-w-0 flex-1 items-center gap-2.5">
          <h1 className="font-display text-[1.75rem] text-ink">{title}</h1>
          {badge}
          {description && (
            <InfoTip text={description} size="md" className="mt-1.5" />
          )}
        </div>
        {actions && (
          <div className="flex shrink-0 flex-wrap items-center gap-2">
            {actions}
          </div>
        )}
      </div>
    </header>
  );
};

/** Heading for a block of content inside a page. */
export const SectionTitle: React.FC<{
  title: React.ReactNode;
  /** Context behind an (i), not a caption. */
  description?: React.ReactNode;
  action?: React.ReactNode;
  className?: string;
  as?: "h2" | "h3";
}> = ({ title, description, action, className = "", as = "h2" }) => {
  const Heading = as;
  // A fixed minimum height, so two sections side by side line up whether one
  // carries a text link and the other a segmented control.
  return (
    <div
      className={`mb-3 flex min-h-8 items-center justify-between gap-3 ${className}`}
    >
      <div className="flex min-w-0 items-center gap-2">
        <Heading className="font-display text-[1.125rem] text-ink">
          {title}
        </Heading>
        {description && <InfoTip text={description} />}
      </div>
      {action && <div className="shrink-0">{action}</div>}
    </div>
  );
};

/** A quiet text link in a section header or card corner. */
export const TextLink: React.FC<{
  onClick: () => void;
  children: React.ReactNode;
  className?: string;
}> = ({ onClick, children, className = "" }) => (
  <button
    type="button"
    onClick={onClick}
    className={`inline-flex cursor-pointer items-center gap-1 rounded-md px-1.5 py-0.5 text-sm font-medium text-muted transition-colors hover:bg-ink/[0.05] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${className}`}
  >
    {children}
  </button>
);
