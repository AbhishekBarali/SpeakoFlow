import React, { createContext, useContext, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { Lightbulb } from "lucide-react";
import homeArt from "@/assets/hero/home.webp";
import assistantArt from "@/assets/hero/assistant.webp";
import meetingsArt from "@/assets/hero/meetings.webp";
import cleanupArt from "@/assets/hero/cleanup.webp";
import dictionaryArt from "@/assets/hero/dictionary.webp";

/**
 * The banner at the top of a feature page: a dark stage with a photograph of
 * light on its right (rendered by `scripts/hero-art.py`), a serif headline,
 * and the one control that matters on that page — usually the shortcut, drawn
 * as keys you click to change.
 *
 * The stage is dark in both themes on purpose. On the light theme it is the
 * one dark object on the page, which is what makes it read as a banner rather
 * than another card; on the dark theme it sits a step below the pane, like a
 * screen set into it. Its two earlier versions both failed at this: a teal
 * gradient that fought the brand teal on every control, then a flat grey card
 * that looked like every other card.
 *
 * Instructions are tips, not furniture: anything wrapped in `HeroTip` (the
 * subtitle, a usage hint under a shortcut, an example) starts folded away,
 * and the lightbulb in the corner opens it. What always shows is the
 * headline, the art, and the controls.
 */

const ART = {
  home: homeArt,
  assistant: assistantArt,
  meetings: meetingsArt,
  cleanup: cleanupArt,
  dictionary: dictionaryArt,
} as const;

export type HeroArtwork = keyof typeof ART;

const TipsOpen = createContext(true);

/**
 * Collapses its content smoothly when `open` turns false, and keeps it out of
 * the accessibility tree while closed. Content keeps its size while folding,
 * so text never rewraps mid-animation.
 */
const Fold: React.FC<{
  open: boolean;
  children: React.ReactNode;
  className?: string;
}> = ({ open, children, className = "" }) => (
  <div className="hero-fold" data-open={open} aria-hidden={!open}>
    <div>
      <div className={className}>{children}</div>
    </div>
  </div>
);

/** Banner content that is a tip: shown while the banner's tips are. */
export const HeroTip: React.FC<{
  children: React.ReactNode;
  className?: string;
}> = ({ children, className }) => {
  const open = useContext(TipsOpen);
  return (
    <Fold open={open} className={className}>
      {children}
    </Fold>
  );
};

export const Hero: React.FC<{
  art: HeroArtwork;
  title: React.ReactNode;
  /** One short line under the title. A tip. */
  subtitle?: React.ReactNode;
  /** More tip content under the subtitle (an example). */
  children?: React.ReactNode;
  /** Always on the stage: shortcut keys, a primary button. */
  actions?: React.ReactNode;
  className?: string;
}> = ({ art, title, subtitle, children, actions, className = "" }) => {
  const { t } = useTranslation();
  // Tips start folded: the banner is the headline and its controls, and the
  // lightbulb opens the how-to for whoever wants it. Not remembered across
  // launches on purpose; a tip left open once would otherwise be back on
  // screen every day, which is the thing this replaced.
  const [open, setOpen] = useState(false);
  const hasTips = Boolean(subtitle || children);

  return (
    <TipsOpen.Provider value={open}>
      <section className={`hero-stage rounded-[1.25rem] ${className}`}>
        <img
          src={ART[art]}
          alt=""
          aria-hidden="true"
          draggable={false}
          decoding="async"
          className="hero-art"
        />
        <div className="relative flex min-h-[10.5rem] flex-col justify-center px-7 py-7 @3xl:max-w-[54%] @3xl:px-9 @3xl:py-8">
          <h2 className="hero-title pe-10 @3xl:pe-0">{title}</h2>
          {subtitle && (
            <HeroTip className="pt-2.5">
              <p className="max-w-md text-[0.9375rem] leading-relaxed text-hero-muted text-pretty">
                {subtitle}
              </p>
            </HeroTip>
          )}
          {children && <HeroTip className="pt-4">{children}</HeroTip>}
          {actions && <div className="pt-6">{actions}</div>}
        </div>
        {hasTips && (
          <button
            type="button"
            onClick={() => setOpen((value) => !value)}
            aria-label={open ? t("common.hideTips") : t("common.showTips")}
            title={open ? t("common.hideTips") : t("common.showTips")}
            aria-pressed={open}
            className="hero-corner absolute end-3.5 top-3.5 grid h-8 w-8 cursor-pointer place-items-center rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-hero-ink/70"
          >
            <Lightbulb className="h-4 w-4" aria-hidden="true" />
          </button>
        )}
      </section>
    </TipsOpen.Provider>
  );
};

/**
 * A headline from a translation key, where `<em>…</em>` in the string is set
 * in the serif's italic ("Say it messy. Get it <em>clean.</em>"). A
 * translation without the tag simply renders upright.
 */
export const HeroTitle: React.FC<{ i18nKey: string }> = ({ i18nKey }) => (
  <Trans i18nKey={i18nKey} components={{ em: <em /> }} />
);

/**
 * A labelled shortcut on the stage: "Dictate" over its keys. `hint` is how to
 * use it, a tip; `status` is something the user needs to know right now
 * ("Turn on AI cleanup to use this shortcut") and never folds away.
 */
export const HeroShortcut: React.FC<{
  label: string;
  hint?: React.ReactNode;
  status?: React.ReactNode;
  children: React.ReactNode;
}> = ({ label, hint, status, children }) => (
  <div className="flex min-w-0 flex-col items-start">
    <span className="mb-2 text-[0.8125rem] font-medium text-hero-muted">
      {label}
    </span>
    {children}
    {status && <span className="mt-2 text-xs text-hero-ink/85">{status}</span>}
    {hint && !status && (
      <HeroTip className="pt-2">
        <span className="text-xs text-hero-muted">{hint}</span>
      </HeroTip>
    )}
  </div>
);
