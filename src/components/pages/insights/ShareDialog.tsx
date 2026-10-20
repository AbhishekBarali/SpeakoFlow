import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { type as osType } from "@tauri-apps/plugin-os";
import { toast } from "sonner";
import {
  ArrowUpRight,
  Check,
  ChevronLeft,
  ChevronRight,
  Copy,
  Download,
  Ellipsis,
  RectangleHorizontal,
  RectangleVertical,
  Square,
} from "lucide-react";
import { commands, type Insights } from "@/bindings";
import { SocialMark, type SocialSite } from "@/components/icons/SocialLogos";
import { Button } from "@/components/ui/Button";
import { Dialog, DialogCloseButton } from "@/components/ui/Dialog";
import { Segmented } from "@/components/ui/Segmented";
import {
  blobToBase64,
  CARD_GROUNDS,
  cardBlob,
  drawCard,
  loadCardAssets,
} from "./shareCard";
import {
  buildCardDeck,
  CARD_FORMATS,
  CARD_SIZES,
  type CardFormat,
} from "./shareContent";
import {
  composeUrl,
  SHARE_TARGETS,
  TAKES_PASTED_IMAGE,
  TARGET_NAMES,
} from "./shareTargets";

/** The size last used, so a second share starts where the first left off.
 *  A convenience: losing it costs nothing. */
const PREFS_KEY = "speakoflow.insights.share";

const readFormat = (): CardFormat => {
  try {
    const stored = JSON.parse(
      window.localStorage.getItem(PREFS_KEY) ?? "{}",
    ) as { format?: string };
    return CARD_FORMATS.includes(stored.format as CardFormat)
      ? (stored.format as CardFormat)
      : "square";
  } catch {
    return "square";
  }
};

const writeFormat = (format: CardFormat) => {
  try {
    window.localStorage.setItem(PREFS_KEY, JSON.stringify({ format }));
  } catch {
    // Private storage or a full disk: the next share starts from square.
  }
};

/** The system this runs on, for the paste keys and the share sheet. */
const system = (() => {
  try {
    return osType();
  } catch {
    return "windows";
  }
})();
const PASTE_KEYS = system === "macos" ? "⌘V" : "Ctrl+V";
/** Windows and macOS have a share sheet; Linux does not. */
const HAS_SHARE_SHEET = system === "windows" || system === "macos";

const FORMAT_ICONS: Record<
  CardFormat,
  React.ComponentType<{ className?: string }>
> = {
  square: Square,
  story: RectangleVertical,
  wide: RectangleHorizontal,
};

/** Room above and below a card in its slot, in pixels. */
const SLOT_PAD = 14;
/** Room either side of a card, so the arrows never sit on it. */
const ARROW_ROOM = 72;

/** Everything but Copy: quiet, so the one dark button is the obvious one. */
const roundButton =
  "grid h-11 w-11 shrink-0 cursor-pointer place-items-center rounded-full border border-hairline bg-surface text-ink transition-[transform,background-color] duration-150 hover:bg-surface-strong active:scale-95 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 focus-visible:ring-offset-2 focus-visible:ring-offset-canvas disabled:cursor-default disabled:opacity-40";

const arrowButton =
  "z-10 grid h-10 w-10 shrink-0 cursor-pointer place-items-center rounded-full border border-hairline bg-surface text-ink shadow-[0_4px_14px_-6px_rgba(10,20,24,0.3)] transition-[opacity,background-color] hover:bg-surface-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 disabled:pointer-events-none disabled:opacity-0";

interface Rendered {
  url: string;
  blob: Blob;
}

/**
 * Share the Insights numbers as a deck of cards: one insight each, moved
 * through with the arrows, the dots, a swipe or the arrow keys, and shared
 * from the row under it: copied, saved, posted to a site (made ready first,
 * with a line saying how it goes into the post) or handed to the system's
 * share sheet.
 * Every card is drawn here, on this computer; nothing is uploaded.
 */
