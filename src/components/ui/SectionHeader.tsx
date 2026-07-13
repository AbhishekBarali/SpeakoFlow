import React from "react";

interface SectionHeaderProps {
  /** Big page title (usually the sidebar label). */
  title: string;
  /** Optional one-line caption under the title. */
  description?: string;
}

/**
 * The page-level header each settings section renders at the top of its root
 * page. Sections own their headers (rather than the app shell) so drill-down
 * sub-pages can replace the whole header instead of stacking a second title
 * underneath it.
 */
export const SectionHeader: React.FC<SectionHeaderProps> = ({
  title,
  description,
}) => (
  <header className="max-w-3xl w-full mx-auto">
    <h1 className="font-display text-2xl leading-tight text-ink">{title}</h1>
    {description && (
      <p className="mt-1 text-sm leading-snug text-muted max-w-xl">
        {description}
      </p>
    )}
  </header>
);
