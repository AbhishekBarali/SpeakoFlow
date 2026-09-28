import React from "react";
import "./OverlayWorking.css";

/** The overlay's waiting indicator: three dots that brighten in turn.
 *
 * It replaced a bar with light sweeping along it, which read as a progress line
 * racing left to right forever — busy and a little frantic for something the
 * user can only wait on. Dots that breathe in sequence say "working on it" at a
 * resting pace, and they occupy the same footprint as the five-bar microphone
 * indicator before them, so recording → transcribing → done changes what the
 * indicator does without changing where anything sits.
 *
 * It is indeterminate on purpose: none of the steps it covers (transcription,
 * AI cleanup, Flow, screen vision) can report a fraction, so it carries no
 * value, and assistive technology hears "busy" rather than a number the app
 * invented. All motion is CSS, so it pauses with the window and disappears
 * under `prefers-reduced-motion` with no JavaScript involved. */
const OverlayWorking: React.FC<{
  label: string;
  /** False while the overlay window is hidden, which parks the animation. */
  active: boolean;
}> = ({ label, active }) => (
  <span
    className={`overlay-working${active ? "" : " is-paused"}`}
    role="progressbar"
    aria-label={label}
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={undefined}
  >
    <span className="overlay-working-dot" aria-hidden="true" />
    <span className="overlay-working-dot" aria-hidden="true" />
    <span className="overlay-working-dot" aria-hidden="true" />
  </span>
);

export default OverlayWorking;
