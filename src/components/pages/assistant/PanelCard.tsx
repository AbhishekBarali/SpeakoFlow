import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type AskAnchor, type DisplayChoice } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useSettingCommand } from "@/hooks/useSettingCommand";
import { Segmented } from "@/components/ui/Segmented";
import { Dropdown } from "@/components/ui/Dropdown";
import { InfoTip } from "@/components/ui/InfoTip";
import { PanelPreview } from "@/components/settings/assistant/AssistantSettings";

/** Where each anchor puts the panel on the miniature screen, in percent. */
const ZONES: Array<{
  value: Exclude<AskAnchor, "custom">;
  key: string;
  box: { left: number; top: number; width: number; height: number };
}> = [
  {
    value: "left",
    key: "left",
    box: { left: 5, top: 12, width: 22, height: 76 },
  },
  {
    value: "topcenter",
    key: "top",
    box: { left: 33, top: 9, width: 34, height: 18 },
  },
  {
    value: "center",
    key: "center",
    box: { left: 37, top: 36, width: 26, height: 28 },
  },
  {
    value: "bottomcenter",
    key: "bottom",
    box: { left: 33, top: 73, width: 34, height: 18 },
  },
  {
    value: "right",
    key: "right",
    box: { left: 73, top: 12, width: 22, height: 76 },
  },
];

/**
 * "Where it opens" as a picture of a screen: click the spot. Each spot is the
 * shape the panel takes there — a column down a side, a strip along an edge.
 */
const PositionPicker: React.FC<{
  value: AskAnchor;
  onChange: (anchor: AskAnchor) => void;
}> = ({ value, onChange }) => {
  const { t } = useTranslation();
  // "Where I left it" is no longer offered; a stored value from before reads
  // as the middle, as the backend treats it.
  const current = value === "custom" ? "center" : value;
  const selected = ZONES.find((zone) => zone.value === current) ?? ZONES[2];

  return (
    <div
      role="radiogroup"
      aria-label={t("settings.assistant.appearance.askAnchorLabel")}
      className="relative aspect-[16/10] w-[9.5rem] shrink-0 overflow-hidden rounded-lg border border-hairline-strong bg-surface-muted"
    >
      <span
        aria-hidden="true"
        className="absolute inset-x-0 top-0 h-[6%] bg-ink/[0.06]"
      />
      {ZONES.map((zone) => {
        const active = zone.value === selected.value;
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
            className={`absolute cursor-pointer rounded-[3px] border transition-[background-color,border-color] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 ${
              active
                ? "border-accent bg-accent/80"
                : "border-dashed border-ink/20 bg-surface/60 hover:border-accent/60 hover:bg-accent/10"
            }`}
          />
        );
      })}
    </div>
  );
};

const FONT_SIZES = ["small", "medium", "large", "extra_large"] as const;
const FONT_KEYS: Record<(typeof FONT_SIZES)[number], string> = {
  small: "small",
  medium: "medium",
  large: "large",
  extra_large: "extraLarge",
};
const PANEL_SIZES = ["mini", "compact", "standard", "large"] as const;

/** A label (with its short (i)) and its control on one line. */
const Line: React.FC<{
  label: string;
  info?: string;
  children: React.ReactNode;
}> = ({ label, info, children }) => (
  <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
    <span className="flex items-center gap-1 text-[0.8125rem] font-medium text-ink">
      {label}
      {info && <InfoTip text={info} />}
    </span>
    {children}
  </div>
);

/**
 * How the floating panel looks and where it appears, beside a preview that
 * redraws as you change it. Every explanation is behind its (i).
 */
export const PanelCard: React.FC = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const run = useSettingCommand();
  const [displays, setDisplays] = useState<DisplayChoice[]>([]);
  const fontSize = settings?.assistant_font_size ?? "medium";
  const panelSize = settings?.assistant_panel_size ?? "standard";
  const stored = settings?.assistant_panel_opacity ?? 1;
  const [opacity, setOpacity] = useState(stored);

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

  return (
    <div className="grid gap-6 rounded-2xl border border-hairline bg-surface p-5 elev-card @3xl:grid-cols-[minmax(0,1fr)_minmax(0,1.05fr)]">
      <PanelPreview fontSize={fontSize} opacity={opacity} />
      <div className="flex min-w-0 flex-col gap-4">
        <Line label={t("settings.assistant.appearance.fontSizeLabel")}>
          <Segmented
            size="sm"
            label={t("settings.assistant.appearance.fontSizeLabel")}
            value={fontSize as (typeof FONT_SIZES)[number]}
            onChange={(size) => void run(commands.setAssistantFontSize(size))}
            options={FONT_SIZES.map((size, index) => ({
              value: size,
              ariaLabel: t(
                `settings.assistant.appearance.fontSizes.${FONT_KEYS[size]}`,
              ),
              title: t(
                `settings.assistant.appearance.fontSizes.${FONT_KEYS[size]}`,
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
        </Line>
        <Line
          label={t("settings.assistant.appearance.panelSizeLabel")}
          info={t("assistantPage.tips.panelSize")}
        >
          <Segmented
            size="sm"
            label={t("settings.assistant.appearance.panelSizeLabel")}
            value={panelSize as (typeof PANEL_SIZES)[number]}
            onChange={(size) => void run(commands.setAssistantPanelSize(size))}
            options={PANEL_SIZES.map((size) => ({
              value: size,
              label: t(`settings.assistant.appearance.panelSizes.${size}`),
            }))}
          />
        </Line>
        <Line
          label={t("settings.assistant.appearance.opacityLabel")}
          info={t("assistantPage.tips.opacity")}
        >
          <div className="flex w-[11rem] items-center gap-2.5">
            <input
              type="range"
              min={0.5}
              max={1}
              step={0.05}
              value={opacity}
              aria-label={t("settings.assistant.appearance.opacityLabel")}
              onChange={(event) => setOpacity(parseFloat(event.target.value))}
              onPointerUp={commitOpacity}
              onKeyUp={commitOpacity}
              className="h-1.5 min-w-0 flex-1 cursor-pointer appearance-none rounded-full focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
              style={{
                background: `linear-gradient(to right, var(--color-accent) ${
                  ((opacity - 0.5) / 0.5) * 100
                }%, var(--color-hairline-strong) ${((opacity - 0.5) / 0.5) * 100}%)`,
              }}
            />
            <span className="w-9 text-end text-xs font-medium text-muted tabular-nums">
              {Math.round(opacity * 100)}%
            </span>
          </div>
        </Line>
        <Line
          label={t("settings.assistant.appearance.askAnchorLabel")}
          info={t("assistantPage.tips.position")}
        >
          <PositionPicker
            value={settings?.assistant_ask_anchor ?? "center"}
            onChange={(anchor) =>
              void run(commands.setAssistantAskAnchor(anchor))
            }
          />
        </Line>
        {displays.length > 1 && (
          <Line
            label={t("settings.assistant.appearance.askDisplayLabel")}
            info={t("assistantPage.tips.display")}
          >
            <Dropdown
              options={displayOptions}
              selectedValue={settings?.assistant_ask_display ?? "last_used"}
              onSelect={(display) =>
                void run(commands.setAssistantAskDisplay(display))
              }
              className="w-[14rem]"
            />
          </Line>
        )}
      </div>
    </div>
  );
};
