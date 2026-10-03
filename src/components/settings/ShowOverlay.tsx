import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { ScreenPositionMap, type ScreenSpot } from "../ui/ScreenPositionMap";
import { useSettings } from "../../hooks/useSettings";
import type { OverlayPosition } from "@/bindings";

interface ShowOverlayProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

type ShownPosition = Exclude<OverlayPosition, "none">;

/** The six places the overlay can sit, as spots on a pictured screen: pill
 *  shaped, along the top and bottom edges, in the middle or in a corner. */
const SPOTS: ReadonlyArray<{
  value: ShownPosition;
  box: ScreenSpot<ShownPosition>["box"];
}> = [
  { value: "topleft", box: { left: 5, top: 13, width: 20, height: 10 } },
  { value: "top", box: { left: 40, top: 13, width: 20, height: 10 } },
  { value: "topright", box: { left: 75, top: 13, width: 20, height: 10 } },
  { value: "bottomleft", box: { left: 5, top: 77, width: 20, height: 10 } },
  { value: "bottom", box: { left: 40, top: 77, width: 20, height: 10 } },
  { value: "bottomright", box: { left: 75, top: 77, width: 20, height: 10 } },
];

/**
 * Where the dictation overlay sits: the same screen-map picker the assistant's
 * panel uses, with the list beside it (which also holds "None", since a hidden
 * overlay has no spot on the map).
 */
export const ShowOverlay: React.FC<ShowOverlayProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const label = (value: OverlayPosition) =>
      t(`settings.advanced.overlay.options.${value}`);

    const overlayOptions = [
      ...SPOTS.map((spot) => ({ value: spot.value, label: label(spot.value) })),
      { value: "none", label: label("none") },
    ];

    const selectedPosition = (getSetting("overlay_position") ||
      "bottom") as OverlayPosition;
    const busy = isUpdating("overlay_position");
    const select = (value: OverlayPosition) => {
      if (!busy) updateSetting("overlay_position", value);
    };

    return (
      <SettingContainer
        title={t("settings.advanced.overlay.title")}
        description={t("settings.advanced.overlay.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
        details={
          <ScreenPositionMap
            spots={SPOTS.map((spot) => ({ ...spot, label: label(spot.value) }))}
            value={selectedPosition === "none" ? null : selectedPosition}
            onChange={select}
            label={t("settings.advanced.overlay.title")}
            shape="pill"
            className={`w-full max-w-[15rem] ${selectedPosition === "none" ? "opacity-60" : ""}`}
          />
        }
      >
        <Dropdown
          options={overlayOptions}
          selectedValue={selectedPosition}
          onSelect={(value) => select(value as OverlayPosition)}
          disabled={busy}
        />
      </SettingContainer>
    );
  },
);
