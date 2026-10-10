import { useCallback } from "react";
import { useSettings } from "@/hooks/useSettings";

/**
 * How recording shortcuts are driven. `auto` is hold-or-tap: hold to talk, or
 * tap once to lock the recording and tap again to stop.
 *
 * Stored as two settings so everything that only cares about hold-style
 * recording keeps reading `push_to_talk`: hold is (true, false), tap is
 * (false, false), auto is (true, true). The backend keeps that invariant too.
 */
export type RecordingModeChoice = "hold" | "tap" | "auto";

export function useRecordingMode() {
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const holdStyle = getSetting("push_to_talk") ?? true;
  const dynamic = getSetting("dynamic_shortcuts") ?? false;
  const mode: RecordingModeChoice = !holdStyle
    ? "tap"
    : dynamic
      ? "auto"
      : "hold";

  const setMode = useCallback(
    async (next: RecordingModeChoice) => {
      // The backend keeps the pair consistent on its own, but the store only
      // learns about the field that was sent. Auto used to send just
      // `dynamic_shortcuts`, so `push_to_talk` stayed false in the store, the
      // mode still read "tap", and clicking Hold or tap appeared to do nothing.
      // So every field that changes is sent, in an order that never leaves
      // auto on over tap.
      if (next === "auto") {
        await updateSetting("push_to_talk", true);
        await updateSetting("dynamic_shortcuts", true);
        return;
      }
      if (next === "hold") {
        await updateSetting("dynamic_shortcuts", false);
        await updateSetting("push_to_talk", true);
        return;
      }
      await updateSetting("dynamic_shortcuts", false);
      await updateSetting("push_to_talk", false);
    },
    [updateSetting],
  );

  return {
    mode,
    setMode,
    updating: isUpdating("push_to_talk") || isUpdating("dynamic_shortcuts"),
  };
}
