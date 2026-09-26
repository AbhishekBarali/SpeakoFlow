/* eslint-disable i18next/no-literal-string -- brand name is not translatable */
import React from "react";

interface WordmarkProps {
  className?: string;
}

/**
 * SpeakoFlow brand wordmark, in the brand sans (`.font-brand`) — the display
 * serif is for page titles, never for the name. "Flow" takes the accent so the
 * name and the brand color read as one mark.
 */
export const Wordmark: React.FC<WordmarkProps> = ({ className = "" }) => (
  <span
    className={`font-brand font-semibold tracking-tight text-ink leading-none select-none inline-block ${className}`}
  >
    Speako<span className="text-accent">Flow</span>
  </span>
);

export default Wordmark;
