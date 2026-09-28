import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type AskAnchor, type DisplayChoice } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useSettingCommand } from "@/hooks/useSettingCommand";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { Segmented } from "@/components/ui/Segmented";
import { Dropdown } from "@/components/ui/Dropdown";
import { InfoTip } from "@/components/ui/InfoTip";
import { PanelPreview } from "@/components/settings/assistant/AssistantSettings";
import {
  askDisplayOptions,
  askDisplayValue,
  type PanelAnchor,
} from "./panelGeometry";

/**
 * "Floating panel", in two halves.
 *
 * How it looks: the real panel (drawn with its own stylesheet) beside the three
 * knobs that change it — text size, panel size, opacity — so every change is
 * visible the moment it is made. Where it opens: a map of a screen whose spots
 * are the places it can open — the same size as the preview — beside the
 * position and screen pickers, so both halves share one pair of columns.
 *
 * The previous version drew the panel full-size on a full-width picture of a
 * 1920×1080 desktop and then listed the knobs underneath as rows, so the one
 * setting took a whole screen of scrolling and the preview sat far from the
 * controls that changed it. Every explanation is behind an (i).
 */

/** The spots on the map, in percent of the pictured screen. The shapes are the
 *  shapes the panel opens in: a column down a side, a strip along an edge. */
const ZONES: Array<{
  value: PanelAnchor;
  key: string;
  box: { left: number; top: number; width: number; height: number };
}> = [
  {
    value: "left",
    key: "left",
    box: { left: 5, top: 14, width: 21, height: 72 },
  },
  {
    value: "topcenter",
    key: "top",
    box: { left: 32, top: 12, width: 36, height: 18 },
  },
  {
    value: "center",
    key: "center",
    box: { left: 37, top: 38, width: 26, height: 26 },
  },
  {
    value: "bottomcenter",
    key: "bottom",
    box: { left: 32, top: 72, width: 36, height: 18 },
  },
  {
    value: "right",
    key: "right",
    box: { left: 74, top: 14, width: 21, height: 72 },
  },
];

