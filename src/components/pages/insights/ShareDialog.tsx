import React, { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import {
  Check,
  Copy,
  Download,
  RectangleHorizontal,
  RectangleVertical,
  Square,
} from "lucide-react";
import { commands, type Insights } from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Segmented } from "@/components/ui/Segmented";
import { Switch } from "@/components/ui/Switch";
import { hasActivity } from "./insightsMath";
import {
  blobToBase64,
  cardBlob,
  drawCard,
  loadCardAssets,
  LOOK_SWATCHES,
} from "./shareCard";
import {
  buildCardContent,
  CARD_FORMATS,
  CARD_LOOKS,
  CARD_SIZES,
  type CardFormat,
  type CardInclude,
  type CardLook,
} from "./shareContent";

/** The size and look last used, so a second share starts where the first
 *  left off. A convenience: losing it costs nothing. */
const PREFS_KEY = "speakoflow.insights.share";

interface Prefs {
  format: CardFormat;
  look: CardLook;
}

const readPrefs = (): Prefs => {
  try {
    const stored = JSON.parse(
      window.localStorage.getItem(PREFS_KEY) ?? "{}",
    ) as Partial<Prefs>;
    return {
      format: CARD_FORMATS.includes(stored.format as CardFormat)
        ? (stored.format as CardFormat)
        : "square",
      look: CARD_LOOKS.includes(stored.look as CardLook)
        ? (stored.look as CardLook)
        : "paper",
    };
  } catch {
    return { format: "square", look: "paper" };
  }
};

const writePrefs = (prefs: Prefs) => {
  try {
    window.localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
  } catch {
    // Private storage or a full disk: the next share starts from the defaults.
  }
};

const FORMAT_ICONS: Record<
  CardFormat,
  React.ComponentType<{ className?: string }>
> = {
  square: Square,
  story: RectangleVertical,
  wide: RectangleHorizontal,
};

/** Four looks as swatches, one choice: a radio group with arrow keys. */
const LookPicker: React.FC<{
  value: CardLook;
  onChange: (look: CardLook) => void;
}> = ({ value, onChange }) => {
  const { t } = useTranslation();
  const buttons = useRef<Partial<Record<CardLook, HTMLButtonElement | null>>>(
    {},
  );
  const onKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    const rtl = document.documentElement.dir === "rtl";
    const steps: Record<string, number> = {
      ArrowRight: rtl ? -1 : 1,
      ArrowDown: 1,
      ArrowLeft: rtl ? 1 : -1,
      ArrowUp: -1,
    };
    const step = steps[event.key];
    if (!step) return;
    event.preventDefault();
    const index = CARD_LOOKS.indexOf(value);
    const next =
      CARD_LOOKS[(index + step + CARD_LOOKS.length) % CARD_LOOKS.length];
    onChange(next);
    buttons.current[next]?.focus();
  };
  return (
    <div
      role="radiogroup"
      aria-label={t("insights.share.look")}
      className="grid grid-cols-4 gap-2"
    >
      {CARD_LOOKS.map((look) => {
        const selected = look === value;
        const swatch = LOOK_SWATCHES[look];
        return (
          <button
            key={look}
            ref={(element) => {
              buttons.current[look] = element;
            }}
            type="button"
            role="radio"
            aria-checked={selected}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(look)}
            onKeyDown={onKeyDown}
            className="group flex cursor-pointer flex-col items-center gap-1.5 rounded-xl p-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
          >
            <span
              aria-hidden="true"
              className={`relative grid h-11 w-full place-items-center rounded-lg border transition-[box-shadow,border-color] duration-150 ${
                selected
                  ? "border-transparent shadow-[0_0_0_2px_var(--color-surface),0_0_0_4px_var(--color-ink)]"
                  : "border-hairline-strong group-hover:border-muted-soft"
              }`}
              style={{ background: swatch.ground }}
            >
              <span
                className="h-2.5 w-6 rounded-full"
                style={{ background: swatch.accent }}
              />
            </span>
            <span
              className={`text-xs ${selected ? "font-medium text-ink" : "text-muted"}`}
            >
              {t(`insights.share.looks.${look}`)}
            </span>
          </button>
        );
      })}
    </div>
  );
};

const ControlLabel: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => <p className="mb-2 text-[0.8125rem] font-medium text-ink">{children}</p>;

/**
 * Turn the Insights numbers into an image to post anywhere: pick a size for
 * where it is going and a look, then save it or copy it. The card is drawn
 * here, on this computer; nothing is uploaded.
 */
