import React, {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { ArrowUp, Copy, CornerDownLeft, Mic } from "lucide-react";
import { commands, type AskAnchor, type DisplayChoice } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useSettingCommand } from "@/hooks/useSettingCommand";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { Segmented } from "@/components/ui/Segmented";
import { Dropdown } from "@/components/ui/Dropdown";
import { FONT_SIZES } from "@/assistant/appearance";
import "@/assistant/AssistantPanel.css";
import {
  anchorPosition,
  askSizeForDisplay,
  type PanelAnchor,
} from "./panelGeometry";

/**
 * The floating panel's settings, around a picture of your screen.
 *
 * The preview is a miniature desktop with the real panel card drawn on it —
 * where it opens and as big as it opens, from the same geometry the backend
 * uses (see `panelGeometry.ts`) — and it is also the control: the dashed
 * outlines are the other places it can open, and clicking one moves it there.
 * Text size, size and opacity redraw the card as you change them.
 */

/** The preview pictures a typical 1920×1080 screen… */
const DISPLAY_W = 1920;
const DISPLAY_H = 1080;
/** …drawn this many CSS px wide, then scaled to fit the page. Text is not
 *  shrunk with the screen, so the card stays readable. */
const VIRTUAL_W = 1100;
const F = VIRTUAL_W / DISPLAY_W;
const VIRTUAL_H = DISPLAY_H * F;
/** Taskbar height on the pictured screen, in display px. */
const TASKBAR = 40;

/** Center last, so it wins wherever two outlines touch. */
const ANCHORS: Array<{ value: PanelAnchor; key: string }> = [
  { value: "left", key: "left" },
  { value: "right", key: "right" },
  { value: "topcenter", key: "top" },
  { value: "bottomcenter", key: "bottom" },
  { value: "center", key: "center" },
];

interface Frame {
  left: number;
  top: number;
  width: number;
  height: number;
  maxHeight: number;
}

/** Where a card of the given height sits at an anchor, in preview px. */
const frameFor = (
  anchor: PanelAnchor,
  size: string,
  cardHeight: number,
): Frame => {
  const { width, maxHeight } = askSizeForDisplay(
    DISPLAY_W,
    DISPLAY_H,
    size,
    anchor,
  );
  const height = Math.min(cardHeight / F, maxHeight);
  const { x, y } = anchorPosition(anchor, DISPLAY_W, DISPLAY_H, width, height);
  return {
    left: x * F,
    top: y * F,
    width: width * F,
    height: height * F,
    maxHeight: maxHeight * F,
  };
};

/** The answer card, drawn with the panel's own stylesheet. */
const PreviewCard: React.FC = () => {
  const { t } = useTranslation();
  return (
    <div className="assistant-preview-surface shadow-[0_18px_40px_-18px_rgba(0,0,0,0.7)]">
      <div className="ask-card">
        <div className="ask-head">
          <Mic className="ask-head-icon" size={12} />
          <p className="ask-question-text">
            {t("settings.assistant.appearance.previewUser")}
          </p>
          <div className="ask-head-actions">
            <span className="ask-action">
              <Copy size={13} />
            </span>
            <span className="ask-action labelled">
              <CornerDownLeft size={13} />
              <span>{t("assistant.insertShort")}</span>
            </span>
          </div>
        </div>
        <div className="ask-answer-body">
          {t("settings.assistant.appearance.previewAssistant")}
        </div>
        <div className="assistant-input-row">
          <div
            className="assistant-input"
            style={{
              display: "flex",
              alignItems: "center",
              color: "var(--as-faint)",
            }}
          >
            {t("assistant.followUpPlaceholder")}
          </div>
          <span className="assistant-send-button">
            <ArrowUp size={15} strokeWidth={2.5} />
          </span>
        </div>
      </div>
    </div>
  );
};