export const ShareDialog: React.FC<{
  open: boolean;
  onClose: () => void;
  insights: Insights;
}> = ({ open, onClose, insights }) => {
  const { t, i18n } = useTranslation();
  const rtl = i18n.dir() === "rtl";
  const [format, setFormat] = useState<CardFormat>(readFormat);
  const [index, setIndex] = useState(0);
  const [rendered, setRendered] = useState<Array<Rendered | null>>([]);
  const [failed, setFailed] = useState(false);
  const [busy, setBusy] = useState<"save" | "copy" | "share" | null>(null);
  const [copied, setCopied] = useState(false);
  /** A post being made: the card is ready, and this says how it goes in. */
  const [posting, setPosting] = useState<{
    site: SocialSite;
    /** The words of the card that was made ready, for the post. */
    post: string;
    /** Where the card was saved, for a site that takes no pasted image. */
    path?: string;
  } | null>(null);
  /** Bumped when the card or its size changes, so a post made ready for the
   *  card before never shows its guide under this one. */
  const postFor = useRef(0);
  const copyButton = useRef<HTMLButtonElement>(null);
  /** Whether Copy has had its focus since the dialog opened. */
  const copyFocused = useRef(false);
  const swipe = useRef<{ x: number; id: number } | null>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ w: 0, h: 0 });

  // Each card has the whole width to itself and slides in from the side.
  useEffect(() => {
    const element = viewport.current;
    if (!open || !element) return;
    const measure = () =>
      setView({ w: element.clientWidth, h: element.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [open]);

  const deck = useMemo(
    () => buildCardDeck(insights, t, i18n.language),
    [insights, t, i18n.language],
  );
  const slides = deck.slides;
  const current = Math.min(index, Math.max(0, slides.length - 1));
  const slide = slides[current];
  const card = rendered[current] ?? null;

  // Draw every card at the chosen size, the one in view first.
  useEffect(() => {
    if (!open || slides.length === 0) return;
    let cancelled = false;
    const urls: string[] = [];
    setRendered(slides.map(() => null));
    setFailed(false);
    const order = slides
      .map((_, position) => position)
      .sort((a, b) => Math.abs(a - current) - Math.abs(b - current) || a - b);
    (async () => {
      try {
        const assets = await loadCardAssets();
        const canvas = document.createElement("canvas");
        for (const position of order) {
          if (cancelled) return;
          drawCard(canvas, slides[position], deck, format, assets, rtl);
          const blob = await cardBlob(canvas);
          if (cancelled) return;
          const url = URL.createObjectURL(blob);
          urls.push(url);
          setRendered((previous) => {
            const next = previous.slice();
            next[position] = { url, blob };
            return next;
          });
        }
      } catch {
        if (!cancelled) setFailed(true);
      }
    })();
    return () => {
      cancelled = true;
      urls.forEach((url) => URL.revokeObjectURL(url));
    };
    // `current` only orders the work; moving between cards must not redraw.
  }, [open, slides, deck, format, i18n.language, rtl]);

  // A closed dialog keeps no cards: their images are let go on close, and
  // the next open starts from the first card's placeholder.
  useEffect(() => {
    if (open) return;
    setIndex(0);
    setRendered([]);
    copyFocused.current = false;
  }, [open]);

  // What the post guide says is about the card it was made for.
  useEffect(() => {
    postFor.current += 1;
    setPosting(null);
  }, [open, index, format]);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1800);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const go = useCallback(
    (step: number) => {
      setCopied(false);
      setIndex((position) =>
        Math.max(0, Math.min(slides.length - 1, position + step)),
      );
    },
    [slides.length],
  );

  const changeFormat = (next: CardFormat) => {
    setFormat(next);
    writeFormat(next);
  };

  /** Put the card in view on the clipboard, saying why if it cannot.
   *  Called straight from a click: the webview's clipboard wants the
   *  click's activation, so the image goes over as a promise. */
  const writeClipboard = async (blob: Blob) => {
    try {
      if (typeof ClipboardItem !== "undefined" && navigator.clipboard?.write) {
        try {
          await navigator.clipboard.write([
            new ClipboardItem({ "image/png": Promise.resolve(blob) }),
          ]);
          return true;
        } catch {
          // Refused: the app's own clipboard below.
        }
      }
      const result = await commands.copyInsightsCard(await blobToBase64(blob));
      if (result.status === "ok") return true;
      toast.error(t("insights.share.copyFailed"), {
        description: result.error,
      });
    } catch (error) {
      toast.error(t("insights.share.copyFailed"), {
        description: String(error),
      });
    }
    return false;
  };

  const copyImage = async () => {
    if (!card || busy) return;
    setBusy("copy");
    try {
      if (await writeClipboard(card.blob)) setCopied(true);
    } finally {
      setBusy(null);
    }
  };

  const saveImage = async () => {
    if (!card || !slide || busy) return;
    setBusy("save");
    try {
      const path = await save({
        defaultPath: `speakoflow-${slide.kind}-${format}.png`,
        filters: [{ name: "PNG", extensions: ["png"] }],
      });
      if (!path) return;
      const result = await commands.saveInsightsCard(
        path,
        await blobToBase64(card.blob),
      );
      if (result.status === "ok") {
        toast.success(t("insights.share.saved"), {
          action: {
            label: t("insights.share.showInFolder"),
            onClick: () => void revealItemInDir(path).catch(() => {}),
          },
        });
      } else {
        toast.error(t("insights.share.saveFailed"), {
          description: result.error,
        });
      }
    } catch (error) {
      toast.error(t("insights.share.saveFailed"), {
        description: String(error),
      });
    } finally {
      setBusy(null);
    }
  };

  /**
   * Get the card ready for a post on `site` and say, here, how it goes in,
   * before the site opens. No site takes an image from a link, so the
   * person puts it in themselves, and a toast would be hidden behind the
   * browser by the time it mattered. X takes a pasted picture, so the card is
   * copied; LinkedIn takes none, so it is saved to Downloads for the post's
   * photo button.
   */
  const preparePost = async (site: SocialSite) => {
    if (!card || !slide || busy) return;
    const post = slide.post;
    const made = postFor.current;
    setBusy("copy");
    try {
      if (TAKES_PASTED_IMAGE[site]) {
        const copied = await writeClipboard(card.blob);
        if (copied && made === postFor.current) setPosting({ site, post });
        return;
      }
      const result = await commands.saveInsightsCardToDownloads(
        await blobToBase64(card.blob),
      );
      if (result.status === "ok") {
        if (made === postFor.current) {
          setPosting({ site, post, path: result.data });
        }
      } else {
        toast.error(t("insights.share.saveFailed"), {
          description: result.error,
        });
      }
    } catch (error) {
      toast.error(t("insights.share.saveFailed"), {
        description: String(error),
      });
    } finally {
      setBusy(null);
    }
  };

  /** Open a new post on `site` with the words of the card made ready. */
  const openPost = async (site: SocialSite, post: string) => {
    try {
      await openUrl(composeUrl(site, post));
    } catch (error) {
      toast.error(
        t("insights.share.openFailed", { site: TARGET_NAMES[site] }),
        {
          description: String(error),
        },
      );
    }
  };

  /** Hand the card to the system's share sheet, opened beside `button`. */
  const shareElsewhere = async (button: HTMLElement) => {
    if (!card || busy) return;
    setBusy("share");
    try {
      const rect = button.getBoundingClientRect();
      const result = await commands.shareInsightsCard(
        await blobToBase64(card.blob),
        t("insights.share.sheetTitle"),
        { x: rect.left, y: rect.top, width: rect.width, height: rect.height },
      );
      if (result.status === "error") {
        toast.error(t("insights.share.shareFailed"), {
          description: result.error,
        });
      }
    } catch (error) {
      toast.error(t("insights.share.shareFailed"), {
        description: String(error),
      });
    } finally {
      setBusy(null);
    }
  };

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    const target = event.target as HTMLElement;
    if (target.closest('[role="radiogroup"], [role="tablist"], input')) return;
    event.preventDefault();
    const forward = event.key === (rtl ? "ArrowLeft" : "ArrowRight");
    go(forward ? 1 : -1);
  };

  const { width, height } = CARD_SIZES[format];
  const slot = Math.max(1, view.w);
  const titleId = "insights-share-title";
  const ready = card !== null && !failed;
  const disabled = !ready || busy !== null;
  const Prev = rtl ? ChevronRight : ChevronLeft;
  const Next = rtl ? ChevronLeft : ChevronRight;

  // Copy is disabled until the first card is drawn, so the dialog opens with
  // the panel focused; Copy takes the focus once it can, unless the person
  // has already moved it.
  useEffect(() => {
    const button = copyButton.current;
    if (!open || !ready || copyFocused.current || !button) return;
    copyFocused.current = true;
    if (document.activeElement === button.closest('[role="dialog"]')) {
      button.focus({ preventScroll: true });
    }
  }, [open, ready]);

  return (
    <Dialog
      open={open}
      onClose={onClose}
      size="xl"
      bare
      labelledBy={titleId}
      className="bg-canvas!"
      bodyClassName=""
    >
      <div onKeyDown={onKeyDown} className="flex flex-col px-6 pt-5 pb-6">
        <div className="flex items-center justify-between gap-4">
          <h2 id={titleId} className="text-[0.9375rem] font-semibold text-ink">
            {t("insights.share.title")}
          </h2>
          <div className="flex items-center gap-3">
            <Segmented<CardFormat>
              size="sm"
              label={t("insights.share.size")}
              value={format}
              onChange={changeFormat}
              options={CARD_FORMATS.map((value) => ({
                value,
                label: t(`insights.share.formats.${value}`),
                icon: FORMAT_ICONS[value],
                title: t(`insights.share.formatHints.${value}`),
              }))}
            />
            <DialogCloseButton
              onClick={onClose}
              label={t("common.close")}
              className="m-0!"
            />
          </div>
        </div>

        <div className="relative -mx-6 mt-2">
          <div
            ref={viewport}
            className="relative h-[min(58vh,32rem)] min-w-0 touch-pan-y overflow-x-clip"
            onPointerDown={(event) => {
              swipe.current = { x: event.clientX, id: event.pointerId };
            }}
            onPointerUp={(event) => {
              const start = swipe.current;
              swipe.current = null;
              if (!start || start.id !== event.pointerId) return;
              const dx = event.clientX - start.x;
              if (Math.abs(dx) < 40) return;
              go(dx < 0 !== rtl ? 1 : -1);
            }}
            onPointerCancel={() => {
              swipe.current = null;
            }}
          >
            <div
              className="flex h-full transition-transform duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none"
              style={{
                transform: `translateX(${rtl ? current * slot : -current * slot}px)`,
              }}
            >
              {slides.map((item, position) => {
                const image = rendered[position];
                const inView = position === current;
                return (
                  <div
                    key={`${item.kind}-${format}`}
                    className={`flex h-full shrink-0 items-center justify-center transition-opacity duration-500 motion-reduce:transition-none ${
                      inView ? "" : "opacity-0"
                    }`}
                    style={{
                      width: `${slot}px`,
                      padding: `${SLOT_PAD}px ${ARROW_ROOM}px`,
                    }}
                    aria-hidden={!inView}
                  >
                    {image ? (
                      <img
                        src={image.url}
                        alt={position === current ? item.alt : ""}
                        draggable={false}
                        className="max-h-full max-w-full rounded-[1.125rem] shadow-[0_28px_56px_-26px_rgba(10,20,24,0.55),0_2px_6px_rgba(10,20,24,0.08)] select-none"
                        style={{ aspectRatio: `${width} / ${height}` }}
                      />
                    ) : (
                      <div
                        className="max-h-full max-w-full animate-pulse rounded-[1.125rem] opacity-60"
                        style={{
                          aspectRatio: `${width} / ${height}`,
                          height: "100%",
                          background: CARD_GROUNDS[item.kind],
                        }}
                      />
                    )}
                  </div>
                );
              })}
            </div>
            {failed && (
              <p className="absolute inset-0 grid place-items-center text-center text-sm text-muted">
                {t("insights.share.drawFailed")}
              </p>
            )}
          </div>

          <button
            type="button"
            className={`${arrowButton} absolute start-4 top-1/2 -translate-y-1/2`}
            onClick={() => go(-1)}
            disabled={current === 0}
            aria-label={t("insights.share.previous")}
          >
            <Prev className="h-5 w-5" aria-hidden="true" />
          </button>
          <button
            type="button"
            className={`${arrowButton} absolute end-4 top-1/2 -translate-y-1/2`}
            onClick={() => go(1)}
            disabled={current >= slides.length - 1}
            aria-label={t("insights.share.next")}
          >
            <Next className="h-5 w-5" aria-hidden="true" />
          </button>
        </div>

        <div
          className="mt-5 flex justify-center gap-1.5"
          role="group"
          aria-label={t("insights.share.cardsLabel")}
        >
          {slides.map((item, position) => (
            <button
              key={item.kind}
              type="button"
              onClick={() => setIndex(position)}
              aria-label={t("insights.share.position", {
                index: position + 1,
                count: slides.length,
              })}
              aria-current={position === current ? "true" : undefined}
              className="grid h-5 w-5 cursor-pointer place-items-center rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
            >
              <span
                className={`block h-2 rounded-full transition-[width,background-color] duration-200 ${
                  position === current ? "w-5 bg-ink" : "w-2 bg-ink/20"
                }`}
              />
            </button>
          ))}
        </div>

        <div className="mt-5 flex flex-wrap items-center justify-center gap-2.5">
          <button
            ref={copyButton}
            type="button"
            onClick={() => void copyImage()}
            disabled={disabled}
            data-autofocus
            className="inline-flex h-11 cursor-pointer items-center gap-2 rounded-full bg-ink px-5 text-sm font-semibold text-canvas transition-[transform,opacity] duration-150 hover:opacity-90 active:scale-[0.98] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 focus-visible:ring-offset-2 focus-visible:ring-offset-canvas disabled:cursor-default disabled:opacity-40"
            aria-live="polite"
          >
            {copied ? (
              <Check className="h-4 w-4" aria-hidden="true" />
            ) : (
              <Copy className="h-4 w-4" aria-hidden="true" />
            )}
            {copied ? t("insights.share.copied") : t("insights.share.copy")}
          </button>
          <button
            type="button"
            onClick={() => void saveImage()}
            disabled={disabled}
            className={roundButton}
            aria-label={t("insights.share.save")}
            title={t("insights.share.save")}
          >
            <Download
              className="h-[1.125rem] w-[1.125rem]"
              aria-hidden="true"
            />
          </button>
          {SHARE_TARGETS.map((site) => (
            <button
              key={site}
              type="button"
              onClick={() => void preparePost(site)}
              disabled={disabled}
              className={roundButton}
              aria-label={t("insights.share.postOn", {
                site: TARGET_NAMES[site],
              })}
              title={TARGET_NAMES[site]}
            >
              <SocialMark site={site} size={17} />
            </button>
          ))}
          {HAS_SHARE_SHEET && (
            <button
              type="button"
              onClick={(event) => void shareElsewhere(event.currentTarget)}
              disabled={disabled}
              className={roundButton}
              aria-label={t("insights.share.more")}
              title={t("insights.share.more")}
            >
              <Ellipsis className="h-5 w-5" aria-hidden="true" />
            </button>
          )}
        </div>
        {posting ? (
          <div
            role="status"
            className="mx-auto mt-4 flex w-full max-w-2xl flex-wrap items-center gap-x-3 gap-y-2 rounded-2xl border border-hairline bg-surface px-4 py-3 elev-card"
          >
            <span className="grid h-8 w-8 shrink-0 place-items-center rounded-full bg-ink text-canvas">
              <SocialMark site={posting.site} size={14} />
            </span>
            <p className="min-w-0 flex-1 text-sm text-ink text-pretty">
              {posting.path
                ? t("insights.share.savedForPost", {
                    site: TARGET_NAMES[posting.site],
                  })
                : t("insights.share.copiedForPost", { keys: PASTE_KEYS })}
            </p>
            <div className="flex items-center gap-2">
              {posting.path && (
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => {
                    const path = posting.path;
                    if (path) void revealItemInDir(path).catch(() => {});
                  }}
                >
                  {t("insights.share.showInFolder")}
                </Button>
              )}
              <Button
                size="sm"
                onClick={() => void openPost(posting.site, posting.post)}
              >
                {t("insights.share.openSite", {
                  site: TARGET_NAMES[posting.site],
                })}
                <ArrowUpRight
                  className="h-3.5 w-3.5 rtl:-scale-x-100"
                  aria-hidden="true"
                />
              </Button>
            </div>
          </div>
        ) : (
          <p className="mt-3 text-center text-xs text-muted">
            {t("insights.share.private")}
          </p>
        )}
      </div>
    </Dialog>
  );
};
