import React from "react";
import "./OverlayProgress.css";

/** The waiting indicator for every state that is neither recording nor a
 * notice: transcription, AI cleanup, Flow generation, screen vision.
 *
 * It is indeterminate, and it is that way because none of those steps can
 * report a fraction — so the component holds no progress value, no timeline and
 * no clock of its own. It renders a block that is always fully present with a
 * seamless sweep of light crossing it (see OverlayProgress.css), which is a
 * shape that cannot claim the work is 94% done.
 *
 * It only ever means "busy". Completion is not an end state of this bar — a
 * sweep that went solid read as a line that had run to one end and stuck — so
 * the overlay swaps it for a `CompletionMark` instead.
 *
 * All motion lives in CSS, so it runs on the compositor rather than React's
 * render clock, pausing with the window and disappearing under
 * `prefers-reduced-motion` without any JavaScript involved. */
const OverlayProgress: React.FC<{
  label: string;
  /** False while the overlay window is hidden, which parks the sweep. */
  active: boolean;
}> = ({ label, active }) => (
  <div
    className={`overlay-progress${active ? "" : " is-paused"}`}
    role="progressbar"
    aria-label={label}
    aria-valuemin={0}
    aria-valuemax={100}
    // Deliberately absent: assistive technology should hear "busy", not a
    // number the app invented.
    aria-valuenow={undefined}
  >
    <span className="progress-sheen" aria-hidden="true" />
  </div>
);

export default OverlayProgress;
