import type { AskAnchor, DisplayChoice } from "@/bindings";

/**
 * Where the floating panel opens and how big it is, mirroring the backend
 * (`ask_preset_scale`, `ask_shape_for_anchor`, `ask_size_for_display` and
 * `anchor_position` in src-tauri/src/assistant.rs), so the settings preview
 * draws the card where the real one opens and at the size it opens at.
 *
 * Every size here is in logical pixels of a display. Pure, so the mirror can
 * be checked without a monitor.
 */

export type PanelAnchor = Exclude<AskAnchor, "custom">;

/**
 * The "Which screen" choices: the screen the mouse is on (the default), then each
 * connected display by number and resolution. A raw device name ("\\.\DISPLAY2")
 * means nothing to anyone; the resolution is what people recognise their screens
 * by.
 */
export const askDisplayOptions = (
  displays: DisplayChoice[],
  t: (key: string, options?: Record<string, unknown>) => string,
): { value: string; label: string }[] => [
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
];

/**
 * The stored display choice as the dropdown shows it. `last_used` was the old
 * default and now follows the cursor, as the backend treats it.
 */
export const askDisplayValue = (stored: string | undefined): string =>
  !stored || stored === "last_used" ? "cursor" : stored;

const ASK_MIN_WIDTH = 380;
const ASK_MAX_WIDTH = 760;
const ASK_MIN_HEIGHT = 340;
const ASK_MAX_HEIGHT = 720;
/** The smallest frame a display is ever asked to hold (ASK_FRAME_FLOOR_*). */
const ASK_FRAME_FLOOR_WIDTH = 320;
const ASK_FRAME_FLOOR_HEIGHT = 160;
/** Gap between the card and the edge of the display. */
export const PANEL_MARGIN = 24;
/** Room left below the card for a taskbar or dock. */
export const TASKBAR_CLEARANCE = 40;
/** The centre sits a little above true centre: the card grows downward. */
const CENTRE_LIFT = 24;

const clamp = (value: number, low: number, high: number) =>
  Math.min(Math.max(value, low), high);

/** The size preset as a multiplier on the display-derived size. */
export const presetScale = (size: string): number => {
  switch (size) {
    case "mini":
      return 0.78;
    case "compact":
      return 0.88;
    case "large":
      return 1.22;
    default:
      return 1;
  }
};

/** The card's proportions for the edge it opens at, as fractions of the display. */
export const shapeForAnchor = (
  anchor: AskAnchor,
): { width: number; height: number } => {
  switch (anchor) {
    // A tall column down one side.
    case "left":
    case "right":
      return { width: 0.25, height: 0.7 };
    // A wide strip along one edge.
    case "topcenter":
    case "bottomcenter":
      return { width: 0.42, height: 0.34 };
    // Balanced, and the default.
    default:
      return { width: 0.3, height: 0.46 };
  }
};

/**
 * The card's width, and the height it may grow to, on a display. The card is
 * sized to its answer, so the height is a ceiling rather than a size.
 */
export const askSizeForDisplay = (
  displayWidth: number,
  displayHeight: number,
  size: string,
  anchor: AskAnchor,
): { width: number; maxHeight: number } => {
  const scale = presetScale(size);
  const shape = shapeForAnchor(anchor);
  const width = clamp(
    displayWidth * shape.width * scale,
    ASK_MIN_WIDTH,
    ASK_MAX_WIDTH * scale,
  );
  const maxHeight = clamp(
    displayHeight * shape.height * scale,
    ASK_MIN_HEIGHT,
    ASK_MAX_HEIGHT * scale,
  );
  // A small screen wins over the minimum: a frame larger than the display would
  // put part of the card off screen.
  const fitWidth = Math.max(
    displayWidth - 2 * PANEL_MARGIN,
    ASK_FRAME_FLOOR_WIDTH,
  );
  const fitHeight = Math.max(
    displayHeight - 2 * PANEL_MARGIN - TASKBAR_CLEARANCE,
    ASK_FRAME_FLOOR_HEIGHT,
  );
  return {
    width: Math.min(width, fitWidth),
    maxHeight: Math.min(maxHeight, fitHeight),
  };
};

/** The top-left corner of a card of the given size at an anchor. */
export const anchorPosition = (
  anchor: AskAnchor,
  displayWidth: number,
  displayHeight: number,
  width: number,
  height: number,
): { x: number; y: number } => {
  const centreX = (displayWidth - width) / 2;
  const centreY = (displayHeight - height) / 2 - CENTRE_LIFT;
  const right = displayWidth - width - PANEL_MARGIN;
  const bottom = displayHeight - height - PANEL_MARGIN - TASKBAR_CLEARANCE;
  const [x, y] =
    anchor === "topcenter"
      ? [centreX, PANEL_MARGIN]
      : anchor === "bottomcenter"
        ? [centreX, bottom]
        : anchor === "left"
          ? [PANEL_MARGIN, centreY]
          : anchor === "right"
            ? [right, centreY]
            : [centreX, centreY];
  // Never let an anchor push the card off its own display.
  return {
    x: clamp(
      x,
      PANEL_MARGIN,
      Math.max(displayWidth - width - PANEL_MARGIN, PANEL_MARGIN),
    ),
    y: clamp(
      y,
      PANEL_MARGIN,
      Math.max(displayHeight - height - PANEL_MARGIN, PANEL_MARGIN),
    ),
  };
};
