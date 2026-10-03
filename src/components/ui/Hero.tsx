import React, {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { Trans, useTranslation } from "react-i18next";
import { Lightbulb } from "lucide-react";
import { useResolvedTheme } from "@/hooks/useResolvedTheme";
import assistantArt from "@/assets/hero/assistant.webp";
import assistantArtLight from "@/assets/hero/assistant-light.webp";
import meetingsArt from "@/assets/hero/meetings.webp";
import meetingsArtLight from "@/assets/hero/meetings-light.webp";

/**
 * The banner at the top of a feature page: a stage with flat artwork on its
 * right (rendered by `scripts/hero-art.py`), a serif headline, and the one
 * control that matters on that page — the shortcut keys you click to change,
 * or Start recording.
 *
 * Only pages whose banner carries that page's main action have one:
 * Assistant (the ask and call keys) and Meetings (Start). Home, AI cleanup
 * and Dictionary had one each and lost it, because there the banner was
 * decoration above controls that a plain row holds just as well.
 *
 * The stage follows the theme. It used to be near-black in both, on the idea
 * that the light theme's one dark object reads as a banner; in practice it
 * was the highest-contrast thing on the page and pulled the eye off the
 * content under it. On the light theme it is now a pale tinted surface with
 * ink-on-paper art (the `-light` renders); the dark theme is unchanged.
 *
 * Instructions are tips, not furniture: anything wrapped in `HeroTip` (the
 * subtitle, a usage hint under a shortcut) starts folded away, and the
 * lightbulb in the corner opens it. What always shows is the headline, the
 * art, and the controls.
 */

const ART = {
  assistant: { dark: assistantArt, light: assistantArtLight },
  meetings: { dark: meetingsArt, light: meetingsArtLight },
} as const;

export type HeroArtwork = keyof typeof ART;

/** A banner's artwork for the theme on screen. */
export const useHeroArt = (art: HeroArtwork): string =>
  ART[art][useResolvedTheme()];

/**
 * Banners whose art has already made its entrance this session. The entrance
 * is a hello, not a page transition: a page is suspended to display:none
 * while hidden (see `Freeze`), which restarts any CSS animation still on it,
 * so the class has to come off once it has played or every visit replays it.
 */
const entered = new Set<HeroArtwork>();

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
  const [entering, setEntering] = useState(() => !entered.has(art));
  const artRef = useRef<HTMLImageElement>(null);
  const artSrc = useHeroArt(art);
  useEffect(() => {
    entered.add(art);
    // Leaving the page mid-entrance cancels the animation instead of ending
    // it, and it must not replay on the way back. React has no prop for
    // `animationcancel`, so it is listened for directly.
    const element = artRef.current;
    if (!element) return;
    const done = () => setEntering(false);
    element.addEventListener("animationcancel", done);
    return () => element.removeEventListener("animationcancel", done);
  }, [art]);

  return (
    <TipsOpen.Provider value={open}>
      <section className={`hero-stage rounded-[1.25rem] ${className}`}>
        <img
          ref={artRef}
          src={artSrc}
          alt=""
          aria-hidden="true"
          draggable={false}
          decoding="async"
          data-art={art}
          onAnimationEnd={() => setEntering(false)}
          className={`hero-art ${entering ? "hero-art-enter" : ""}`}
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
