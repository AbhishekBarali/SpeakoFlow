import React from "react";
import "./CompletionMark.css";

/** The overlay's "done" state: a bare check that draws itself in.
 *
 * It replaces the working indicator rather than finishing it. The old ending
 * turned the indeterminate sweep into a solid bar, which read as a progress
 * line that had run to one end and jammed there — a waiting shape asked to
 * mean "finished". A check says finished on its own, so it carries no disc
 * behind it and no "Done" word beside it; both only repeated what the check
 * already says.
 *
 * All motion is CSS, so it runs on the compositor and disappears under
 * `prefers-reduced-motion` with no JavaScript involved. */
const CompletionMark: React.FC<{ label: string }> = ({ label }) => (
  <span className="completion-mark" role="img" aria-label={label}>
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path
        className="completion-check"
        d="M1.6 8.6 5 11.8 12.2 4.4"
        pathLength={1}
      />
    </svg>
  </span>
);

export default CompletionMark;