/** "Where it opens" as a picture of a screen: click the spot. */
const PositionMap: React.FC<{
  value: PanelAnchor;
  onChange: (anchor: PanelAnchor) => void;
  className?: string;
}> = ({ value, onChange, className = "" }) => {
  const { t } = useTranslation();
  return (
    <div
      role="radiogroup"
      aria-label={t("assistantPage.panel.where")}
      className={`relative aspect-video overflow-hidden rounded-xl border border-hairline-strong bg-surface-muted ${className}`}
    >
      {/* The top edge of a window, so it reads as a screen, not a grid. */}
      <span
        aria-hidden="true"
        className="absolute inset-x-0 top-0 h-[6%] bg-ink/[0.06]"
      />
      {ZONES.map((zone) => {
        const active = zone.value === value;
        const label = t(`settings.assistant.appearance.askAnchors.${zone.key}`);
        return (
          <button
            key={zone.value}
            type="button"
            role="radio"
            aria-checked={active}
            aria-label={label}
            title={label}
            onClick={() => {
              if (!active) onChange(zone.value);
            }}
            style={{
              left: `${zone.box.left}%`,
              top: `${zone.box.top}%`,
              width: `${zone.box.width}%`,
              height: `${zone.box.height}%`,
            }}
            className={`absolute cursor-pointer rounded-[5px] border transition-[background-color,border-color,box-shadow] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/50 ${
              active
                ? "border-accent-fill bg-accent-fill shadow-[0_4px_12px_-4px_rgb(0_150_132/0.55)]"
                : "border-dashed border-ink/20 bg-surface/50 hover:border-accent/60 hover:bg-accent/10"
            }`}
          />
        );
      })}
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

/** A label (with its (i)) above a control, and an optional value on the right
 *  of the label line. Same type as a settings-row title, so the two halves of
 *  the card read as one. */
const Field: React.FC<{
  label: string;
  info?: string;
  value?: React.ReactNode;
  children: React.ReactNode;
}> = ({ label, info, value, children }) => (
  <div className="min-w-0">
    <div className="mb-2 flex min-h-5 items-center justify-between gap-3">
      <span className="flex items-center gap-1 text-sm font-medium text-ink">
        {label}
        {info && <InfoTip text={info} />}
      </span>
      {value && (
        <span className="text-xs font-medium text-muted tabular-nums">
          {value}
        </span>
      )}
    </div>
    {children}
  </div>
);

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

  const displayOptions = useMemo(
    () => askDisplayOptions(displays, t),
    [displays, t],
  );

  const commitOpacity = () => {
    if (opacity !== stored) {
      void run(commands.setAssistantPanelOpacity(opacity));
    }
  };

  // The map's spots as a list too: the map is the quick way, this is the one
  // that names every place and works from the keyboard.
  const anchorOptions = ZONES.map((zone) => ({
    value: zone.value,
    label: t(`settings.assistant.appearance.askAnchors.${zone.key}`),
  }));

  const fill = ((opacity - 0.5) / 0.5) * 100;

  return (
    <SettingsGroup title={t("assistantPage.cards.panel.title")}>
      {/* How it looks. */}
      <div className="grid items-center gap-6 p-5 @2xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
        <PanelPreview fontSize={fontSize} opacity={opacity} />
        <div className="flex min-w-0 flex-col gap-5">
          <Field label={t("assistantPage.panel.text")}>
            <Segmented
              size="sm"
              fill
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
          </Field>
          <Field
            label={t("assistantPage.panel.size")}
            info={t("assistantPage.tips.panelSize")}
          >
            <Segmented
              size="sm"
              fill
              label={t("assistantPage.panel.size")}
              value={panelSize as (typeof PANEL_SIZES)[number]}
              onChange={(next) =>
                void run(commands.setAssistantPanelSize(next))
              }
              options={PANEL_SIZES.map((id) => ({
                value: id,
                label: t(`settings.assistant.appearance.panelSizes.${id}`),
              }))}
            />
          </Field>
          <Field
            label={t("assistantPage.panel.opacity")}
            info={t("assistantPage.tips.opacity")}
            value={`${Math.round(opacity * 100)}%`}
          >
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
              className="h-1.5 w-full cursor-pointer appearance-none rounded-full focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
              style={{
                background: `linear-gradient(to right, var(--color-accent-fill) ${fill}%, var(--color-hairline-strong) ${fill}%)`,
              }}
            />
          </Field>
        </div>
      </div>

      {/* Where it opens, in the same two columns as the half above: the map is
          the same box as the preview (16:9 is the preview's own proportion at
          the default text size), and its controls line up under the knobs.
          A map sized on its own sat small in a column twice its width, and a
          lone dropdown beside it left the rest of the row empty. */}
      <div className="grid items-center gap-6 p-5 @2xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
        <PositionMap
          value={anchor}
          onChange={(next) => void run(commands.setAssistantAskAnchor(next))}
          className="w-full max-w-[26rem] @2xl:max-w-none"
        />
        <div className="flex min-w-0 flex-col gap-5">
          <Field
            label={t("assistantPage.panel.where")}
            info={t("assistantPage.tips.position")}
          >
            <Dropdown
              options={anchorOptions}
              selectedValue={anchor}
              onSelect={(next) =>
                void run(commands.setAssistantAskAnchor(next as PanelAnchor))
              }
              className="w-full"
            />
          </Field>
          {displays.length > 1 && (
            <Field
              label={t("assistantPage.panel.screen")}
              info={t("assistantPage.tips.display")}
            >
              <Dropdown
                options={displayOptions}
                selectedValue={askDisplayValue(settings?.assistant_ask_display)}
                onSelect={(display) =>
                  void run(commands.setAssistantAskDisplay(display))
                }
                className="w-full"
              />
            </Field>
          )}
        </div>
      </div>
    </SettingsGroup>
  );
};
