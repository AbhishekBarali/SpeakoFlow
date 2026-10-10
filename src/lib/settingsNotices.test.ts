import { describe, expect, test } from "bun:test";
import {
  describeSettingsNotice,
  settingsNoticesToTakeDown,
} from "./settingsNotices";

const t = (key: string, options?: Record<string, unknown>) =>
  options ? `${key} ${JSON.stringify(options)}` : key;

describe("describeSettingsNotice", () => {
  test("a restore is an ordinary toast, losses are warnings", () => {
    expect(
      describeSettingsNotice({ kind: "restored", kept_as: null }, t).warning,
    ).toBe(false);
    for (const kind of ["unrecoverable", "not_saving", "save_failing"] as const)
      expect(describeSettingsNotice({ kind, kept_as: null }, t).warning).toBe(
        true,
      );
  });

  test("names the kept file only when there is one", () => {
    const kept = describeSettingsNotice(
      {
        kind: "unrecoverable",
        kept_as: "settings_store.json.corrupt-1700000000",
      },
      t,
    );
    expect(kept.description).toContain(
      "settings_store.json.corrupt-1700000000",
    );
    expect(
      describeSettingsNotice({ kind: "unrecoverable", kept_as: null }, t)
        .description,
    ).toBeUndefined();
  });
});

describe("settingsNoticesToTakeDown", () => {
  test("takes down what the backend stopped reporting", () => {
    expect(
      settingsNoticesToTakeDown(
        ["restored", "save_failing"],
        [{ kind: "restored", kept_as: null }],
      ),
    ).toEqual(["save_failing"]);
    expect(
      settingsNoticesToTakeDown(
        ["save_failing"],
        [{ kind: "save_failing", kept_as: null }],
      ),
    ).toEqual([]);
  });
});
