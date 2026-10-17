import { expect, test } from "bun:test";
import {
  groupIntoTurns,
  ME_SPEAKER_KEY,
  speakerNameResolver,
  transcriptToText,
  type TranscriptItem,
} from "./speakers";

const item = (
  speakerKey: string,
  text: string,
  startMs: number,
): TranscriptItem => ({
  key: `${speakerKey}-${startMs}`,
  source: speakerKey === ME_SPEAKER_KEY ? "mic" : "system",
  speakerKey,
  startMs,
  endMs: startMs + 1000,
  text,
  confidence: null,
});

const labels = {
  me: "You",
  others: "Others",
  numbered: (n: number) => `Speaker ${n}`,
};

test("the copied transcript is one named paragraph per turn", () => {
  const items = [
    item("spk_0", "Ich hab jetzt alles gemacht.", 0),
    // Same speaker again: joins the turn rather than repeating the name.
    item("spk_0", "Und ich bin richtig unglücklich.", 2000),
    item(ME_SPEAKER_KEY, "Was ist passiert?", 4000),
    item("spk_1", "Ein Unfall.", 6000),
  ];
  const turns = groupIntoTurns(items);
  const nameFor = speakerNameResolver([], ["spk_0", "spk_1"], labels);

  expect(transcriptToText(turns, nameFor)).toBe(
    [
      "Speaker 1: Ich hab jetzt alles gemacht. Und ich bin richtig unglücklich.",
      "You: Was ist passiert?",
      "Speaker 2: Ein Unfall.",
    ].join("\n\n"),
  );
});

test("a renamed speaker is copied under the name the user gave them", () => {
  const turns = groupIntoTurns([item("spk_0", "Hallo.", 0)]);
  const nameFor = speakerNameResolver(
    [{ speaker_key: "spk_0", display_name: "Michi", is_me: false }],
    ["spk_0"],
    labels,
  );
  expect(transcriptToText(turns, nameFor)).toBe("Michi: Hallo.");
});

test("an empty transcript copies as nothing", () => {
  expect(transcriptToText([], () => "x")).toBe("");
});
