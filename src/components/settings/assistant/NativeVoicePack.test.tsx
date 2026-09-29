import { expect, mock, test } from "bun:test";
import React from "react";
import { act, create } from "react-test-renderer";

/**
 * The rows under a native engine's tile: which pack they download and which
 * voices they offer have to match what the backend loads, or a user downloads
 * one Kitten size and hears another.
 */

mock.module("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) =>
      options ? `${key}|${JSON.stringify(options)}` : key,
  }),
}));

const models = [
  { id: "kitten-nano-0.8", size_mb: 64, is_downloaded: true },
  { id: "kitten-micro-0.8", size_mb: 45, is_downloaded: false },
  { id: "kitten-mini-0.8", size_mb: 68, is_downloaded: false },
  { id: "pocket-tts", size_mb: 178, is_downloaded: false },
  { id: "supertonic-3-int8", size_mb: 129, is_downloaded: false },
];
mock.module("@/stores/modelStore", () => ({
  useModelStore: (select: (state: unknown) => unknown) =>
    select({
      models,
      downloadingModels: {},
      verifyingModels: {},
      extractingModels: {},
      downloadProgress: {},
      downloadModel: async () => {},
      cancelDownload: async () => {},
      deleteModel: async () => {},
    }),
}));

interface StubDropdownProps {
  options: { value: string; label: string }[];
  selectedValue: string | null;
  onSelect: (value: string) => void;
}
const StubDropdown: React.FC<StubDropdownProps> = () => null;
mock.module("@/components/ui/Dropdown", () => ({ Dropdown: StubDropdown }));
mock.module("@/components/ui/SettingContainer", () => ({
  SettingContainer: ({
    title,
    description,
    children,
  }: {
    title: string;
    description?: string;
    children: React.ReactNode;
  }) => (
    <section data-title={title} data-description={description}>
      {children}
    </section>
  ),
}));

const { NativeEngineRows } = await import("./NativeVoicePack");

const render = (
  engine: "kitten" | "pocket" | "supertonic",
  model = "",
  voice = "",
) => {
  const chosen: { model?: string; voice?: string } = {};
  let root!: ReturnType<typeof create>;
  act(() => {
    root = create(
      <NativeEngineRows
        engine={engine}
        model={model}
        voice={voice}
        unsupported={false}
        disabled={false}
        onModel={(m) => (chosen.model = m)}
        onVoice={(v) => (chosen.voice = v)}
      />,
    );
  });
  const dropdowns = root.root
    .findAllByType(StubDropdown)
    .map((node) => node.props as StubDropdownProps);
  const titles = root.root
    .findAll((node) => typeof node.props["data-title"] === "string")
    .map((node) => node.props["data-title"] as string);
  return { dropdowns, titles, chosen };
};

test("Kitten offers its three sizes and downloads the one chosen", () => {
  const { dropdowns, titles, chosen } = render("kitten", "kitten-mini-0.8");
  const [sizes, voices] = dropdowns;
  expect(sizes.options.map((o) => o.value)).toEqual([
    "kitten-nano-0.8",
    "kitten-micro-0.8",
    "kitten-mini-0.8",
  ]);
  expect(sizes.selectedValue).toBe("kitten-mini-0.8");
  // The size label carries the download size from the catalog.
  expect(sizes.options[2].label).toContain("68");
  expect(titles).toContain("settings.assistant.tts.packLabel.kitten");
  expect(voices.options).toHaveLength(8);
  expect(voices.selectedValue).toBe("Bella");
  sizes.onSelect("kitten-micro-0.8");
  expect(chosen.model).toBe("kitten-micro-0.8");
});

test("an older Kitten setting with no size reads as nano", () => {
  const [sizes] = render("kitten", "").dropdowns;
  expect(sizes.selectedValue).toBe("kitten-nano-0.8");
});

test("Pocket has no size, 14 voices, and keeps a saved voice", () => {
  const { dropdowns, titles, chosen } = render("pocket", "", "george");
  expect(dropdowns).toHaveLength(1);
  const [voices] = dropdowns;
  expect(voices.options).toHaveLength(14);
  expect(voices.selectedValue).toBe("George");
  expect(voices.options[0].label).toContain("voiceNamedFemale");
  expect(titles).toContain("settings.assistant.tts.packLabel.pocket");
  voices.onSelect("Mary");
  expect(chosen.voice).toBe("Mary");
});

test("a Pocket voice this build no longer offers shows the default", () => {
  const [voices] = render("pocket", "", "Marius").dropdowns;
  expect(voices.selectedValue).toBe("Mary");
});

test("Supertonic's numbered voices get readable names", () => {
  const [voices] = render("supertonic", "", "Bella").dropdowns;
  expect(voices.options).toHaveLength(10);
  expect(voices.selectedValue).toBe("F1");
  expect(voices.options[0].label).toContain("voiceNumberedFemale");
  expect(voices.options[5].label).toContain("voiceNumberedMale");
});
