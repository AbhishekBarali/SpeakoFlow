/**
 * Flip the task checkbox of the list item that starts at `offset`.
 *
 * `offset` is where react-markdown says the `<li>` begins in the source, so the
 * first `[ ]` / `[x]` at or after it is that item's box — not the first box in
 * the document with the same text, which is what matching on the label would
 * find. Returns the notes unchanged if no box is there.
 */
export const toggleTaskAt = (notes: string, offset: number): string => {
  if (offset < 0 || offset >= notes.length) return notes;
  const box = /\[( |x|X)\]/.exec(notes.slice(offset));
  if (!box) return notes;
  const at = offset + box.index + 1;
  const next = notes[at] === " " ? "x" : " ";
  return notes.slice(0, at) + next + notes.slice(at + 1);
};
