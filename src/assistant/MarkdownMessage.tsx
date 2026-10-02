import { memo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

/** Module scope so every render hands react-markdown the same array. */
const REMARK_PLUGINS = [remarkGfm];

interface MarkdownMessageProps {
  content: string;
  components?: Components;
}

/**
 * One rendered Markdown message.
 *
 * react-markdown does no memoisation of its own: every render of the element
 * runs the whole remark / remark-gfm / rehype pipeline. The panels around it
 * re-render many times a second during a call (the level meter, streamed
 * tokens, the tool timer), so without `memo` every past reply was re-parsed on
 * each of those renders. Memoised on `content` and `components` (callers pass a
 * module-scope components map), a message is parsed again only when its text
 * actually changes.
 */
const MarkdownMessage = memo(function MarkdownMessage({
  content,
  components,
}: MarkdownMessageProps) {
  return (
    <ReactMarkdown remarkPlugins={REMARK_PLUGINS} components={components}>
      {content}
    </ReactMarkdown>
  );
});

export default MarkdownMessage;