export const ShareDialog: React.FC<{
  open: boolean;
  onClose: () => void;
  insights: Insights;
}> = ({ open, onClose, insights }) => {
  const { t, i18n } = useTranslation();
  const [prefs, setPrefs] = useState<Prefs>(readPrefs);
  const [include, setInclude] = useState<CardInclude>({
    assistant: true,
    meetings: true,
  });
  const [ready, setReady] = useState(false);
  const [failed, setFailed] = useState(false);
  const [busy, setBusy] = useState<"save" | "copy" | null>(null);
  const [copied, setCopied] = useState(false);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const canInclude = {
    assistant: hasActivity(insights, "assistant"),
    meetings: hasActivity(insights, "meetings"),
  };
  const showAssistant = include.assistant && canInclude.assistant;
  const showMeetings = include.meetings && canInclude.meetings;

  const content = useMemo(
    () =>
      buildCardContent(insights, t, i18n.language, {
        assistant: showAssistant,
        meetings: showMeetings,
      }),
    [insights, t, i18n.language, showAssistant, showMeetings],
  );

  useEffect(() => {
    if (!open) {
      // The canvas goes with the dialog; a reopened one is blank until drawn.
      setReady(false);
      return;
    }
    if (!content) return;
    let cancelled = false;
    loadCardAssets()
      .then((assets) => {
        if (cancelled || !canvasRef.current) return;
        drawCard(canvasRef.current, content, prefs.format, prefs.look, assets);
        setReady(true);
        setFailed(false);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [open, content, prefs.format, prefs.look]);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1800);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const update = (next: Partial<Prefs>) => {
    setPrefs((current) => {
      const merged = { ...current, ...next };
      writePrefs(merged);
      return merged;
    });
  };

  const saveImage = async () => {
    const canvas = canvasRef.current;
    if (!canvas || busy) return;
    setBusy("save");
    try {
      const blob = await cardBlob(canvas);
      const path = await save({
        defaultPath: `speakoflow-insights-${prefs.format}.png`,
        filters: [{ name: "PNG", extensions: ["png"] }],
      });
      if (!path) return;
      const result = await commands.saveInsightsCard(
        path,
        await blobToBase64(blob),
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

  const copyImage = async () => {
    const canvas = canvasRef.current;
    if (!canvas || busy) return;
    setBusy("copy");
    try {
      // The webview's clipboard first, with the click's user activation still
      // live: the blob is handed over as a promise rather than awaited first.
      if (typeof ClipboardItem !== "undefined" && navigator.clipboard?.write) {
        try {
          await navigator.clipboard.write([
            new ClipboardItem({ "image/png": cardBlob(canvas) }),
          ]);
          setCopied(true);
          return;
        } catch {
          // Refused: the app's own clipboard below.
        }
      }
      const result = await commands.copyInsightsCard(
        await blobToBase64(await cardBlob(canvas)),
      );
      if (result.status === "ok") {
        setCopied(true);
      } else {
        toast.error(t("insights.share.copyFailed"), {
          description: result.error,
        });
      }
    } catch (error) {
      toast.error(t("insights.share.copyFailed"), {
        description: String(error),
      });
    } finally {
      setBusy(null);
    }
  };

  const { width, height } = CARD_SIZES[prefs.format];

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={t("insights.share.title")}
      description={t("insights.share.description")}
      size="xl"
      footer={
        <>
          <p className="me-auto hidden text-xs text-muted sm:block">
            {t("insights.share.private")}
          </p>
          <Button
            variant="secondary"
            onClick={() => void copyImage()}
            disabled={!ready || failed || busy !== null}
            aria-live="polite"
          >
            {copied ? (
              <Check className="h-4 w-4 text-accent" aria-hidden="true" />
            ) : (
              <Copy className="h-4 w-4" aria-hidden="true" />
            )}
            {copied ? t("insights.share.copied") : t("insights.share.copy")}
          </Button>
          <Button
            onClick={() => void saveImage()}
            disabled={!ready || failed || busy !== null}
            data-autofocus
          >
            <Download className="h-4 w-4" aria-hidden="true" />
            {t("insights.share.save")}
          </Button>
        </>
      }
    >
      <div className="grid gap-6 md:grid-cols-[minmax(0,1fr)_18rem]">
        <div className="flex h-[min(56vh,30rem)] min-w-0 items-center justify-center overflow-hidden rounded-2xl border border-hairline bg-surface-muted p-5">
          {failed ? (
            <p className="max-w-xs text-center text-sm text-muted">
              {t("insights.share.drawFailed")}
            </p>
          ) : (
            <canvas
              ref={canvasRef}
              width={width}
              height={height}
              role="img"
              aria-label={content?.label ?? ""}
              className={`share-card max-h-full min-h-0 max-w-full min-w-0 rounded-[0.875rem] shadow-[0_24px_48px_-22px_rgba(10,20,24,0.45),0_2px_6px_rgba(10,20,24,0.08)] transition-opacity duration-200 ${
                ready ? "opacity-100" : "opacity-0"
              }`}
              style={{ aspectRatio: `${width} / ${height}` }}
            />
          )}
        </div>

        <div className="flex flex-col gap-6">
          <div>
            <ControlLabel>{t("insights.share.size")}</ControlLabel>
            <Segmented<CardFormat>
              label={t("insights.share.size")}
              value={prefs.format}
              onChange={(format) => update({ format })}
              fill
              options={CARD_FORMATS.map((format) => ({
                value: format,
                label: t(`insights.share.formats.${format}`),
                icon: FORMAT_ICONS[format],
                title: t(`insights.share.formatHints.${format}`),
              }))}
            />
            <p className="mt-2 text-xs text-muted">
              {t(`insights.share.formatHints.${prefs.format}`)}
            </p>
          </div>

          <div>
            <ControlLabel>{t("insights.share.look")}</ControlLabel>
            <LookPicker
              value={prefs.look}
              onChange={(look) => update({ look })}
            />
          </div>

          {(canInclude.assistant || canInclude.meetings) && (
            <div>
              <ControlLabel>{t("insights.share.show")}</ControlLabel>
              <div className="divide-y divide-hairline rounded-xl border border-hairline">
                {(["assistant", "meetings"] as const)
                  .filter((key) => canInclude[key])
                  .map((key) => (
                    <div
                      key={key}
                      className="flex items-center justify-between gap-3 px-3 py-2.5"
                    >
                      <span className="text-sm text-ink">
                        {t(`insights.activity.${key}`)}
                      </span>
                      <Switch
                        checked={include[key]}
                        onChange={(checked) =>
                          setInclude((current) => ({
                            ...current,
                            [key]: checked,
                          }))
                        }
                        label={t(`insights.share.include.${key}`)}
                      />
                    </div>
                  ))}
              </div>
            </div>
          )}
        </div>
      </div>
    </Dialog>
  );
};
