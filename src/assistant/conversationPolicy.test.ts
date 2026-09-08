import { describe, expect, it } from "bun:test";
import { matchDeviceByName } from "./conversationPolicy";

const device = (
  kind: MediaDeviceKind,
  label: string,
  deviceId = label,
): MediaDeviceInfo =>
  ({ kind, label, deviceId, groupId: "" }) as MediaDeviceInfo;

describe("matchDeviceByName", () => {
  it("prefers an exact label", () => {
    const devices = [
      device("audioinput", "Yeti Stereo Microphone"),
      device("audioinput", "Default - Yeti Stereo Microphone"),
    ];
    expect(
      matchDeviceByName(devices, "audioinput", "Yeti Stereo Microphone")?.label,
    ).toBe("Yeti Stereo Microphone");
  });

  it("sees through the decoration Chromium adds to a label", () => {
    const devices = [device("audioinput", "Default - Yeti Stereo Microphone")];
    expect(
      matchDeviceByName(devices, "audioinput", "Yeti Stereo Microphone")?.label,
    ).toBe("Default - Yeti Stereo Microphone");
  });

  it("ignores devices of the wrong kind", () => {
    const devices = [device("audiooutput", "Yeti Stereo Microphone")];
    expect(
      matchDeviceByName(devices, "audioinput", "Yeti Stereo Microphone"),
    ).toBeNull();
  });

  /**
   * Labels are empty until microphone permission is granted, and an empty label
   * matches every substring test — so without this it would return the first
   * unnamed device and record from the wrong microphone.
   */
  it("never matches an unnamed device", () => {
    const devices = [
      device("audioinput", "", "id-1"),
      device("audioinput", "  ", "id-2"),
    ];
    expect(matchDeviceByName(devices, "audioinput", "Yeti")).toBeNull();
  });

  /**
   * The stored name comes from the native audio engine, so on Linux it is not a
   * browser label at all. Answering "use the default" is what keeps the call
   * startable; guessing between two candidates would silently record from the
   * wrong one.
   */
  it("gives up rather than guessing", () => {
    const devices = [
      device("audioinput", "Microphone (USB Audio)"),
      device("audioinput", "Microphone (Realtek)"),
    ];
    expect(matchDeviceByName(devices, "audioinput", "Microphone")).toBeNull();
    expect(
      matchDeviceByName(devices, "audioinput", "sysdefault:CARD=PCH"),
    ).toBeNull();
  });
});
