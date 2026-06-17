import React, { useState } from "react";
import { ChevronDown } from "lucide-react";
import { useTranslation } from "react-i18next";

interface MoreOptionsProps {
  children: React.ReactNode;
  /** Optional override for the collapsed label. */
  label?: string;
  /** Optional override for the expanded label. */
  labelOpen?: string;
  defaultOpen?: boolean;
}

/**
 * Progressive-disclosure control. Keeps secondary settings out of sight until
 * asked for. Render it as the last child inside a `SettingsGroup` — the
 * revealed rows appear above the toggle so the "Show less" affordance stays
 * pinned to the bottom of the card.
 */
export const MoreOptions: React.FC<MoreOptionsProps> = ({
  children,
  label,
  labelOpen,
  defaultOpen = false,
}) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(defaultOpen);

  return (
    <>
      {open && children}
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="w-full flex items-center justify-center gap-1.5 px-4 py-2.5 text-xs font-medium text-muted hover:text-ink transition-colors cursor-pointer"
      >
        <span>
          {open
            ? (labelOpen ?? t("common.showLess"))
            : (label ?? t("common.showAdvanced"))}
        </span>
        <ChevronDown
          className={`w-3.5 h-3.5 transition-transform duration-200 ${open ? "rotate-180" : ""}`}
        />
      </button>
    </>
  );
};
