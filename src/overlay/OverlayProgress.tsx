import React from "react";
import "./OverlayProgress.css";

/** The waiting indicator for every state that is neither recording nor a
 * notice: transcription, AI cleanup, Flow generation, screen vision.
 *
 * It is indeterminate, and it is that way because none of those steps can
 * report a fraction — so the component holds no progress value, no timeline and
 * no clock of its own. It renders a block that is always fully present with a
 * seamless sweep of light crossing it (see OverlayProgress.css), which is a
 * shape that cannot claim the work is 94% done. Completion is the one real
 * event, and it is a class change, not the end of an animation.
 *
 * All motion lives in CSS, so it runs on the compositor rather than React's
 * render clock, pausing with the window and disappearing under
 * `prefers-reduced-motion` without any JavaScript involved. */
const OverlayProgress: React.FC<{
  label: string;
  completed: boolean;
  /** False while the overlay window is hidden, which parks the sweep. */
  active: boolean;
}> = ({ label, completed, active }) => (
  <div
    className={`overlay-progress${completed ? " is-complete" : ""}${
      active ? "" : " is-paused"
    }`}
    role="progressbar"
    aria-label={label}
    aria-valuemin={0}
    aria-valuemax={100}
    // Deliberately absent until a result exists: assistive technology should
    // hear "busy", not a number the app invented.
    aria-valuenow={completed ? 100 : undefined}
  >
    <span className="progress-sheen" aria-hidden="true" />
  </div>
);

export default OverlayProgress;
