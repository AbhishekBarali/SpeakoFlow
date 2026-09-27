import { expect, test } from "bun:test";
import { toggleTaskAt } from "./notesTasks";

const NOTES = [
  "## Next steps",
  "- [ ] **Priya** — send the copy",
  "- [ ] **Priya** — send the copy",
  "- [x] **Me** — book the room",
].join("\n");

const lineStart = (index: number) =>
  NOTES.split("\n")
    .slice(0, index)
    .reduce((sum, line) => sum + line.length + 1, 0);

test("ticking a box changes only that item, even beside an identical one", () => {
  const next = toggleTaskAt(NOTES, lineStart(2));
  expect(next.split("\n")[1]).toBe("- [ ] **Priya** — send the copy");
  expect(next.split("\n")[2]).toBe("- [x] **Priya** — send the copy");
});

test("unticking a done item clears it", () => {
  const next = toggleTaskAt(NOTES, lineStart(3));
  expect(next.split("\n")[3]).toBe("- [ ] **Me** — book the room");
});

test("toggling twice is a no-op", () => {
  const offset = lineStart(1);
  expect(toggleTaskAt(toggleTaskAt(NOTES, offset), offset)).toBe(NOTES);
});

test("an offset with no box after it leaves the notes alone", () => {
  expect(toggleTaskAt("## Summary\nNo tasks here.", 0)).toBe(
    "## Summary\nNo tasks here.",
  );
  expect(toggleTaskAt(NOTES, -1)).toBe(NOTES);
  expect(toggleTaskAt(NOTES, NOTES.length + 5)).toBe(NOTES);
});
