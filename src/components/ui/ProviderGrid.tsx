import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown } from "lucide-react";
import {
  ProviderTile,
  type ProviderKind,
} from "@/components/icons/ProviderLogos";
import { LogoChoice } from "./LogoChoice";

export interface ProviderGridOption {
  value: string;
  label: string;
  /** A key is saved, or the provider needs none. */
  ready?: boolean;
  /** What it is set to (usually the model), shown under the name. */
  hint?: string;
  disabled?: boolean;
}

/** The providers most people reach for, in the order they are offered. */
const POPULAR: Partial<Record<ProviderKind, string[]>> = {
  llm: [
    "openai",
    "anthropic",
    "gemini",
    "openrouter",
    "groq",
    "deepseek",
    "mistral",
    "xai",
    "local",
  ],
};

/** "AWS Bedrock (Mantle)" → name "AWS Bedrock", detail "Mantle". */
const splitLabel = (label: string): { name: string; detail?: string } => {
  const match = /^(.*?)\s*\(([^)]*)\)\s*$/.exec(label);
  return match ? { name: match[1], detail: match[2] } : { name: label };
};

/**
 * Pick a cloud provider by its logo.
 *
 * A dozen AI companies behind one dropdown meant reading a list of names to
 * find a mark everyone already recognises. This shows them as tiles: the
 * current one ringed, each one's configured model under its name, and a dot on
 * the ones whose key is already saved — so "which of these have I set up?" is
 * answered without opening anything. Long lists start with the providers that
 * matter (the selected one, the ones set up, the popular ones) and keep the
 * rest one click away.
 */
export const ProviderGrid: React.FC<{
  kind: ProviderKind;
  options: ProviderGridOption[];
  value: string | null;
  onChange: (value: string) => void;
  /** Accessible name for the group. */
  label: string;
  disabled?: boolean;
  /** How many tiles to show before "Show all". */
  collapsedCount?: number;
}> = ({
  kind,
  options,
  value,
  onChange,
  label,
  disabled = false,
  collapsedCount = 8,
}) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const collapsible = options.length > collapsedCount + 1;

  const shown = useMemo(() => {
    if (!collapsible || expanded) return options;
    const popular = POPULAR[kind] ?? [];
    const rank = (option: ProviderGridOption) => {
      if (option.value === value) return 0;
      if (option.ready) return 1;
      const index = popular.indexOf(option.value);
      return index >= 0 ? 2 + index / 100 : 3;
    };
    const keep = new Set(
      [...options]
        .sort((a, b) => rank(a) - rank(b))
        .slice(0, collapsedCount)
        .map((option) => option.value),
    );
    // The original order, so tiles never jump when one gets a key.
    return options.filter((option) => keep.has(option.value));
  }, [options, collapsible, expanded, kind, value, collapsedCount]);

  return (
    <div className="w-full">
      <LogoChoice
        label={label}
        value={value}
        onChange={onChange}
        disabled={disabled}
        minTile="9.5rem"
        readyLabel={t("assistantPage.cards.keySaved")}
        options={shown.map((option) => {
          const { name, detail } = splitLabel(option.label);
          return {
            value: option.value,
            label: name,
            hint: option.hint || detail,
            title: option.hint
              ? `${option.label} · ${option.hint}`
              : option.label,
            ready: option.ready,
            disabled: option.disabled,
            icon: <ProviderTile id={option.value} kind={kind} size="md" />,
          };
        })}
      />
      {collapsible && (
        <button
          type="button"
          onClick={() => setExpanded((open) => !open)}
          aria-expanded={expanded}
          className="mt-2 inline-flex cursor-pointer items-center gap-1 rounded-md px-1.5 py-1 text-[0.8125rem] font-medium text-muted transition-colors hover:bg-ink/[0.05] hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
        >
          <ChevronDown
            className={`h-3.5 w-3.5 transition-transform duration-200 ${expanded ? "rotate-180" : ""}`}
            aria-hidden="true"
          />
          {expanded
            ? t("providerGrid.showFewer")
            : t("providerGrid.showAll", { count: options.length })}
        </button>
      )}
    </div>
  );
};
