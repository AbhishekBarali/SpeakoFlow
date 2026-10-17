import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Check, Copy } from "lucide-react";
import { copyText } from "./api";

/** A quiet text button in a meeting toolbar (template, regenerate, copy). */
export const ICON_ACTION =
  "inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-lg px-2.5 text-[0.8125rem] font-medium text-muted transition-colors hover:bg-ink/6 hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 disabled:cursor-not-allowed disabled:opacity-50 aria-expanded:bg-ink/6 aria-expanded:text-ink";

/** How long the button says "Copied". */
const COPIED_MS = 1600;

/**
 * Copy something to the clipboard from a toolbar, and say so in place.
 *
 * `text` is a function so a long transcript is only joined into one string
 * when someone actually presses the button, not on every render.
 */
export const CopyAction: React.FC<{
  text: () => string;
  /** Hover text, for when the label alone ("Copy") does not say what is copied. */
  title?: string;
  disabled?: boolean;
}> = ({ text, title, disabled = false }) => {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), COPIED_MS);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const copy = () => {
    const value = text();
    if (!value.trim()) return;
    void copyText(value)
      .then(() => setCopied(true))
      .catch((error: unknown) => {
        toast.error(t("meetings.errors.copyFailed", { error: String(error) }));
      });
  };

  return (
    <button
      type="button"
      onClick={copy}
      disabled={disabled}
      title={title}
      className={ICON_ACTION}
    >
      {copied ? (
        <Check className="h-3.5 w-3.5 text-success" aria-hidden="true" />
      ) : (
        <Copy className="h-3.5 w-3.5" aria-hidden="true" />
      )}
      <span aria-live="polite">
        {copied ? t("meetings.summary.copied") : t("meetings.summary.copy")}
      </span>
    </button>
  );
};
