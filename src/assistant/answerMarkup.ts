import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * Markdown to an HTML string, through the same react-markdown + remark-gfm
 * pipeline the answer card renders with, so a paste matches what was on screen.
 *
 * Its own module because `react-dom/server` is ~70 KB that only Copy and Insert
 * need: `richAnswer.ts` loads it on demand, which keeps it out of the chunk
 * every window starts with.
 *
 * react-markdown never passes raw HTML through (it is escaped to visible text,
 * as on the card) and drops unsafe link protocols, so the output holds only the
 * elements the Markdown describes. The card's own renderers (the code block's
 * copy button) are deliberately not used: they are interface, not content.
 */

/** Module scope so every call hands react-markdown the same array. */
const REMARK_PLUGINS = [remarkGfm];

/**
 * Word and Outlook read a bare `<p>` as "Normal (Web)", which is Times New
 * Roman, so a pasted answer's paragraphs came out in a different font from its
 * lists and from the document around it (measured by pasting into Word over
 * COM). `MsoNormal` is the class Office's own HTML uses for the document's
 * Normal style; every other editor ignores an unknown class.
 */
const COMPONENTS: Components = {
  p: ({ children }) => createElement("p", { className: "MsoNormal" }, children),
};

export function renderAnswerMarkup(markdown: string): string {
  return renderToStaticMarkup(
    createElement(
      ReactMarkdown,
      { remarkPlugins: REMARK_PLUGINS, components: COMPONENTS },
      markdown,
    ),
  ).trim();
}
