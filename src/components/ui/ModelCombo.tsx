import React from "react";
import { RefreshCw } from "lucide-react";
import { Input } from "./Input";

export type ModelComboOption = { value: string; label: string };

export interface ModelComboProps {
  value: string;
  options: ModelComboOption[];
  onCommit: (value: string) => void;
  onLoad: () => void;
  loading: boolean;
  error?: string | null;
  placeholder?: string;
  loadLabel: string;
  disabled?: boolean;
  /** Wrapper div classes (defaults to a right-aligned column). */
  className?: string;
  /** Classes for the text input itself. */
  inputClassName?: string;
  /** Unused; kept for call-site compatibility with the old creatable select. */
  formatCreateLabel?: (input: string) => string;
}

/** An editable text field with type-to-search suggestions (native `<datalist>`)
 *  paired with a "Load" (refresh) button and an inline error line. Shared by the
 *  assistant model/TTS pickers AND the dictation-cleanup model field so the two
 *  can never drift apart.
 *
 *  A plain editable input (not a react-select value chip) is deliberate: the
 *  value stays fully editable — cursor at the end, edit any character — which is
 *  what you want for tweaking a model or deployment name like `gpt-5.1-mini`,
 *  while the datalist still filters suggestions as you type. Module-scope so it
 *  isn't recreated on every parent render. */
export const ModelCombo: React.FC<ModelComboProps> = ({
  value,
  options,
  onCommit,
  onLoad,
  loading,
  error,
  placeholder,
  loadLabel,
  disabled,
  className = "flex flex-col items-end gap-1",
  inputClassName = "w-[292px]",
}) => {
  const listId = React.useId();
  const [local, setLocal] = React.useState(value);
  React.useEffect(() => setLocal(value), [value]);

  const commit = (next: string) => {
    const trimmed = next.trim();
    if (trimmed && trimmed !== value.trim()) onCommit(trimmed);
  };

  return (
    <div className={className}>
      <div className="flex items-center gap-2">
        <Input
          type="text"
          list={listId}
          value={local}
          onChange={(e) => {
            const next = e.target.value;
            setLocal(next);
            // Picking a suggestion from the datalist matches an option exactly —
            // commit immediately so a click doesn't require an extra blur.
            if (options.some((o) => o.value === next)) commit(next);
          }}
          onBlur={() => commit(local)}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit(local);
          }}
          placeholder={placeholder}
          disabled={disabled}
          className={inputClassName}
        />
        <datalist id={listId}>
          {options.map((o) => (
            <option
              key={o.value}
              value={o.value}
              label={o.label !== o.value ? o.label : undefined}
            />
          ))}
        </datalist>
        <button
          type="button"
          onClick={onLoad}
          disabled={loading || disabled}
          aria-label={loadLabel}
          title={loadLabel}
          className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-mid-gray/30 hover:bg-mid-gray/10 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <RefreshCw size={14} className={loading ? "animate-spin" : ""} />
        </button>
      </div>
      {error && (
        <span className="text-xs text-red-500 max-w-[360px] text-right break-words">
          {error}
        </span>
      )}
    </div>
  );
};

export default ModelCombo;
