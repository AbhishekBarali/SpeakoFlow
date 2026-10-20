import { describe, expect, test } from "bun:test";
import type { ModelInfo } from "@/bindings";
import {
  isReplacementBusy,
  replacementPlan,
  replacementView,
  type DownloadSnapshot,
} from "./retiredReplacement";

const model = (overrides: Partial<ModelInfo> & { id: string }): ModelInfo => ({
  name: overrides.id,
  description: "",
  filename: `${overrides.id}.gguf`,
  url: null,
  sha256: null,
  size_mb: 100,
  is_downloaded: false,
  is_downloading: false,
  partial_size: 0,
  is_directory: false,
  engine_type: "TranscribeCpp",
  accuracy_score: 0.5,
  speed_score: 0.5,
  supports_translation: false,
  supports_streaming: false,
  is_recommended: false,
  recommended_rank: null,
  supported_languages: ["en"],
  supports_language_selection: false,
  is_custom: false,
  local_path: null,
  local_folder: null,
  ...overrides,
});

const OLD = "parakeet-tdt-0.6b-v3";
const NEW = "parakeet-tdt-0.6b-v3-gguf";

const retired = model({
  id: OLD,
  engine_type: "Parakeet",
  is_downloaded: true,
  is_legacy: true,
  is_retired: true,
  replacement_id: NEW,
});

const byId = (...models: ModelInfo[]) =>
  new Map(models.map((entry) => [entry.id, entry]));

const IDLE: DownloadSnapshot = {
  downloading: {},
  verifying: {},
  extracting: {},
  progress: {},
  stats: {},
  errors: {},
};

describe("replacementPlan", () => {
  test("downloads the new version and selects it when the retired one is in use", () => {
    expect(
      replacementPlan(retired, byId(retired, model({ id: NEW })), OLD),
    ).toEqual({ kind: "download", id: NEW, selectWhenDone: true });
  });

  test("leaves the selection alone for a retired model nobody uses", () => {
    expect(
      replacementPlan(retired, byId(retired, model({ id: NEW })), "whisper"),
    ).toEqual({ kind: "download", id: NEW, selectWhenDone: false });
  });

  test("switches to a new version that is already here", () => {
    const here = model({ id: NEW, is_downloaded: true });
    expect(replacementPlan(retired, byId(retired, here), OLD)).toEqual({
      kind: "select",
      id: NEW,
    });
  });

  test("has nothing to do without a listed replacement", () => {
    expect(replacementPlan(retired, byId(retired), OLD)).toBeNull();
    const orphan = model({ id: "old", is_retired: true, replacement_id: null });
    expect(replacementPlan(orphan, byId(orphan), "old")).toBeNull();
  });
});

describe("replacementView", () => {
  const listed = byId(retired, model({ id: NEW, size_mb: 640 }));

  test("offers the download, then follows it", () => {
    expect(replacementView(retired, listed, IDLE, null)?.phase).toBe(
      "download",
    );
    const view = replacementView(
      retired,
      listed,
      {
        ...IDLE,
        downloading: { [NEW]: true },
        progress: { [NEW]: { percentage: 42 } },
        stats: { [NEW]: { speed: 3.5 } },
      },
      null,
    );
    expect(view).toMatchObject({
      phase: "downloading",
      percent: 42,
      speed: 3.5,
    });
    expect(view?.model.id).toBe(NEW);
    expect(
      replacementView(
        retired,
        listed,
        { ...IDLE, verifying: { [NEW]: true } },
        null,
      )?.phase,
    ).toBe("verifying");
    expect(
      replacementView(
        retired,
        listed,
        { ...IDLE, extracting: { [NEW]: true } },
        null,
      )?.phase,
    ).toBe("extracting");
  });

  test("a failed download offers a retry with its reason", () => {
    expect(
      replacementView(
        retired,
        listed,
        { ...IDLE, errors: { [NEW]: "network unreachable" } },
        null,
      ),
    ).toMatchObject({ phase: "failed", error: "network unreachable" });
  });

  test("a retry in flight shows progress, not the old failure", () => {
    expect(
      replacementView(
        retired,
        listed,
        { ...IDLE, downloading: { [NEW]: true }, errors: { [NEW]: "x" } },
        null,
      )?.phase,
    ).toBe("downloading");
  });

  test("a new version on disk is used, and shows the switch while it happens", () => {
    const here = byId(retired, model({ id: NEW, is_downloaded: true }));
    expect(replacementView(retired, here, IDLE, null)?.phase).toBe("use");
    expect(replacementView(retired, here, IDLE, NEW)?.phase).toBe("switching");
  });

  test("no listed replacement, no view", () => {
    expect(replacementView(retired, byId(retired), IDLE, null)).toBeNull();
  });
});

test("only the download stages count as busy", () => {
  expect(isReplacementBusy("downloading")).toBe(true);
  expect(isReplacementBusy("verifying")).toBe(true);
  expect(isReplacementBusy("extracting")).toBe(true);
  expect(isReplacementBusy("download")).toBe(false);
  expect(isReplacementBusy("use")).toBe(false);
  expect(isReplacementBusy("switching")).toBe(false);
  expect(isReplacementBusy("failed")).toBe(false);
});