/** The miniature desktop: the card where it opens, and every other spot. */
const PanelDesktop: React.FC<{
  anchor: PanelAnchor;
  size: string;
  fontSize: string;
  opacity: number;
  onAnchor: (anchor: PanelAnchor) => void;
}> = ({ anchor, size, fontSize, opacity, onAnchor }) => {
  const { t } = useTranslation();
  const screenRef = useRef<HTMLDivElement>(null);
  const cardRef = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState(0);
  const [cardHeight, setCardHeight] = useState(170);

  // Fit the pictured screen to the width it is given.
  useLayoutEffect(() => {
    const screen = screenRef.current;
    if (!screen) return;
    const measure = () => setScale(screen.clientWidth / VIRTUAL_W);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(screen);
    return () => observer.disconnect();
  }, []);

  // The card is sized to its answer, so where it sits depends on how tall the
  // text makes it at the chosen text size and width.
  useLayoutEffect(() => {
    const card = cardRef.current;
    if (!card) return;
    const measure = () => setCardHeight(card.offsetHeight);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(card);
    return () => observer.disconnect();
  }, []);

  const card = frameFor(anchor, size, cardHeight);

  return (
    <div
      ref={screenRef}
      className="assistant-desktop relative w-full overflow-hidden rounded-xl"
      style={{ aspectRatio: `${DISPLAY_W} / ${DISPLAY_H}` }}
    >
      <div
        role="radiogroup"
        aria-label={t("assistantPage.panel.where")}
        className="assistant-scope absolute left-0 top-0 origin-top-left"
        style={
          {
            width: VIRTUAL_W,
            height: VIRTUAL_H,
            transform: `scale(${scale})`,
            visibility: scale > 0 ? "visible" : "hidden",
            "--as-msg-font": FONT_SIZES[fontSize] ?? FONT_SIZES.medium,
            "--as-alpha": String(Math.max(opacity, 0.5)),
          } as React.CSSProperties
        }
      >
        <div
          aria-hidden="true"
          className="absolute inset-x-0 bottom-0 border-t border-white/[0.07] bg-black/30"
          style={{ height: TASKBAR * F }}
        />

        {ANCHORS.map(({ value, key }) => {
          const frame = frameFor(value, size, cardHeight);
          const active = value === anchor;
          const label = t(`settings.assistant.appearance.askAnchors.${key}`);
          return (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={active}
              aria-label={label}
              title={active ? undefined : label}
              onClick={() => {
                if (!active) onAnchor(value);
              }}
              style={{
                left: frame.left,
                top: frame.top,
                width: frame.width,
                height: frame.height,
              }}
              className={`group absolute grid place-items-center rounded-[18px] border-2 border-dashed transition-[left,top,width,height,background-color,border-color] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-white/50 motion-reduce:transition-none ${
                active
                  ? "cursor-default border-transparent"
                  : "cursor-pointer border-white/[0.14] bg-white/[0.02] hover:border-teal-300/70 hover:bg-teal-300/[0.08]"
              }`}
            >
              {!active && (
                <span className="text-[17px] font-medium text-white/0 transition-colors duration-200 group-hover:text-white/85 group-focus-visible:text-white/85">
                  {label}
                </span>
              )}
            </button>
          );
        })}

        <div
          ref={cardRef}
          aria-hidden="true"
          className="pointer-events-none absolute overflow-hidden transition-[left,top,width] duration-[420ms] ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none"
          style={{
            left: card.left,
            top: card.top,
            width: card.width,
            maxHeight: card.maxHeight,
          }}
        >
          <PreviewCard />
        </div>
      </div>
    </div>
  );
};

const FONT_SIZE_IDS = ["small", "medium", "large", "extra_large"] as const;
const FONT_KEYS: Record<(typeof FONT_SIZE_IDS)[number], string> = {
  small: "small",
  medium: "medium",
  large: "large",
  extra_large: "extraLarge",
};
const PANEL_SIZES = ["mini", "compact", "standard", "large"] as const;

const ANCHOR_KEYS: Record<PanelAnchor, string> = {
  left: "left",
  right: "right",
  topcenter: "top",
  bottomcenter: "bottom",
  center: "center",
};

