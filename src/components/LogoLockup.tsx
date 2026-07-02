/* eslint-disable i18next/no-literal-string -- brand name is not translatable */
import React from "react";
import Logo from "./Logo";

interface LogoLockupProps {
  /** Extra classes for the wrapper (margins, alignment, etc.). */
  className?: string;
  /** Sizing for the mark; the wordmark is tuned to sit beside `h-6`. */
  iconClassName?: string;
  /** Accessible label for the whole lockup. */
  title?: string;
}

/**
 * SpeakoFlow brand lockup — the waveform speech-bubble mark next to the
 * "SpeakoFlow" wordmark, matching the reference logo.
 *
 * The whole lockup sits on a transparent background: the mark fills via
 * `currentColor` and the wordmark inherits the same color, so pairing the
 * wrapper with `text-ink` makes both adapt to light/dark automatically. The
 * wordmark is set in Inter semibold (`.font-brand`) to match the
 * geometric sans of the reference art.
 */
export const LogoLockup: React.FC<LogoLockupProps> = ({
  className = "",
  iconClassName = "h-6 w-auto",
  title = "SpeakoFlow",
}) => (
  <div
    className={`flex items-center gap-2 text-ink select-none ${className}`}
    role="img"
    aria-label={title}
  >
    <Logo className={`${iconClassName} shrink-0`} />
    <span className="font-brand text-lg leading-none whitespace-nowrap">
      SpeakoFlow
    </span>
  </div>
);

export default LogoLockup;
