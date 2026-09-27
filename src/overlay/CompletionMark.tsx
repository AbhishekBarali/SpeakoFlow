import React from "react";
import "./CompletionMark.css";

/** The overlay's "done" state: a tinted disc that settles in, then a check
 * that draws itself across it.
 *
 * It replaces the working indicator rather than finishing it. The old ending
 * turned the indeterminate sweep into a solid bar, which read as a progress
 * line that had run to one end and jammed there — a waiting shape asked to
 * mean "finished". A check says finished on its own, and drawing it (rather
 * than popping it in) gives the moment a beat of intent without being loud.
 *
 * All motion is CSS, so it runs on the compositor and disappears under
 * `prefers-reduced-motion` with no JavaScript involved. */
const CompletionMark: React.FC<{ label: string }> = ({ label }) => (
  <span className="completion-mark" role="img" aria-label={label}>
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <circle className="completion-disc" cx="8" cy="8" r="7.25" />
      <path
        className="completion-check"
        d="M4.9 8.35 7.05 10.4 11.2 5.95"
        pathLength={1}
      />
    </svg>
  </span>
);

export default CompletionMark;
