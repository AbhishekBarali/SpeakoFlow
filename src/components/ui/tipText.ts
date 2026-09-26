/**
 * Split an explanation into what an (i) shows first and what it keeps behind
 * "More".
 *
 * Settings text was written as paragraphs — privacy notes, edge cases, how the
 * feature works inside — and an (i) that opened onto a paragraph was the
 * complaint. The first sentence is almost always the useful one ("what does
 * this do?"), so the tip leads with it and the rest is one click away rather
 * than gone. This works on the translated text, so every language gets it
 * without a single string being rewritten.
 */

/** Below this, a tip is short enough to show whole. */
const SHORT_ENOUGH = 120;
/** A remainder this short is not worth a "More" link. */
const MIN_REST = 24;

/**
 * Words that end in a period without ending a sentence. Lower-cased, without
 * the trailing dot.
 */
const ABBREVIATIONS = new Set([
  "e.g",
  "i.e",
  "etc",
  "vs",
  "approx",
  "incl",
  "no",
  "mr",
  "mrs",
  "ms",
  "dr",
]);

export interface TipText {
  lead: string;
  rest: string | null;
}

/** Index just past the first sentence end, or -1 when there is none. */
const firstSentenceEnd = (text: string): number => {
  // A sentence ends at . ! ? (optionally followed by a closing quote or
  // bracket) when whitespace and a new sentence follow.
  const pattern = /[.!?]["'”’)\]]*\s+(?=["'“‘(\[]?[A-Z0-9À-ÖØ-Þ])/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    const before = text.slice(0, match.index);
    const word = before.split(/\s+/).pop()?.toLowerCase() ?? "";
    if (match[0].startsWith(".") && ABBREVIATIONS.has(word)) continue;
    return match.index + match[0].trimEnd().length;
  }
  return -1;
};

export const splitTip = (text: string): TipText => {
  const clean = text.trim().replace(/\s+/g, " ");
  if (clean.length <= SHORT_ENOUGH) return { lead: clean, rest: null };
  const end = firstSentenceEnd(clean);
  if (end < 0) return { lead: clean, rest: null };
  const lead = clean.slice(0, end).trim();
  const rest = clean.slice(end).trim();
  if (rest.length < MIN_REST) return { lead: clean, rest: null };
  return { lead, rest };
};
