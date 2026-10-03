import { describe, expect, test } from "bun:test";
import {
  PANEL_MARGIN,
  TASKBAR_CLEARANCE,
  anchorPosition,
  askSizeForDisplay,
  presetScale,
} from "./panelGeometry";

describe("askSizeForDisplay", () => {
  test("each size preset is bigger than the one before it", () => {
    const widths = ["mini", "compact", "standard", "large"].map(
      (size) => askSizeForDisplay(1920, 1080, size, "center").width,
    );
    expect(widths).toEqual([...widths].sort((a, b) => a - b));
    expect(new Set(widths).size).toBe(4);
    expect(widths[2]).toBeCloseTo(576);
  });

  test("a side column is narrower than a strip along the top", () => {
    const side = askSizeForDisplay(1920, 1080, "standard", "left");
    const strip = askSizeForDisplay(1920, 1080, "standard", "topcenter");
    expect(side.width).toBeCloseTo(480);
    expect(strip.width).toBe(760);
    expect(side.maxHeight).toBeGreaterThan(strip.maxHeight);
  });

  test("the card never shrinks below its minimum width", () => {
    expect(askSizeForDisplay(1280, 720, "mini", "center").width).toBe(380);
  });

  test("a small screen wins over the minimum", () => {
    const size = askSizeForDisplay(400, 300, "standard", "center");
    expect(size.width).toBe(400 - 2 * PANEL_MARGIN);
    expect(size.maxHeight).toBe(300 - 2 * PANEL_MARGIN - TASKBAR_CLEARANCE);
  });

  test("unknown presets read as standard", () => {
    expect(presetScale("huge")).toBe(1);
    expect(askSizeForDisplay(1920, 1080, "huge", "center").width).toBeCloseTo(
      576,
    );
  });
});

describe("anchorPosition", () => {
  const at = (anchor: Parameters<typeof anchorPosition>[0]) =>
    anchorPosition(anchor, 1920, 1080, 480, 300);

  test("sides sit at the margin, lifted slightly above centre", () => {
    expect(at("left")).toEqual({ x: PANEL_MARGIN, y: 366 });
    expect(at("right")).toEqual({ x: 1920 - 480 - PANEL_MARGIN, y: 366 });
  });

  test("the bottom strip leaves room for the taskbar", () => {
    expect(at("topcenter")).toEqual({ x: 720, y: PANEL_MARGIN });
    expect(at("bottomcenter")).toEqual({
      x: 720,
      y: 1080 - 300 - PANEL_MARGIN - TASKBAR_CLEARANCE,
    });
  });

  test("the old free position reads as the default, the top", () => {
    expect(at("custom")).toEqual(at("topcenter"));
  });

  test("a card wider than the screen still starts on it", () => {
    expect(anchorPosition("center", 1920, 1080, 1900, 300).x).toBe(
      PANEL_MARGIN,
    );
  });
});
