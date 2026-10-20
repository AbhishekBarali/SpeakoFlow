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

import { commands } from "@/bindings";

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
  loading ??= import("./answerMarkup").then(
    (module) => {
      renderer = module.renderAnswerMarkup;
      return renderer;
    },
    (error: unknown) => {
      // Not cached: a later Copy or Insert tries the import again.
      loading = null;
      throw error;
    },
  );
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
 * carrying (plain prose, or nothing at all), or when the renderer could not
 * be loaded: the answer is then copied or inserted as text, as it always was,
 * rather than not at all.
 */
export async function answerHtml(markdown: string): Promise<string | null> {
  try {
    return formattedHtml(renderer ?? (await loadAnswerRenderer()), markdown);
  } catch {
    return null;
  }
}

/**
 * The app's own clipboard write (`assistant_copy_answer`), answering whether
 * it worked. Injected so the tests can stand in for the backend.
 */
export type NativeCopy = (
  text: string,
  html: string | null,
) => Promise<boolean>;

const nativeCopy: NativeCopy = async (text, html) => {
  try {
    return (await commands.assistantCopyAnswer(text, html)).status === "ok";
  } catch {
    return false;
  }
};

/**
 * Copy an answer: HTML beside the Markdown when it is formatted, the Markdown
 * alone otherwise.
 *
 * The webview's clipboard goes first, as it always has. WebKit (macOS, Linux)
 * refuses that write once the click's user activation has passed or the panel
 * is not the focused window, and some WebKitGTK builds have no
 * `ClipboardItem`, so a refused write is retried through the app's own
 * clipboard, which needs neither. A formatted answer keeps its HTML there too.
 */
export async function copyAnswer(
  markdown: string,
  native: NativeCopy = nativeCopy,
): Promise<void> {
  const html = await answerHtml(markdown);
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
      // Refused: the app's own clipboard below.
    }
  }
  if (html && (await native(markdown, html))) return;
  try {
    if (!navigator.clipboard?.writeText) throw new Error("no clipboard");
    await navigator.clipboard.writeText(markdown);
  } catch (error) {
    if (!(await native(markdown, null))) throw error;
  }
}
