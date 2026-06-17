/* eslint-disable i18next/no-literal-string -- brand name is not translatable */
import React from "react";

interface WordmarkProps {
  className?: string;
}

/**
 * SpeakoFlow brand wordmark. Set in the editorial display serif so it speaks
 * the same type language as the section headers, rather than a mismatched
 * hand-drawn logo.
 */
export const Wordmark: React.FC<WordmarkProps> = ({ className = "" }) => (
  <span
    className={`font-display font-medium tracking-tight text-ink leading-none select-none inline-block ${className}`}
  >
    Speako<span className="text-muted">Flow</span>
  </span>
);

export default Wordmark;