/** "Floating panel": the picture of your screen, then one row per knob. */
export const PanelCard: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const run = useSettingCommand();
  const [displays, setDisplays] = useState<DisplayChoice[]>([]);
  const fontSize = settings?.assistant_font_size ?? "medium";
  const panelSize = settings?.assistant_panel_size ?? "standard";
  const stored = settings?.assistant_panel_opacity ?? 1;
  const [opacity, setOpacity] = useState(stored);
  const storedAnchor: AskAnchor = settings?.assistant_ask_anchor ?? "center";
  // "Where I left it" is no longer offered; it reads as the middle, as the
  // backend treats it.
  const anchor: PanelAnchor =
    storedAnchor === "custom" ? "center" : storedAnchor;

  useEffect(() => setOpacity(stored), [stored]);

  useEffect(() => {
    let active = true;
    const load = () => {
      void commands.listAssistantDisplays().then((result) => {
        if (active && result.status === "ok") setDisplays(result.data);
      });
    };
    load();
    window.addEventListener("focus", load);
    return () => {
      active = false;
      window.removeEventListener("focus", load);
    };
  }, []);

  const currentDisplay =
    displays.findIndex((display) => display.is_current) + 1;
  const displayOptions = useMemo(
    () => [
      {
        value: "last_used",
        label: currentDisplay
          ? t("settings.assistant.appearance.askDisplays.lastUsedOn", {
              number: currentDisplay,
            })
          : t("settings.assistant.appearance.askDisplays.lastUsed"),
      },
      {
        value: "cursor",
        label: t("settings.assistant.appearance.askDisplays.cursor"),
      },
      ...displays.map((display, index) => ({
        value: display.id,
        label: t("settings.assistant.appearance.askDisplays.numbered", {
          number: index + 1,
          width: display.width,
          height: display.height,
          suffix: display.is_primary
            ? t("settings.assistant.appearance.askDisplays.mainSuffix")
            : "",
        }),
      })),
    ],
    [displays, currentDisplay, t],
  );

  const commitOpacity = () => {
    if (opacity !== stored) {
      void run(commands.setAssistantPanelOpacity(opacity));
    }
  };

  const fill = ((opacity - 0.5) / 0.5) * 100;

  return (
    <SettingsGroup title={t("assistantPage.cards.panel.title")}>
      <SettingContainer
        title={t("assistantPage.panel.where")}
        description={t("assistantPage.tips.position")}
        grouped
        details={
          <PanelDesktop
            anchor={anchor}
            size={panelSize}
            fontSize={fontSize}
            opacity={opacity}
            onAnchor={(next) => void run(commands.setAssistantAskAnchor(next))}
          />
        }
      >
        <span className="text-[0.8125rem] text-muted">
          {t(`settings.assistant.appearance.askAnchors.${ANCHOR_KEYS[anchor]}`)}
        </span>
      </SettingContainer>

      <SettingContainer title={t("assistantPage.panel.text")} grouped>
        <Segmented
          size="sm"
          label={t("assistantPage.panel.text")}
          value={fontSize as (typeof FONT_SIZE_IDS)[number]}
          onChange={(next) => void run(commands.setAssistantFontSize(next))}
          options={FONT_SIZE_IDS.map((id, index) => ({
            value: id,
            ariaLabel: t(
              `settings.assistant.appearance.fontSizes.${FONT_KEYS[id]}`,
            ),
            title: t(
              `settings.assistant.appearance.fontSizes.${FONT_KEYS[id]}`,
            ),
            label: (
              <span
                aria-hidden="true"
                className="inline-block w-4 text-center font-semibold leading-none"
                style={{ fontSize: `${0.625 + index * 0.15625}rem` }}
              >
                A
              </span>
            ),
          }))}
        />
      </SettingContainer>

      <SettingContainer
        title={t("assistantPage.panel.size")}
        description={t("assistantPage.tips.panelSize")}
        grouped
      >
        <Segmented
          size="sm"
          label={t("assistantPage.panel.size")}
          value={panelSize as (typeof PANEL_SIZES)[number]}
          onChange={(next) => void run(commands.setAssistantPanelSize(next))}
          options={PANEL_SIZES.map((id) => ({
            value: id,
            label: t(`settings.assistant.appearance.panelSizes.${id}`),
          }))}
        />
      </SettingContainer>

      <SettingContainer
        title={t("assistantPage.panel.opacity")}
        description={t("assistantPage.tips.opacity")}
        grouped
      >
        <div className="flex w-[12rem] items-center gap-2.5">
          <input
            type="range"
            min={0.5}
            max={1}
            step={0.05}
            value={opacity}
            aria-label={t("assistantPage.panel.opacity")}
            onChange={(event) => setOpacity(parseFloat(event.target.value))}
            onPointerUp={commitOpacity}
            onKeyUp={commitOpacity}
            className="h-1.5 min-w-0 flex-1 cursor-pointer appearance-none rounded-full focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
            style={{
              background: `linear-gradient(to right, var(--color-accent) ${fill}%, var(--color-hairline-strong) ${fill}%)`,
            }}
          />
          <span className="w-9 text-end text-xs font-medium text-muted tabular-nums">
            {Math.round(opacity * 100)}%
          </span>
        </div>
      </SettingContainer>

      {displays.length > 1 && (
        <SettingContainer
          title={t("assistantPage.panel.screen")}
          description={t("assistantPage.tips.display")}
          grouped
        >
          <Dropdown
            options={displayOptions}
            selectedValue={settings?.assistant_ask_display ?? "last_used"}
            onSelect={(display) =>
              void run(commands.setAssistantAskDisplay(display))
            }
            className="w-[15rem]"
          />
        </SettingContainer>
      )}
    </SettingsGroup>
  );
};
