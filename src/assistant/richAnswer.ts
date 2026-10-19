/**
 * An assistant answer as it should land on the clipboard.
 *
 * Answers are Markdown. Copied or inserted as text alone, a rich editor (Word,
 * Outlook, Gmail, Google Docs, Teams) showed `**Settings**` and `1.` as typed
 * characters. So a formatted answer goes on the clipboard twice: rendered as
 * HTML, which rich editors take, and as its Markdown text, which Notepad,
 * terminals, code editors and Markdown-aware fields (GitHub, Slack, Discord)
 * take. Each app picks the flavour it understands. This is the same split a
 * browser makes when you select a rendered page and copy it.
 *
 * The plain flavour stays Markdown rather than being stripped: the answer is
 * often headed for a field that renders Markdown itself (a GitHub comment, a
 * Slack message), and stripping it there would lose the formatting the user
 * asked for.
 */

type Renderer = (markdown: string) => string;

let renderer: Renderer | null = null;
let loading: Promise<Renderer> | null = null;

/**
 * Load the Markdown renderer (`answerMarkup.ts`). The assistant window calls
 * this when it mounts, so by the time anyone presses Copy it is already here
 * and the clipboard write happens with no `await` in front of it, which is
 * what WebKit's user-gesture rule for clipboard writes needs.
 */
export function loadAnswerRenderer(): Promise<Renderer> {
  loading ??= import("./answerMarkup").then((module) => {
    renderer = module.renderAnswerMarkup;
    return renderer;
  });
  return loading;
}

/**
 * Tags plain text already says on its own. An answer whose HTML has nothing
 * else in it is prose, and gets no HTML flavour: pasting `<p>` into Word or
 * Outlook would restyle ordinary text in the HTML default font for no gain.
 */
const PROSE_TAGS = new Set(["p", "br"]);

/** `render(markdown)`, or `null` when it carries no formatting worth pasting. */
function formattedHtml(render: Renderer, markdown: string): string | null {
  if (!markdown.trim()) return null;
  const html = render(markdown);
  for (const [, tag] of html.matchAll(/<([a-z][a-z0-9]*)\b/gi)) {
    if (!PROSE_TAGS.has(tag.toLowerCase())) return html;
  }
  return null;
}

/**
 * The answer rendered to HTML, or `null` when it has no formatting worth
 * carrying (plain prose, or nothing at all).
 */
export async function answerHtml(markdown: string): Promise<string | null> {
  return formattedHtml(renderer ?? (await loadAnswerRenderer()), markdown);
}

/**
 * Copy an answer: HTML beside the Markdown when it is formatted, the Markdown
 * alone otherwise, or when this webview cannot write HTML to the clipboard
 * (no `ClipboardItem`, or a write it refuses).
 */
export async function copyAnswer(markdown: string): Promise<void> {
  const html = formattedHtml(
    renderer ?? (await loadAnswerRenderer()),
    markdown,
  );
  if (
    html &&
    typeof ClipboardItem !== "undefined" &&
    navigator.clipboard?.write
  ) {
    try {
      await navigator.clipboard.write([
        new ClipboardItem({
          "text/plain": new Blob([markdown], { type: "text/plain" }),
          "text/html": new Blob([html], { type: "text/html" }),
        }),
      ]);
      return;
    } catch {
      // Fall through to the text every earlier version copied.
    }
  }
  await navigator.clipboard.writeText(markdown);
}
