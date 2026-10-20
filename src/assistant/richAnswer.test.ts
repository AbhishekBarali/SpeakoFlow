import { afterEach, expect, test } from "bun:test";
import { answerHtml, copyAnswer } from "./richAnswer";

/**
 * What a copied or inserted answer puts on the clipboard. The case behind it:
 * an answer with bold steps and a numbered list pasted into a rich editor as
 * `**Settings → Advanced**` and literal `1.`.
 */

const STEPS = [
  "Could you try running it on the processor instead?",
  "",
  "1. Open **Settings → Advanced** and turn on **Experimental features**.",
  "2. Set **ONNX acceleration** to **CPU**.",
  "3. Paste `%LOCALAPPDATA%\\logs` and press **Enter**.",
].join("\n");

test("a formatted answer renders to the HTML a rich editor shows", async () => {
  const html = await answerHtml(STEPS);
  expect(html).not.toBeNull();
  expect(html).toContain("<ol>");
  expect(html).toContain("<strong>Settings → Advanced</strong>");
  expect(html).toContain("<code>%LOCALAPPDATA%\\logs</code>");
  expect(html).not.toContain("**");
});

test("paragraphs take the document's Normal style in Word and Outlook", async () => {
  const html = await answerHtml(STEPS);
  expect(html).toContain('<p class="MsoNormal">Could you try');
  expect(html).not.toContain("<p>");
});

test("GFM tables and strikethrough render like the card does", async () => {
  const html = await answerHtml("| a | b |\n|---|---|\n| 1 | 2 |\n\n~~old~~");
  expect(html).toContain("<table>");
  expect(html).toContain("<del>old</del>");
});

test("plain prose gets no HTML flavour", async () => {
  expect(await answerHtml("Thanks for reporting this, Tim.")).toBeNull();
  expect(
    await answerHtml("First paragraph.\n\nSecond one,\nwith a break."),
  ).toBeNull();
  expect(await answerHtml("   ")).toBeNull();
  expect(await answerHtml("")).toBeNull();
});

test("raw HTML and unsafe links in an answer never reach the clipboard as markup", async () => {
  const html = await answerHtml(
    '**Hi** <img src=x onerror="alert(1)"> [link](javascript:alert(1))',
  );
  expect(html).not.toBeNull();
  // Escaped to visible text, exactly as the card shows it, never an element.
  expect(html).not.toContain("<img");
  expect(html).toContain("&lt;img");
  expect(html).not.toContain("javascript:");
});

// ---------------------------------------------------------------------------
// copyAnswer
// ---------------------------------------------------------------------------

type Written =
  | { kind: "items"; items: Record<string, string> }
  | { kind: "text"; text: string };

const originalClipboard = Object.getOwnPropertyDescriptor(
  globalThis.navigator,
  "clipboard",
);
const originalClipboardItem = (globalThis as { ClipboardItem?: unknown })
  .ClipboardItem;

afterEach(() => {
  if (originalClipboard) {
    Object.defineProperty(globalThis.navigator, "clipboard", originalClipboard);
  } else {
    delete (globalThis.navigator as { clipboard?: unknown }).clipboard;
  }
  (globalThis as { ClipboardItem?: unknown }).ClipboardItem =
    originalClipboardItem;
});

class FakeClipboardItem {
  constructor(readonly items: Record<string, Blob>) {}
}

function installClipboard(
  options: { writeFails?: boolean; writeTextFails?: boolean } = {},
): Written[] {
  const written: Written[] = [];
  (globalThis as { ClipboardItem?: unknown }).ClipboardItem = FakeClipboardItem;
  Object.defineProperty(globalThis.navigator, "clipboard", {
    configurable: true,
    value: {
      write: async (items: FakeClipboardItem[]) => {
        if (options.writeFails) throw new Error("NotAllowedError");
        const flat: Record<string, string> = {};
        for (const [type, blob] of Object.entries(items[0].items)) {
          flat[type] = await blob.text();
        }
        written.push({ kind: "items", items: flat });
      },
      writeText: async (text: string) => {
        if (options.writeTextFails) throw new Error("NotAllowedError");
        written.push({ kind: "text", text });
      },
    },
  });
  return written;
}

/** Stands in for the app's own clipboard (`assistant_copy_answer`). */
function nativeClipboard(works = true) {
  const calls: Array<{ text: string; html: string | null }> = [];
  const native = async (text: string, html: string | null) => {
    calls.push({ text, html });
    return works;
  };
  return { calls, native };
}

test("copying a formatted answer writes HTML beside the Markdown", async () => {
  const written = installClipboard();
  const { calls, native } = nativeClipboard();
  await copyAnswer(STEPS, native);
  expect(written).toHaveLength(1);
  const entry = written[0];
  if (entry.kind !== "items") throw new Error("expected a rich write");
  expect(entry.items["text/plain"]).toBe(STEPS);
  expect(entry.items["text/html"]).toContain("<strong>CPU</strong>");
  expect(calls).toEqual([]);
});

test("copying prose writes text alone, as before", async () => {
  const written = installClipboard();
  const { calls, native } = nativeClipboard();
  await copyAnswer("Thanks, Tim.", native);
  expect(written).toEqual([{ kind: "text", text: "Thanks, Tim." }]);
  expect(calls).toEqual([]);
});

test("a refused rich write is copied with its HTML by the app instead", async () => {
  const written = installClipboard({ writeFails: true });
  const { calls, native } = nativeClipboard();
  await copyAnswer(STEPS, native);
  expect(written).toEqual([]);
  expect(calls).toHaveLength(1);
  expect(calls[0].text).toBe(STEPS);
  expect(calls[0].html).toContain("<strong>CPU</strong>");
});

test("with both rich writes refused, the Markdown text is copied", async () => {
  const written = installClipboard({ writeFails: true });
  const { native } = nativeClipboard(false);
  await copyAnswer(STEPS, native);
  expect(written).toEqual([{ kind: "text", text: STEPS }]);
});

test("a refused text write is copied by the app instead", async () => {
  installClipboard({ writeTextFails: true });
  const { calls, native } = nativeClipboard();
  await copyAnswer("Thanks, Tim.", native);
  expect(calls).toEqual([{ text: "Thanks, Tim.", html: null }]);
});

test("a copy that fails everywhere is reported", async () => {
  installClipboard({ writeTextFails: true });
  const { native } = nativeClipboard(false);
  await expect(copyAnswer("Thanks, Tim.", native)).rejects.toThrow();
});
