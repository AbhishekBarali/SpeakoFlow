import React from "react";
import { PageHeader } from "./Page";

interface SectionHeaderProps {
  /** Big page title (usually the sidebar label). */
  title: string;
  /** Optional one-line caption under the title. */
  description?: string;
  /** Optional controls at the right of the title row. */
  actions?: React.ReactNode;
}

/**
 * The page-level header each section renders at the top of its root page.
 * A thin wrapper over `PageHeader` so older sections and the new pages share
 * one heading style.
 */
export const SectionHeader: React.FC<SectionHeaderProps> = ({
  title,
  description,
  actions,
}) => (
  <PageHeader
    title={title}
    description={description}
    actions={actions}
    className="mb-0"
  />
);
