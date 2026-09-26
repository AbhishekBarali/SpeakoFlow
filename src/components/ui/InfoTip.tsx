import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Info } from "lucide-react";
import { Tooltip } from "./Tooltip";
import { splitTip } from "./tipText";

/**
 * The (i) that holds an explanation, so the explanation does not sit on the
 * page. Hover or focus shows it; a click pins it open (for touch screens and
 * for anyone reading slowly), and Escape or a click elsewhere closes it.
 *
 * A tip is one sentence. Longer explanations show their first sentence, and
 * the rest sits behind "More" in the pinned tip — so an (i) never opens onto a
 * paragraph, and nothing is lost.
 */
export const InfoTip: React.FC<{
  text: React.ReactNode;
  /** Accessible name for the button. Defaults to "More information". */
  label?: string;
  className?: string;
  /** Tone for placement on the gradient hero. */
  tone?: "default" | "onHero";
  size?: "sm" | "md";
}> = ({ text, label, className = "", tone = "default", size = "sm" }) => {
  const { t } = useTranslation();
  const [hovered, setHovered] = useState(false);
  const [pinned, setPinned] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const ref = useRef<HTMLButtonElement>(null);
  const tipRef = useRef<HTMLDivElement>(null);
  const closeTimer = useRef<number | null>(null);
  const tipId = useId();
  const open = hovered || pinned;
  const split = typeof text === "string" ? splitTip(text) : null;

  // Hover on and off with a short grace period, so the pointer can cross the
  // gap between the (i) and the tip to reach "More" without it closing.
  const hoverOn = () => {
    if (closeTimer.current !== null) {
      window.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
    setHovered(true);
  };
  const hoverOff = () => {
    if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    closeTimer.current = window.setTimeout(() => {
      closeTimer.current = null;
      setHovered(false);
    }, 140);
  };
  useEffect(
    () => () => {
      if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    },
    [],
  );

  useEffect(() => {
    if (!open) setExpanded(false);
  }, [open]);

  useEffect(() => {
    if (!pinned) return;
    const close = (event: MouseEvent) => {
      const target = event.target as Node;
      if (ref.current?.contains(target) || tipRef.current?.contains(target)) {
        return;
      }
      setPinned(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setPinned(false);
      }
    };
    document.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [pinned]);

  const iconSize = size === "md" ? "h-4 w-4" : "h-[0.875rem] w-[0.875rem]";

  return (
    <>
      <button
        ref={ref}
        type="button"
        aria-label={label ?? t("common.moreInfo")}
        aria-describedby={open ? tipId : undefined}
        aria-expanded={pinned}
        onMouseEnter={hoverOn}
        onMouseLeave={hoverOff}
        onFocus={hoverOn}
        onBlur={hoverOff}
        onClick={(event) => {
          event.stopPropagation();
          setPinned((value) => !value);
        }}
        className={`inline-grid shrink-0 cursor-help place-items-center rounded-full p-0.5 transition-colors focus-visible:outline-none focus-visible:ring-2 ${
          tone === "onHero"
            ? "text-white/60 hover:text-white focus-visible:ring-white/60"
            : "text-muted-soft hover:text-muted focus-visible:ring-accent/40"
        } ${className}`}
      >
        <Info className={iconSize} strokeWidth={1.9} aria-hidden="true" />
      </button>
      {open && (
        <Tooltip targetRef={ref} position="top">
          <div
            ref={tipRef}
            id={tipId}
            role="tooltip"
            // Keeps a hovered tip open while the pointer moves onto it, so
            // "More" can be reached without pinning first.
            onMouseEnter={hoverOn}
            onMouseLeave={hoverOff}
            className="text-start text-[0.8125rem] leading-relaxed text-body"
          >
            {split ? (
              <>
                {split.lead}
                {split.rest &&
                  (expanded ? (
                    <span className="mt-1.5 block text-muted">
                      {split.rest}
                    </span>
                  ) : (
                    <button
                      type="button"
                      onClick={(event) => {
                        event.stopPropagation();
                        setPinned(true);
                        setExpanded(true);
                      }}
                      className="ms-1 cursor-pointer font-medium text-accent hover:underline focus-visible:outline-none focus-visible:underline"
                    >
                      {t("common.more")}
                    </button>
                  ))}
              </>
            ) : (
              text
            )}
          </div>
        </Tooltip>
      )}
    </>
  );
};
